use std::sync::Arc;

use application::error::AppError;
use application::ports::{FunctionRunner, RunOutcome};
use async_trait::async_trait;
use domain::ContentHash;
use tracing::instrument;
use wasmtime::{Engine, Instance, Module, Store};

use crate::module_cache::ModuleCache;

/// Runs guest modules under the v0 `invoke(ptr, len) -> len` ABI.
pub struct WasmtimeRunner {
    engine: Engine,
    cache: Arc<ModuleCache>,
}

impl WasmtimeRunner {
    pub fn new() -> Result<Self, AppError> {
        let mut config = wasmtime::Config::new();
        config.async_support(false);
        let engine = Engine::new(&config).map_err(|e| AppError::Compile(e.to_string()))?;
        Ok(Self {
            engine,
            cache: Arc::new(ModuleCache::new()),
        })
    }

    /// Empty the in-process Module cache (cold worker / post-restart before preload).
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    fn compile_or_get(&self, hash: &ContentHash, wasm: &[u8]) -> Result<(Module, bool), AppError> {
        if let Some(module) = self.cache.get(hash) {
            return Ok((module, true));
        }
        let module =
            Module::new(&self.engine, wasm).map_err(|e| AppError::Invoke(e.to_string()))?;
        self.cache.insert(hash.clone(), module.clone());
        Ok((module, false))
    }

    fn compile_sync(
        engine: &Engine,
        cache: &ModuleCache,
        hash: &ContentHash,
        wasm: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        let module = if let Some(module) = cache.get(hash) {
            module
        } else {
            Module::new(engine, wasm).map_err(|e| AppError::Compile(e.to_string()))?
        };
        assert_abi(engine, &module)?;
        cache.insert(hash.clone(), module.clone());
        module
            .serialize()
            .map_err(|e| AppError::Compile(e.to_string()))
    }

    fn deserialize_or_get(
        engine: &Engine,
        cache: &ModuleCache,
        hash: &ContentHash,
        compiled: &[u8],
    ) -> Result<(Module, bool), AppError> {
        if let Some(module) = cache.get(hash) {
            return Ok((module, true));
        }
        // SAFETY: `compiled` was produced by `Module::serialize` after a validating
        // `Module::new` in this host (same Engine config). Never deserialize
        // client-supplied AOT bytes.
        let module = unsafe {
            Module::deserialize(engine, compiled)
                .map_err(|e| AppError::Invoke(format!("deserialize compiled module: {e}")))?
        };
        cache.insert(hash.clone(), module.clone());
        Ok((module, false))
    }

    fn invoke_sync(module: &Module, engine: &Engine, input: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut store = Store::new(engine, ());
        let instance =
            Instance::new(&mut store, module, &[]).map_err(|e| AppError::Invoke(e.to_string()))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| AppError::Invoke("module missing export `memory`".into()))?;

        let invoke = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "invoke")
            .map_err(|e| AppError::Invoke(format!("module missing `invoke`: {e}")))?;

        // Reserve room at offset 64 so a tiny prologue stays clear if guests grow later.
        const OFFSET: usize = 64;
        let needed = OFFSET + input.len();
        let pages_needed = needed.div_ceil(65536) as u64;
        let current = memory.size(&store);
        if pages_needed > current {
            memory
                .grow(&mut store, pages_needed - current)
                .map_err(|e| AppError::Invoke(e.to_string()))?;
        }

        memory
            .write(&mut store, OFFSET, input)
            .map_err(|e| AppError::Invoke(e.to_string()))?;

        let out_len = invoke
            .call(
                &mut store,
                (
                    OFFSET as i32,
                    i32::try_from(input.len()).unwrap_or(i32::MAX),
                ),
            )
            .map_err(|e| AppError::Invoke(e.to_string()))?;

        if out_len < 0 {
            return Err(AppError::Invoke(format!(
                "invoke returned negative len {out_len}"
            )));
        }
        let out_len = out_len as usize;

        // Allow guests to write past the original input length; grow if needed.
        let end = OFFSET
            .checked_add(out_len)
            .ok_or_else(|| AppError::Invoke("output length overflow".into()))?;
        let pages_needed = end.div_ceil(65536) as u64;
        let current = memory.size(&store);
        if pages_needed > current {
            memory
                .grow(&mut store, pages_needed - current)
                .map_err(|e| AppError::Invoke(e.to_string()))?;
        }

        let mut output = vec![0u8; out_len];
        memory
            .read(&store, OFFSET, &mut output)
            .map_err(|e| AppError::Invoke(e.to_string()))?;
        Ok(output)
    }
}

fn assert_abi(engine: &Engine, module: &Module) -> Result<(), AppError> {
    let mut store = Store::new(engine, ());
    let instance =
        Instance::new(&mut store, module, &[]).map_err(|e| AppError::Compile(e.to_string()))?;
    instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| AppError::Compile("module missing export `memory`".into()))?;
    instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "invoke")
        .map_err(|e| AppError::Compile(format!("module missing `invoke`: {e}")))?;
    Ok(())
}

fn join_err(err: tokio::task::JoinError) -> AppError {
    AppError::Invoke(format!("join error: {err}"))
}

#[async_trait]
impl FunctionRunner for WasmtimeRunner {
    #[instrument(skip(self, wasm), fields(hash = %hash, wasm_len = wasm.len()))]
    async fn compile(&self, hash: &ContentHash, wasm: &[u8]) -> Result<Vec<u8>, AppError> {
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let hash = hash.clone();
        let wasm = wasm.to_vec();
        tokio::task::spawn_blocking(move || Self::compile_sync(&engine, &cache, &hash, &wasm))
            .await
            .map_err(join_err)?
    }

    #[instrument(skip(self, compiled), fields(hash = %hash, compiled_len = compiled.len()))]
    async fn load_precompiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError> {
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let hash = hash.clone();
        let compiled = compiled.to_vec();
        tokio::task::spawn_blocking(move || {
            Self::deserialize_or_get(&engine, &cache, &hash, &compiled).map(|_| ())
        })
        .await
        .map_err(join_err)?
    }

    #[instrument(skip(self, compiled, input), fields(hash = %hash, input_len = input.len()))]
    async fn run_precompiled(
        &self,
        hash: &ContentHash,
        compiled: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError> {
        let (module, warm_module) =
            Self::deserialize_or_get(&self.engine, &self.cache, hash, compiled)?;
        let engine = self.engine.clone();
        let input = input.to_vec();
        let output =
            tokio::task::spawn_blocking(move || Self::invoke_sync(&module, &engine, &input))
                .await
                .map_err(join_err)??;
        Ok(RunOutcome {
            output,
            warm_module,
        })
    }

    #[instrument(skip(self, wasm, input), fields(hash = %hash, input_len = input.len()))]
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError> {
        let (module, warm_module) = self.compile_or_get(hash, wasm)?;
        let engine = self.engine.clone();
        let input = input.to_vec();

        // Cranelift compile / instantiate can be CPU-heavy; keep the async runtime free.
        let output =
            tokio::task::spawn_blocking(move || Self::invoke_sync(&module, &engine, &input))
                .await
                .map_err(join_err)??;

        Ok(RunOutcome {
            output,
            warm_module,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::FunctionRunner;

    fn echo_wasm() -> Vec<u8> {
        // Echo: return the same len; bytes already at ptr.
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                local.get $len
              )
            )
            "#,
        )
        .expect("wat")
    }

    fn missing_invoke_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
            )
            "#,
        )
        .expect("wat")
    }

    #[tokio::test]
    async fn echoes_payload_and_warms_cache() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = echo_wasm();
        let hash = ContentHash::from_bytes(&wasm);

        let first = runner.run(&hash, &wasm, b"hello").await.expect("run");
        assert_eq!(first.output, b"hello");
        assert!(!first.warm_module);

        let second = runner.run(&hash, &wasm, b"world").await.expect("run");
        assert_eq!(second.output, b"world");
        assert!(second.warm_module);
    }

    #[tokio::test]
    async fn compile_then_run_precompiled_echoes() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = echo_wasm();
        let hash = ContentHash::from_bytes(&wasm);

        let compiled = runner.compile(&hash, &wasm).await.expect("compile");
        assert!(!compiled.is_empty());

        // Same runner: compile already filled the cache.
        let warm = runner
            .run_precompiled(&hash, &compiled, b"hello")
            .await
            .expect("run");
        assert_eq!(warm.output, b"hello");
        assert!(warm.warm_module);

        // Fresh runner: deserialize path, then cache hit.
        let other = WasmtimeRunner::new().expect("engine");
        let cold = other
            .run_precompiled(&hash, &compiled, b"hello")
            .await
            .expect("deserialize run");
        assert_eq!(cold.output, b"hello");
        assert!(!cold.warm_module);

        let hot = other
            .run_precompiled(&hash, &compiled, b"world")
            .await
            .expect("cached run");
        assert_eq!(hot.output, b"world");
        assert!(hot.warm_module);
    }

    #[tokio::test]
    async fn compile_rejects_module_missing_invoke() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = missing_invoke_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let err = runner.compile(&hash, &wasm).await.expect_err("abi");
        match err {
            AppError::Compile(msg) => assert!(msg.contains("invoke"), "{msg}"),
            other => panic!("expected Compile, got {other}"),
        }
    }
}
