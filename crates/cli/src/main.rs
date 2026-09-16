use clap::Parser;
use cli::commands::{deploy, describe, invoke};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "nitrum-fn")]
#[command(
    about = "Deploy, invoke, and describe WASM functions on nitrum-fn",
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
    /// Print sha256 of a local `.wasm` (same as `x-nitrum-fn-hash` / `--fn-shasum`)
    Describe(describe::DescribeArgs),
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
        Commands::Describe(args) => describe::run(args).await,
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
    fn parses_describe() {
        let cli = Cli::try_parse_from(["nitrum-fn", "describe", "./oracle.wasm"]).expect("parse");
        match cli.command {
            Commands::Describe(args) => {
                assert_eq!(args.wasm, PathBuf::from("./oracle.wasm"));
            }
            _ => panic!("expected describe"),
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
            "--fn-shasum",
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
            "--pcr0",
            "abc",
            "--attestation-out",
            "./attestation.bin",
        ])
        .expect("parse");
        match cli.command {
            Commands::Invoke(args) => {
                assert_eq!(args.name, "oracle");
                assert_eq!(args.url, "https://invoke.example.com");
                assert!(args.insecure);
                assert_eq!(args.data, r#"{"ids":["eth"]}"#);
                assert_eq!(
                    args.fn_shasum.as_deref(),
                    Some("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899")
                );
                assert_eq!(args.pcr0.as_deref(), Some("abc"));
                assert_eq!(
                    args.attestation_out,
                    Some(PathBuf::from("./attestation.bin"))
                );
            }
            _ => panic!("expected invoke"),
        }
    }

    #[test]
    fn invoke_accepts_expect_hash_alias() {
        let cli = Cli::try_parse_from([
            "nitrum-fn",
            "invoke",
            "oracle",
            "--expect-hash",
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
        ])
        .expect("parse");
        match cli.command {
            Commands::Invoke(args) => {
                assert_eq!(
                    args.fn_shasum.as_deref(),
                    Some("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899")
                );
            }
            _ => panic!("expected invoke"),
        }
    }

    #[test]
    fn invoke_pcr0_requires_fn_shasum() {
        let err = match Cli::try_parse_from(["nitrum-fn", "invoke", "oracle", "--pcr0", "abc"]) {
            Ok(_) => panic!("pcr0 requires fn-shasum"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("fn-shasum") || msg.contains("--fn-shasum"),
            "{msg}"
        );
    }

    #[test]
    fn invoke_attestation_out_requires_pcr0() {
        let err = match Cli::try_parse_from([
            "nitrum-fn",
            "invoke",
            "oracle",
            "--attestation-out",
            "./attestation.bin",
        ]) {
            Ok(_) => panic!("attestation-out requires pcr0"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(msg.contains("pcr0") || msg.contains("--pcr0"), "{msg}");
    }
}
