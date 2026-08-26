use clap::Parser;
use cli::commands::deploy;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "nitrum-fn")]
#[command(
    about = "Deploy WASM functions to a nitrum-fn api",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Deploy a WASM function
    Deploy(deploy::DeployArgs),
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Deploy(args) => deploy::run(args).await,
    };

    if let Err(e) = result {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_deploy() {
        let cli = Cli::try_parse_from([
            "nitrum-fn",
            "deploy",
            "./echo.wasm",
            "--name",
            "echo",
            "--url",
            "http://127.0.0.1:8080",
        ])
        .expect("parse");
        match cli.command {
            Commands::Deploy(args) => {
                assert_eq!(args.wasm, PathBuf::from("./echo.wasm"));
                assert_eq!(args.name, "echo");
                assert_eq!(args.url, "http://127.0.0.1:8080");
                assert_eq!(args.timeout_secs, 180);
            }
        }
    }
}
