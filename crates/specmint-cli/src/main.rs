use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

use specmint::generate::{ConfigSource, GenerateOptions, generate};

#[derive(Debug, Parser)]
#[command(
    name = "specmint",
    version,
    about = "OpenAPI → TypeScript generator (Kubb drop-in)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Load schema, build IR, run enabled plugins, write outputs.
    Generate {
        /// Path to `specmint.toml`
        #[arg(long, short = 'c', default_value = "specmint.toml")]
        config: std::path::PathBuf,

        /// Override generation mode (types-only skips plugins that write test data, such as factories and msw, even if enabled in config)
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,

        /// Comma-separated plugin names to run (overrides config `enabled` flags)
        #[arg(long, value_delimiter = ',')]
        plugins: Option<Vec<String>>,

        /// Force re-fetch / re-emit even when caches match
        #[arg(long)]
        force: bool,

        /// Print stage timings to stderr
        #[arg(long)]
        timings: bool,

        /// Also write the resolved schema bytes to this path (debug)
        #[arg(long)]
        write_schema: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Full,
    Types,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("specmint=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            print_error(err.as_ref());
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate {
            config,
            mode,
            plugins,
            force,
            timings,
            write_schema,
        } => {
            let started = Instant::now();
            let report = generate(GenerateOptions {
                config: ConfigSource::Path(config),
                mode: mode.map(|m| match m {
                    ModeArg::Full => specmint::config::GenerateMode::Full,
                    ModeArg::Types => specmint::config::GenerateMode::Types,
                }),
                plugins: specmint::plugins::builtin(),
                plugin_filter: plugins,
                force,
                write_schema,
            })?;

            if timings || report.skipped {
                let cache = if !report.source_is_url {
                    "file"
                } else if report.from_cache {
                    "hit"
                } else {
                    "miss"
                };
                eprintln!(
                    "specmint: fetch={:?} parse={:?} emit={:?} write={:?} total={:?} schema={} KiB cache={}{}",
                    report.timings.fetch,
                    report.timings.parse,
                    report.timings.emit,
                    report.timings.write,
                    started.elapsed(),
                    report.schema_bytes.div_ceil(1024),
                    cache,
                    if report.skipped {
                        " (unchanged, skipped emit)"
                    } else {
                        ""
                    }
                );
            }

            for path in &report.written {
                println!("{}", path.display());
            }
            Ok(())
        }
    }
}

fn print_error(err: &dyn std::error::Error) {
    eprintln!("error: {err}");
    let mut source = err.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}
