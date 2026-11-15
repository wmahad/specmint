//! Core knows plugins only through the `Plugin` trait: these tests drive the
//! registry and `generate` with test-only plugins that core has never heard of.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tempfile::tempdir;

use specmint::Result;
use specmint::config::{Config, GenerateMode};
use specmint::generate::{ConfigSource, GenerateOptions, generate};
use specmint::plugin::{ActiveDependency, OutputFile, Plugin, PluginContext, PluginRegistry};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeOptions {
    #[serde(default)]
    enabled: Option<bool>,
    file: Option<String>,
    /// When true the plugin no longer needs its dependencies, like msw with
    /// `parser = "data"`.
    #[serde(default)]
    standalone: bool,
}

/// Records the order plugins ran in.
type RunLog = Arc<Mutex<Vec<&'static str>>>;

struct FakePlugin {
    name: &'static str,
    dependencies: &'static [&'static str],
    on_by_default: bool,
    test_data: bool,
    log: RunLog,
}

impl FakePlugin {
    fn new(name: &'static str, log: &RunLog) -> Self {
        Self {
            name,
            dependencies: &[],
            on_by_default: false,
            test_data: false,
            log: Arc::clone(log),
        }
    }

    fn options(&self, config: &Config) -> Result<Option<FakeOptions>> {
        config.plugins.section(self.name)
    }

    fn file(&self, config: &Config) -> Result<String> {
        Ok(self
            .options(config)?
            .and_then(|o| o.file)
            .unwrap_or_else(|| format!("{}.ts", self.name)))
    }
}

impl Plugin for FakePlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn dependencies(&self) -> &'static [&'static str] {
        self.dependencies
    }

    fn enabled(&self, config: &Config) -> Result<bool> {
        Ok(self
            .options(config)?
            .map(|o| o.enabled.unwrap_or(true))
            .unwrap_or(self.on_by_default))
    }

    fn writes_test_data(&self) -> bool {
        self.test_data
    }

    fn active_dependencies(&self, config: &Config) -> Result<Vec<ActiveDependency>> {
        if self.options(config)?.is_some_and(|o| o.standalone) {
            return Ok(Vec::new());
        }
        Ok(self
            .dependencies
            .iter()
            .map(|dep| ActiveDependency {
                plugin: dep,
                alternative: Some(format!(
                    "set `standalone = true` in [plugins.{}]",
                    self.name
                )),
            })
            .collect())
    }

    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(self.file(config)?)])
    }

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        self.log.lock().unwrap().push(self.name);
        Ok(vec![OutputFile {
            relative_path: PathBuf::from(self.file(ctx.config)?),
            contents: format!("// {} for {} schemas\n", self.name, ctx.spec.schemas.len()),
        }])
    }
}

fn config(plugins_json: &str) -> Config {
    Config::from_json_str(&format!(
        r#"{{ "source": {{ "input": "spec.yaml" }}, "output": {{ "path": "out" }}, "plugins": {plugins_json} }}"#
    ))
    .unwrap()
}

fn names(
    registry: &PluginRegistry,
    config: &Config,
    mode: Option<GenerateMode>,
    filter: Option<&[String]>,
) -> Vec<&'static str> {
    registry
        .select(config, mode, filter)
        .unwrap()
        .names()
        .collect()
}

#[test]
fn a_plugin_reads_its_own_config_section_to_decide_whether_it_is_on() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("alpha", &log)),
        Box::new(FakePlugin::new("beta", &log)),
        Box::new(FakePlugin {
            on_by_default: true,
            ..FakePlugin::new("gamma", &log)
        }),
    ])
    .unwrap();

    let cfg = config(r#"{ "alpha": {}, "beta": { "enabled": false } }"#);

    assert_eq!(names(&registry, &cfg, None, None), ["alpha", "gamma"]);
}

#[test]
fn a_selection_reports_the_output_files_of_its_plugins() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("alpha", &log)),
        Box::new(FakePlugin::new("beta", &log)),
    ])
    .unwrap();

    let cfg = config(r#"{ "alpha": { "file": "custom.ts" }, "beta": {} }"#);
    let selection = registry.select(&cfg, None, None).unwrap();

    assert_eq!(
        selection.output_files(&cfg).unwrap(),
        [PathBuf::from("custom.ts"), PathBuf::from("beta.ts")]
    );
}

#[test]
fn a_config_section_for_an_unknown_plugin_is_an_error() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![Box::new(FakePlugin::new("alpha", &log))]).unwrap();

    let cfg = config(r#"{ "alhpa": {} }"#);
    let err = registry.select(&cfg, None, None).err().unwrap().to_string();

    assert!(err.contains("unknown plugin `alhpa`"), "{err}");
    assert!(err.contains("alpha"), "{err}");
}

#[test]
fn a_plugin_that_cannot_parse_its_options_is_an_error_naming_the_plugin() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![Box::new(FakePlugin::new("alpha", &log))]).unwrap();

    let cfg = config(r#"{ "alpha": { "fiel": "x.ts" } }"#);
    let err = registry.select(&cfg, None, None).err().unwrap().to_string();

    assert!(err.contains("plugins.alpha"), "{err}");
    assert!(err.contains("fiel"), "{err}");
}

#[test]
fn the_plugin_filter_overrides_the_config_on_off_switches() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("alpha", &log)),
        Box::new(FakePlugin::new("beta", &log)),
    ])
    .unwrap();

    let cfg = config(r#"{ "alpha": {} }"#);
    let filter = vec!["beta".to_string()];

    assert_eq!(names(&registry, &cfg, None, Some(&filter)), ["beta"]);
}

#[test]
fn an_unknown_name_in_the_plugin_filter_is_an_error() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![Box::new(FakePlugin::new("alpha", &log))]).unwrap();

    let cfg = config("{}");
    let filter = vec!["omega".to_string()];
    let err = registry
        .select(&cfg, None, Some(&filter))
        .err()
        .unwrap()
        .to_string();

    assert!(err.contains("unknown plugin `omega`"), "{err}");
}

#[test]
fn types_mode_skips_plugins_that_write_test_data() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("alpha", &log)),
        Box::new(FakePlugin {
            test_data: true,
            ..FakePlugin::new("mocks", &log)
        }),
    ])
    .unwrap();

    let cfg = config(r#"{ "alpha": {}, "mocks": {} }"#);

    assert_eq!(
        names(&registry, &cfg, Some(GenerateMode::Types), None),
        ["alpha"]
    );
    assert_eq!(
        names(&registry, &cfg, Some(GenerateMode::Full), None),
        ["alpha", "mocks"]
    );
}

#[test]
fn plugins_run_after_the_plugins_they_depend_on() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin {
            dependencies: &["base"],
            ..FakePlugin::new("dependent", &log)
        }),
        Box::new(FakePlugin::new("base", &log)),
    ])
    .unwrap();

    let cfg = config(r#"{ "dependent": {}, "base": {} }"#);

    assert_eq!(names(&registry, &cfg, None, None), ["base", "dependent"]);
}

#[test]
fn a_registry_rejects_unknown_dependencies_cycles_and_duplicate_names() {
    let log = RunLog::default();

    let unknown = PluginRegistry::new(vec![Box::new(FakePlugin {
        dependencies: &["missing"],
        ..FakePlugin::new("alpha", &log)
    })]);
    let err = unknown.err().unwrap().to_string();
    assert!(err.contains("unknown dependency `missing`"), "{err}");

    let cycle = PluginRegistry::new(vec![
        Box::new(FakePlugin {
            dependencies: &["beta"],
            ..FakePlugin::new("alpha", &log)
        }),
        Box::new(FakePlugin {
            dependencies: &["alpha"],
            ..FakePlugin::new("beta", &log)
        }),
    ]);
    let err = cycle.err().unwrap().to_string();
    assert!(
        err.contains("cyclic plugin dependencies: alpha, beta"),
        "{err}"
    );
    assert!(!err.contains("registry"), "{err}");

    let duplicate = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("alpha", &log)),
        Box::new(FakePlugin::new("alpha", &log)),
    ]);
    let err = duplicate.err().unwrap().to_string();
    assert!(err.contains("registered twice"), "{err}");
}

#[test]
fn generate_runs_a_plugin_core_has_never_heard_of() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("generated");
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny.openapi.yaml");
    let config_json = serde_json::json!({
        "source": { "input": fixture, "cache": { "dir": dir.path().join("cache") } },
        "output": { "path": out },
        "plugins": { "base": {}, "dependent": { "file": "nested/dependent.ts" } },
    });

    let log = RunLog::default();
    let report = generate(GenerateOptions {
        config: ConfigSource::Json(config_json.to_string()),
        plugins: vec![
            Box::new(FakePlugin {
                dependencies: &["base"],
                ..FakePlugin::new("dependent", &log)
            }),
            Box::new(FakePlugin::new("base", &log)),
        ],
        mode: None,
        plugin_filter: None,
        force: true,
        write_schema: None,
    })
    .unwrap();

    assert_eq!(*log.lock().unwrap(), ["base", "dependent"]);
    let dependent = std::fs::read_to_string(out.join("nested/dependent.ts")).unwrap();
    assert!(
        dependent.ends_with("*/\n// dependent for 5 schemas\n"),
        "{dependent}"
    );
    assert_eq!(
        report.written,
        [out.join("base.ts"), out.join("nested/dependent.ts")]
    );
}

#[test]
fn generate_fails_when_no_plugin_is_on() {
    let dir = tempdir().unwrap();
    let config_json = serde_json::json!({
        "source": { "input": dir.path().join("missing.yaml") },
        "output": { "path": dir.path().join("out") },
    });

    let log = RunLog::default();
    let err = generate(GenerateOptions {
        config: ConfigSource::Json(config_json.to_string()),
        plugins: vec![Box::new(FakePlugin::new("alpha", &log))],
        mode: None,
        plugin_filter: None,
        force: true,
        write_schema: None,
    })
    .err()
    .unwrap()
    .to_string();

    assert!(err.contains("no plugins enabled"), "{err}");
}

#[test]
fn core_writes_the_banner_and_header_above_each_plugins_content() {
    let dir = tempdir().unwrap();
    let out = dir.path().join("generated");
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny.openapi.yaml");
    let config_json = serde_json::json!({
        "source": { "input": fixture, "cache": { "dir": dir.path().join("cache") } },
        "output": { "path": out, "banner": "// my banner\n// line two\n\n" },
        "plugins": { "alpha": {}, "beta": {} },
    });

    let log = RunLog::default();
    generate(GenerateOptions {
        config: ConfigSource::Json(config_json.to_string()),
        plugins: vec![
            Box::new(FakePlugin::new("alpha", &log)),
            Box::new(FakePlugin::new("beta", &log)),
        ],
        mode: None,
        plugin_filter: None,
        force: true,
        write_schema: None,
    })
    .unwrap();

    for name in ["alpha", "beta"] {
        assert_eq!(
            std::fs::read_to_string(out.join(format!("{name}.ts"))).unwrap(),
            format!(
                "// my banner\n// line two\n\
                 /**\n * Generated by specmint.\n * Do not edit manually.\n */\n\
                 // {name} for 5 schemas\n"
            )
        );
    }
}

fn dependent_and_base(log: &RunLog) -> PluginRegistry {
    PluginRegistry::new(vec![
        Box::new(FakePlugin::new("base", log)),
        Box::new(FakePlugin {
            dependencies: &["base"],
            ..FakePlugin::new("dependent", log)
        }),
    ])
    .unwrap()
}

#[test]
fn a_plugin_whose_dependency_is_off_stops_the_run_and_names_the_fix() {
    let log = RunLog::default();
    let registry = dependent_and_base(&log);

    let cfg = config(r#"{ "dependent": {} }"#);
    let err = registry.select(&cfg, None, None).err().unwrap().to_string();

    assert_eq!(
        err,
        "plugin[dependent]: needs the `base` plugin, which is off. \
         Switch on `base`, or set `standalone = true` in [plugins.dependent]."
    );
}

#[test]
fn a_dependency_without_an_alternative_only_suggests_switching_it_on() {
    struct Plain;
    impl Plugin for Plain {
        fn name(&self) -> &'static str {
            "plain"
        }
        fn dependencies(&self) -> &'static [&'static str] {
            &["base"]
        }
        fn enabled(&self, _: &Config) -> Result<bool> {
            Ok(true)
        }
        fn output_files(&self, _: &Config) -> Result<Vec<PathBuf>> {
            Ok(vec![])
        }
        fn generate(&self, _: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
            Ok(vec![])
        }
    }
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("base", &log)),
        Box::new(Plain),
    ])
    .unwrap();

    let err = registry
        .select(&config("{}"), None, None)
        .err()
        .unwrap()
        .to_string();

    assert_eq!(
        err,
        "plugin[plain]: needs the `base` plugin, which is off. Switch on `base`."
    );
}

#[test]
fn a_dependency_that_options_drop_lets_the_plugin_run_alone() {
    let log = RunLog::default();
    let registry = dependent_and_base(&log);

    let cfg = config(r#"{ "dependent": { "standalone": true } }"#);

    assert_eq!(names(&registry, &cfg, None, None), ["dependent"]);
}

#[test]
fn specmint_never_switches_on_a_dependency_itself() {
    let log = RunLog::default();
    let registry = dependent_and_base(&log);

    // Both on in config, but the filter leaves the dependency out.
    let cfg = config(r#"{ "base": {}, "dependent": {} }"#);
    let filter = vec!["dependent".to_string()];
    let err = registry
        .select(&cfg, None, Some(&filter))
        .err()
        .unwrap()
        .to_string();

    assert_eq!(
        err,
        "plugin[dependent]: needs the `base` plugin, which is not in --plugins. \
         Add `base` to --plugins, or set `standalone = true` in [plugins.dependent]."
    );
}

#[test]
fn types_mode_switching_off_a_dependency_stops_the_run() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin {
            test_data: true,
            ..FakePlugin::new("mocks", &log)
        }),
        Box::new(FakePlugin {
            dependencies: &["mocks"],
            ..FakePlugin::new("dependent", &log)
        }),
    ])
    .unwrap();

    let cfg = config(r#"{ "mocks": {}, "dependent": {} }"#);
    let err = registry
        .select(&cfg, Some(GenerateMode::Types), None)
        .err()
        .unwrap()
        .to_string();

    assert_eq!(
        err,
        "plugin[dependent]: needs the `mocks` plugin, which --mode types skips. \
         Run without --mode types, or set `standalone = true` in [plugins.dependent]."
    );
}

#[test]
fn an_active_dependency_must_be_a_declared_dependency() {
    struct Sneaky;
    impl Plugin for Sneaky {
        fn name(&self) -> &'static str {
            "sneaky"
        }
        fn enabled(&self, _: &Config) -> Result<bool> {
            Ok(true)
        }
        fn active_dependencies(&self, _: &Config) -> Result<Vec<ActiveDependency>> {
            Ok(vec![ActiveDependency {
                plugin: "base",
                alternative: None,
            }])
        }
        fn output_files(&self, _: &Config) -> Result<Vec<PathBuf>> {
            Ok(vec![])
        }
        fn generate(&self, _: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
            Ok(vec![])
        }
    }
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("base", &log)),
        Box::new(Sneaky),
    ])
    .unwrap();

    // An active dependency outside `dependencies()` would not be ordered before the
    // plugin, so it is a bug in the plugin, not a config problem.
    let err = registry
        .select(&config(r#"{ "base": {} }"#), None, None)
        .err()
        .unwrap()
        .to_string();
    assert!(
        err.contains("depends on `base` with this config, but `base` is not in its dependencies()"),
        "{err}"
    );
}

#[test]
fn plugins_keep_registration_order_unless_a_dependency_forces_a_move() {
    let log = RunLog::default();
    let registry = PluginRegistry::new(vec![
        Box::new(FakePlugin::new("a", &log)),
        Box::new(FakePlugin {
            dependencies: &["d"],
            ..FakePlugin::new("b", &log)
        }),
        Box::new(FakePlugin {
            dependencies: &["a"],
            ..FakePlugin::new("c", &log)
        }),
        Box::new(FakePlugin::new("d", &log)),
        Box::new(FakePlugin::new("e", &log)),
    ])
    .unwrap();

    assert_eq!(
        registry.names().collect::<Vec<_>>(),
        ["a", "c", "d", "b", "e"]
    );
}
