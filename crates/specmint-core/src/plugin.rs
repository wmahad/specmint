//! The plugin interface and the registry that runs plugins.
//!
//! Core knows plugins only through [`Plugin`]: each plugin owns its name, reads
//! its own section of the config's `plugins` table, decides whether it is on and
//! reports the files it writes. The registry orders plugins by their
//! dependencies and filters them generically, so adding a plugin never needs a
//! change here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::adapter::ParsedSpec;
use crate::config::{Config, GenerateMode};
use crate::error::{Error, Result};
use crate::naming::Naming;

/// One file a plugin writes, relative to `output.path`.
pub struct OutputFile {
    pub relative_path: PathBuf,
    pub contents: String,
}

/// What a plugin reads while generating.
pub struct PluginContext<'a> {
    pub config: &'a Config,
    pub naming: &'a Naming,
    pub spec: &'a ParsedSpec,
}

/// A plugin dependency that applies with the current config: a plugin that
/// must be on for this plugin's output to work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveDependency {
    /// Name of the plugin depended on.
    pub plugin: &'static str,
    /// Another way to fix the config, besides switching the plugin on, written
    /// to finish the sentence "Switch on `x`, or …" (e.g. "set `parser = \"data\"`
    /// in [plugins.msw]").
    pub alternative: Option<String>,
}

pub trait Plugin: Send + Sync {
    /// The plugin's name: its key under `plugins` in the config and its id in
    /// `--plugins`.
    fn name(&self) -> &'static str;

    /// Every plugin this plugin can need. They run first.
    fn dependencies(&self) -> &'static [&'static str] {
        &[]
    }

    /// The dependencies that apply with this config, because this plugin's
    /// output imports theirs. Each must be one of [`Plugin::dependencies`].
    /// When one is off the run stops; specmint never switches a plugin on by
    /// itself. Defaults to every dependency.
    fn active_dependencies(&self, _config: &Config) -> Result<Vec<ActiveDependency>> {
        Ok(self
            .dependencies()
            .iter()
            .map(|plugin| ActiveDependency {
                plugin,
                alternative: None,
            })
            .collect())
    }

    /// Whether the config switches this plugin on.
    fn enabled(&self, config: &Config) -> Result<bool>;

    /// Whether the plugin writes test data (factories, mock handlers).
    /// `--mode types` skips these plugins.
    fn writes_test_data(&self) -> bool {
        false
    }

    /// The files [`Plugin::generate`] writes for this config.
    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>>;

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>>;
}

/// Every available plugin, in dependency order.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Fails on duplicate names, unknown dependencies and dependency cycles.
    pub fn new(plugins: Vec<Box<dyn Plugin>>) -> Result<Self> {
        Ok(Self {
            plugins: dependency_order(plugins)?,
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.plugins.iter().map(|p| p.name())
    }

    /// The plugins to run: those the config switches on, minus test-data plugins
    /// in `--mode types`. A `filter` (from `--plugins`) replaces the config's
    /// on/off switches. Config sections and filter entries must name a known
    /// plugin, and every chosen plugin's active dependencies must be chosen too.
    pub fn select(
        &self,
        config: &Config,
        mode: Option<GenerateMode>,
        filter: Option<&[String]>,
    ) -> Result<Selection<'_>> {
        for name in config.plugins.names() {
            self.check_known(name)?;
        }
        let filter: Option<HashSet<&str>> = match filter {
            Some(names) => {
                for name in names {
                    self.check_known(name)?;
                }
                Some(names.iter().map(String::as_str).collect())
            }
            None => None,
        };

        let mut selected = Vec::new();
        let mut off = HashMap::new();
        for plugin in &self.plugins {
            // Every plugin parses its options, so a typo in a plugin that ends
            // up filtered out still fails the run.
            let on_in_config = plugin.enabled(config)?;
            let reason = match &filter {
                Some(names) if !names.contains(plugin.name()) => Some(OffBecause::NotInFilter),
                Some(_) => None,
                None if !on_in_config => Some(OffBecause::Config),
                None if mode == Some(GenerateMode::Types) && plugin.writes_test_data() => {
                    Some(OffBecause::TypesMode)
                }
                None => None,
            };
            match reason {
                Some(reason) => {
                    off.insert(plugin.name(), reason);
                }
                None => selected.push(plugin.as_ref()),
            }
        }
        check_active_dependencies(&selected, &off, config)?;
        Ok(Selection { plugins: selected })
    }

    fn check_known(&self, name: &str) -> Result<()> {
        if self.plugins.iter().any(|p| p.name() == name) {
            return Ok(());
        }
        let known: Vec<_> = self.names().collect();
        Err(Error::config(format!(
            "unknown plugin `{name}` (known plugins: {})",
            known.join(", ")
        )))
    }
}

/// Why a plugin is not in a run, which decides how to fix a dependency on it.
#[derive(Debug, Clone, Copy)]
enum OffBecause {
    Config,
    NotInFilter,
    TypesMode,
}

impl OffBecause {
    /// `(why, fix)`, e.g. `("which is off", "Switch on `x`")`.
    fn explain(self, plugin: &str) -> (&'static str, String) {
        match self {
            Self::Config => ("which is off", format!("Switch on `{plugin}`")),
            Self::NotInFilter => (
                "which is not in --plugins",
                format!("Add `{plugin}` to --plugins"),
            ),
            Self::TypesMode => (
                "which --mode types skips",
                "Run without --mode types".to_string(),
            ),
        }
    }
}

fn check_active_dependencies(
    selected: &[&dyn Plugin],
    off: &HashMap<&'static str, OffBecause>,
    config: &Config,
) -> Result<()> {
    for plugin in selected {
        for dependency in plugin.active_dependencies(config)? {
            if !plugin.dependencies().contains(&dependency.plugin) {
                return Err(Error::plugin(
                    plugin.name(),
                    format!(
                        "depends on `{0}` with this config, but `{0}` is not in its dependencies()",
                        dependency.plugin
                    ),
                ));
            }
            let Some(reason) = off.get(dependency.plugin) else {
                continue;
            };
            let (why, fix) = reason.explain(dependency.plugin);
            let fix = match &dependency.alternative {
                Some(alternative) => format!("{fix}, or {alternative}."),
                None => format!("{fix}."),
            };
            return Err(Error::plugin(
                plugin.name(),
                format!("needs the `{}` plugin, {why}. {fix}", dependency.plugin),
            ));
        }
    }
    Ok(())
}

/// The plugins chosen for one run, in dependency order.
pub struct Selection<'a> {
    plugins: Vec<&'a dyn Plugin>,
}

impl Selection<'_> {
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.plugins.iter().map(|p| p.name())
    }

    pub fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for plugin in &self.plugins {
            files.extend(plugin.output_files(config)?);
        }
        Ok(files)
    }

    pub fn run(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        let mut files = Vec::new();
        for plugin in &self.plugins {
            tracing::info!(plugin = plugin.name(), "running plugin");
            files.extend(plugin.generate(ctx)?);
        }
        Ok(files)
    }
}

/// Dependency order that otherwise keeps registration order.
fn dependency_order(plugins: Vec<Box<dyn Plugin>>) -> Result<Vec<Box<dyn Plugin>>> {
    let mut index: HashMap<&'static str, usize> = HashMap::new();
    for (i, plugin) in plugins.iter().enumerate() {
        if index.insert(plugin.name(), i).is_some() {
            return Err(Error::plugin(
                plugin.name(),
                "plugin registered twice".to_string(),
            ));
        }
    }

    for plugin in &plugins {
        for dep in plugin.dependencies() {
            if !index.contains_key(dep) {
                return Err(Error::plugin(
                    plugin.name(),
                    format!("unknown dependency `{dep}`"),
                ));
            }
        }
    }

    // Repeatedly place the earliest-registered plugin whose dependencies are
    // all placed, so plugins only move later when a dependency forces it.
    let mut placed = vec![false; plugins.len()];
    let mut order = Vec::with_capacity(plugins.len());
    while order.len() < plugins.len() {
        let next = plugins.iter().enumerate().position(|(i, plugin)| {
            !placed[i] && plugin.dependencies().iter().all(|dep| placed[index[dep]])
        });
        let Some(i) = next else {
            // What is left waits on itself: plugins on (or behind) a cycle.
            let stuck: Vec<&'static str> = plugins
                .iter()
                .zip(&placed)
                .filter(|(_, placed)| !**placed)
                .map(|(p, _)| p.name())
                .collect();
            return Err(Error::plugin(
                stuck[0],
                format!("cyclic plugin dependencies: {}", stuck.join(", ")),
            ));
        };
        placed[i] = true;
        order.push(i);
    }

    let mut slots: Vec<Option<Box<dyn Plugin>>> = plugins.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|i| slots[i].take().expect("each index appears once"))
        .collect())
}
