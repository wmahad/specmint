//! napi-rs bindings — thin wrappers around [`specmint::generate::generate`] so the
//! npm package can call straight into the same pipeline the CLI uses.

use napi_derive::napi;

use specmint::generate::{ConfigSource, GenerateOptions, generate as run_generate};

#[napi(object)]
pub struct GenerateReportJs {
    pub written: Vec<String>,
    pub skipped: bool,
    pub spec_hash: String,
    pub from_cache: bool,
}

impl From<specmint::generate::GenerateReport> for GenerateReportJs {
    fn from(report: specmint::generate::GenerateReport) -> Self {
        Self {
            written: report
                .written
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
            skipped: report.skipped,
            spec_hash: report.spec_hash,
            from_cache: report.from_cache,
        }
    }
}

fn to_napi_err(err: specmint::Error) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

/// Runs generation from a config already resolved to JSON (the JS/TS config path —
/// the caller has already `import()`ed `specmint.config.ts`/`.js` and serialized its
/// default export).
#[napi]
pub fn generate(config_json: String) -> napi::Result<GenerateReportJs> {
    run_generate(GenerateOptions {
        config: ConfigSource::Json(config_json),
        mode: None,
        plugins: specmint::plugins::builtin(),
        plugin_filter: None,
        force: false,
        write_schema: None,
    })
    .map(Into::into)
    .map_err(to_napi_err)
}

/// Runs generation straight from a native `specmint.toml` path — mirrors the CLI's
/// default behavior with zero JSON/JS involvement.
#[napi]
pub fn generate_from_path(config_path: String) -> napi::Result<GenerateReportJs> {
    run_generate(GenerateOptions {
        config: ConfigSource::Path(config_path.into()),
        mode: None,
        plugins: specmint::plugins::builtin(),
        plugin_filter: None,
        force: false,
        write_schema: None,
    })
    .map(Into::into)
    .map_err(to_napi_err)
}
