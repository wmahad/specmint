//! End-to-end generate orchestration.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::info;

use crate::adapter::{ParseOptions, parse_openapi_with_options};
use crate::config::{Config, GenerateMode};
use crate::emit::{EmitCtx, EmitterRegistry};
use crate::error::{Error, Result};
use crate::naming::Naming;
use crate::source::{GenerateFingerprint, SourceLoader};
use crate::write::{outputs_exist, write_outputs};

/// Where to load the config from — a file path (native TOML) or config already
/// resolved to JSON (e.g. by the npm package's JS/TS config loader).
pub enum ConfigSource {
    Path(PathBuf),
    Json(String),
}

pub struct GenerateOptions {
    pub config: ConfigSource,
    pub mode: Option<GenerateMode>,
    pub emitter_filter: Option<Vec<String>>,
    pub force: bool,
    pub write_schema: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct StageTimings {
    pub fetch: Duration,
    pub parse: Duration,
    pub emit: Duration,
    pub write: Duration,
}

#[derive(Debug)]
pub struct GenerateReport {
    pub written: Vec<PathBuf>,
    pub skipped: bool,
    pub spec_hash: String,
    pub timings: StageTimings,
    /// True when schema bytes came from HTTP cache (304 / content-hash). Always
    /// false for file inputs.
    pub from_cache: bool,
    /// Whether `[source].input` was an `http(s)://` URL.
    pub source_is_url: bool,
    pub schema_bytes: usize,
}

pub fn generate(opts: GenerateOptions) -> Result<GenerateReport> {
    let config = match &opts.config {
        ConfigSource::Path(path) => Config::load(path)?,
        ConfigSource::Json(json) => Config::from_json_str(json)?,
    };
    let enabled = config.effective_emitters(opts.mode, opts.emitter_filter.as_deref());
    if !enabled.any_enabled() {
        return Err(Error::config(
            "no emitters enabled (check config / --mode / --emitters)",
        ));
    }

    let mut timings = StageTimings::default();

    let source_is_url = config.source.is_url();
    let t0 = Instant::now();
    let loader = SourceLoader::new(&config.source.cache)?;
    let schema = loader.load(&config.source, opts.force)?;
    timings.fetch = t0.elapsed();
    info!(
        from_cache = schema.from_cache,
        source_is_url,
        hash = %schema.content_hash,
        "schema loaded"
    );

    if let Some(path) = &opts.write_schema {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &schema.bytes)?;
    }

    let t1 = Instant::now();
    let spec = parse_openapi_with_options(
        &schema.bytes,
        ParseOptions {
            validate: config.adapter.validate,
        },
    )?;
    timings.parse = t1.elapsed();
    let spec_hash = spec.content_hash();
    info!(
        schemas = spec.schemas.len(),
        operations = spec.operations.len(),
        %spec_hash,
        "parsed spec built"
    );

    let fingerprint = GenerateFingerprint::in_cache_dir(&config.source.cache.dir);
    let expected_files = expected_output_files(&config, enabled);
    if !opts.force
        && fingerprint.read_spec_hash()?.as_deref() == Some(spec_hash.as_str())
        && outputs_exist(&config.output.path, &expected_files)
    {
        info!("parsed spec unchanged and outputs present; skipping emit");
        return Ok(GenerateReport {
            written: expected_files
                .into_iter()
                .map(|f| config.output.path.join(f))
                .collect(),
            skipped: true,
            spec_hash,
            timings,
            from_cache: schema.from_cache,
            source_is_url,
            schema_bytes: schema.bytes.len(),
        });
    }

    let naming = Naming::new(config.naming.clone())?;
    let registry = EmitterRegistry::builtin();
    let ctx = EmitCtx {
        config: &config,
        naming: &naming,
        spec: &spec,
        enabled,
    };

    let t2 = Instant::now();
    let files = registry.run(&ctx)?;
    timings.emit = t2.elapsed();

    let t3 = Instant::now();
    // Types-only mode should not wipe factories/handlers that Kubb still owns.
    let clean = config.output.clean && opts.mode != Some(GenerateMode::Types);
    let written = write_outputs(&config.output.path, clean, &files)?;
    timings.write = t3.elapsed();

    fingerprint.write_spec_hash(&spec_hash)?;

    Ok(GenerateReport {
        written,
        skipped: false,
        spec_hash,
        timings,
        from_cache: schema.from_cache,
        source_is_url,
        schema_bytes: schema.bytes.len(),
    })
}

fn expected_output_files(
    config: &Config,
    enabled: crate::config::EffectiveEmitters,
) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if enabled.types {
        files.push(PathBuf::from(&config.emitters.types.file));
    }
    if enabled.factories {
        files.push(PathBuf::from(&config.emitters.factories.file));
    }
    if enabled.msw {
        files.push(PathBuf::from(&config.emitters.msw.file));
    }
    if enabled.zod {
        files.push(PathBuf::from(&config.emitters.zod.file));
    }
    files
}
