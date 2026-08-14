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
        let engine = Engine::new(&config).map_err(|e| AppError::Invoke(e.to_string()))?;
        Ok(Self {
            engine,
            cache: Arc::new(ModuleCache::new()),
        })
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

    fn invoke_sync(module: &Module, engine: &Engine, input: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, module, &[])
            .map_err(|e| AppError::Invoke(e.to_string()))?;

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
                (OFFSET as i32, i32::try_from(input.len()).unwrap_or(i32::MAX)),
            )
            .map_err(|e| AppError::Invoke(e.to_string()))?;

        if out_len < 0 {
            return Err(AppError::Invoke(format!("invoke returned negative len {out_len}")));
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

#[async_trait]
impl FunctionRunner for WasmtimeRunner {
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
        let output = tokio::task::spawn_blocking(move || Self::invoke_sync(&module, &engine, &input))
            .await
            .map_err(|e| AppError::Invoke(format!("join error: {e}")))??;

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
}
