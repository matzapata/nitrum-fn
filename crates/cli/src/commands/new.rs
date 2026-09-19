use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use domain::FunctionId;

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Project directory name (also used as the suggested deploy `--name`)
    #[arg(value_name = "NAME", default_value = "hello-world")]
    pub name: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %args.name), err)]
pub async fn run(args: NewArgs) -> Result<()> {
    let directory = scaffold(
        &args.name,
        &env::current_dir().context("current directory")?,
    )?;
    let name = args.name.as_str();
    let rel = directory
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name);

    println!("Created ./{rel}");
    println!();
    println!("  rustup target add wasm32-unknown-unknown");
    println!(
        "  cargo build --manifest-path {rel}/Cargo.toml --target wasm32-unknown-unknown --release"
    );
    println!(
        "  nitrum-fn deploy {rel}/target/wasm32-unknown-unknown/release/hello_world.wasm --name {name}"
    );
    Ok(())
}

/// Write the bundled hello-world template into `{cwd}/{name}/`.
fn scaffold(name: &str, cwd: &Path) -> Result<PathBuf> {
    FunctionId::new(name).with_context(|| {
        format!("invalid name {name:?} (use 1–64 chars: ASCII alphanumeric, hyphen, or underscore)")
    })?;

    let directory = cwd.join(name);
    if directory.exists()
        && directory
            .read_dir()
            .with_context(|| format!("read {}", directory.display()))?
            .next()
            .is_some()
    {
        bail!("Directory is not empty");
    }

    fs::create_dir_all(directory.join("src"))
        .with_context(|| format!("create {}", directory.join("src").display()))?;

    let writes: Vec<(&str, &str)> = vec![
        ("src/lib.rs", template_lib_rs()),
        ("Cargo.toml", template_cargo_toml()),
        (".gitignore", "/target\n/Cargo.lock\n"),
    ];

    for (relative_path, contents) in writes {
        let dest = directory.join(relative_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&dest, contents).with_context(|| format!("write {}", dest.display()))?;
    }

    Ok(directory)
}

/// Files under `template/` — customer scaffold (not the monorepo `examples/hello-world`).
macro_rules! bundled_template {
    ($rel:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/template/", $rel))
    };
}

const fn template_lib_rs() -> &'static str {
    bundled_template!("src/lib.rs")
}

const fn template_cargo_toml() -> &'static str {
    bundled_template!("Cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nitrum-fn-new-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scaffolds_hello_world() {
        let cwd = temp_dir("ok");
        let dir = scaffold("hello-world", &cwd).expect("scaffold");
        assert_eq!(dir, cwd.join("hello-world"));

        let lib = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("#[runtime::main]"));
        assert!(lib.contains("Hello, world!"));

        let cargo = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("git = \"https://github.com/matzapata/nitrum-fn\""));
        assert!(cargo.contains("package = \"runtime\""));
        assert!(!cargo.contains("path = "));

        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "/target\n/Cargo.lock\n");

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn rejects_invalid_name() {
        let cwd = temp_dir("bad-name");
        let err = scaffold("Bad Name!", &cwd).unwrap_err();
        assert!(
            err.to_string().contains("invalid name") || err.to_string().contains("Bad Name"),
            "{err:#}"
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn rejects_non_empty_directory() {
        let cwd = temp_dir("nonempty");
        let dest = cwd.join("my-fn");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("existing.txt"), "nope").unwrap();

        let err = scaffold("my-fn", &cwd).unwrap_err();
        assert!(
            err.to_string().contains("Directory is not empty"),
            "{err:#}"
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn allows_empty_existing_directory() {
        let cwd = temp_dir("empty");
        let dest = cwd.join("my-fn");
        fs::create_dir_all(&dest).unwrap();

        let dir = scaffold("my-fn", &cwd).expect("scaffold into empty dir");
        assert!(dir.join("Cargo.toml").is_file());
        let _ = fs::remove_dir_all(&cwd);
    }
}
