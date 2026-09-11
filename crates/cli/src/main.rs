use clap::Parser;
use cli::commands::{deploy, invoke};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "nitrum-fn")]
#[command(
    about = "Deploy and invoke WASM functions on nitrum-fn",
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
    /// Invoke a deployed function
    Invoke(invoke::InvokeArgs),
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Deploy(args) => deploy::run(args).await,
        Commands::Invoke(args) => invoke::run(args).await,
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
            "--allow-url",
            "https://api.example.com",
        ])
        .expect("parse");
        match cli.command {
            Commands::Deploy(args) => {
                assert_eq!(args.wasm, PathBuf::from("./echo.wasm"));
                assert_eq!(args.name, "echo");
                assert_eq!(args.url, "http://127.0.0.1:8080");
                assert_eq!(args.timeout_secs, 180);
                assert_eq!(args.allow_urls, vec!["https://api.example.com"]);
            }
            _ => panic!("expected deploy"),
        }
    }

    #[test]
    fn parses_invoke() {
        let cli = Cli::try_parse_from([
            "nitrum-fn",
            "invoke",
            "oracle",
            "--url",
            "https://invoke.example.com",
            "--insecure",
            "-d",
            r#"{"ids":["eth"]}"#,
        ])
        .expect("parse");
        match cli.command {
            Commands::Invoke(args) => {
                assert_eq!(args.name, "oracle");
                assert_eq!(args.url, "https://invoke.example.com");
                assert!(args.insecure);
                assert_eq!(args.data, r#"{"ids":["eth"]}"#);
            }
            _ => panic!("expected invoke"),
        }
    }
}
