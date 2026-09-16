use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use domain::ContentHash;

#[derive(Debug, Args)]
pub struct DescribeArgs {
    /// Path to a compiled `.wasm` module
    pub wasm: PathBuf,
}

/// Print the content hash of a local `.wasm` (sha256, same as `x-nitrum-fn-hash` / `--fn-shasum`).
///
/// stdout:
/// ```text
/// hash=<64-char hex>
/// wasm_bytes=<n>
/// ```
#[tracing::instrument(level = "debug", skip_all, fields(wasm = %args.wasm.display()), err)]
pub async fn run(args: DescribeArgs) -> Result<()> {
    let (hash, bytes) = hash_wasm(&args.wasm)?;
    println!("hash={}", hash.to_hex());
    println!("wasm_bytes={bytes}");
    Ok(())
}

fn hash_wasm(path: &Path) -> Result<(ContentHash, usize)> {
    let wasm = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if wasm.is_empty() {
        bail!("empty wasm");
    }
    Ok((ContentHash::from_bytes(&wasm), wasm.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_file_bytes() {
        let dir = std::env::temp_dir().join(format!("nitrum-fn-describe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.wasm");
        let wasm = b"\0asm describe";
        std::fs::write(&path, wasm).unwrap();

        let (hash, bytes) = hash_wasm(&path).unwrap();
        assert_eq!(hash, ContentHash::from_bytes(wasm));
        assert_eq!(bytes, wasm.len());
    }

    #[test]
    fn rejects_empty_file() {
        let dir =
            std::env::temp_dir().join(format!("nitrum-fn-describe-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.wasm");
        std::fs::write(&path, b"").unwrap();

        let err = hash_wasm(&path).unwrap_err();
        assert!(err.to_string().contains("empty wasm"));
    }
}
