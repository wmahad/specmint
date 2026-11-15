//! The plugin interface and the registry that runs plugins.
//!
//! Core knows plugins only through [`Plugin`]: each plugin owns its name, reads
//! its own section of the config's `plugins` table, decides whether it is on and
//! reports the files it writes. The registry orders plugins by their
//! dependencies and filters them generically, so adding a plugin never needs a
//! change here.

use std::collections::{HashMap, HashSet, VecDeque};
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

pub trait Plugin: Send + Sync {
    /// The plugin's name: its key under `plugins` in the config and its id in
    /// `--plugins`.
    fn name(&self) -> &'static str;

    /// Names of the plugins whose output this plugin's output imports. They run
    /// first.
    fn dependencies(&self) -> &'static [&'static str] {
        &[]
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
    /// plugin.
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
        for plugin in &self.plugins {
            // Every plugin parses its options, so a typo in a plugin that ends
            // up filtered out still fails the run.
            let on_in_config = plugin.enabled(config)?;
            let on = match &filter {
                Some(names) => names.contains(plugin.name()),
                None => {
                    on_in_config
                        && !(mode == Some(GenerateMode::Types) && plugin.writes_test_data())
                }
            };
            if on {
                selected.push(plugin.as_ref());
            }
        }
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

/// Topological sort that keeps registration order among independent plugins.
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

    let mut indegree = vec![0usize; plugins.len()];
    let mut dependents: Vec<Vec<usize>> = vec![vec![]; plugins.len()];
    for (i, plugin) in plugins.iter().enumerate() {
        for dep in plugin.dependencies() {
            let Some(&dep_idx) = index.get(dep) else {
                return Err(Error::plugin(
                    plugin.name(),
                    format!("unknown dependency `{dep}`"),
                ));
            };
            dependents[dep_idx].push(i);
            indegree[i] += 1;
        }
    }

    let mut queue: VecDeque<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, d)| (*d == 0).then_some(i))
        .collect();
    let mut order = Vec::with_capacity(plugins.len());
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &next in &dependents[i] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    if order.len() != plugins.len() {
        return Err(Error::plugin(
            "registry",
            "cyclic plugin dependencies".to_string(),
        ));
    }

    let mut slots: Vec<Option<Box<dyn Plugin>>> = plugins.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|i| slots[i].take().expect("each index appears once"))
        .collect())
}
