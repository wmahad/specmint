//! Core knows plugins only through the `Plugin` trait: these tests drive the
//! registry and `generate` with test-only plugins that core has never heard of.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tempfile::tempdir;

use specmint::Result;
use specmint::config::{Config, GenerateMode};
use specmint::generate::{ConfigSource, GenerateOptions, generate};
use specmint::plugin::{OutputFile, Plugin, PluginContext, PluginRegistry};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeOptions {
    #[serde(default)]
    enabled: Option<bool>,
    file: Option<String>,
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
    assert!(err.contains("cyclic plugin dependencies"), "{err}");

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
    assert_eq!(
        std::fs::read_to_string(out.join("nested/dependent.ts")).unwrap(),
        "// dependent for 5 schemas\n"
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
