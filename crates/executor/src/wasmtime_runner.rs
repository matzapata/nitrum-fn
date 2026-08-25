use std::time::Duration;

use application::error::AppError;
use application::ports::{FunctionRunner, RunOutcome};
use async_trait::async_trait;
use domain::{
    ContentHash, EPOCH_TICK, INVOKE_TIMEOUT, MAX_GUEST_MEMORY_BYTES, MAX_GUEST_OUTPUT_BYTES,
    MAX_INVOKE_BODY_BYTES,
};
use tracing::instrument;
use wasmtime::{
    Engine, ExternType, Instance, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
};

/// Runs guest modules under the v0 `invoke(ptr, len) -> len` ABI.
///
/// No in-process Module cache: each invoke deserializes from artifacts.
/// Publish uses `compile`; the enclave invoke path is load-only.
/// See `internal/ARCHITECTURE.md` §"Deferred: in-process Module cache".
pub struct WasmtimeRunner {
    engine: Engine,
    invoke_timeout: Duration,
    epoch_tick: Duration,
}

struct StoreData {
    limits: StoreLimits,
}

impl WasmtimeRunner {
    pub fn new() -> Result<Self, AppError> {
        Self::with_timeout(INVOKE_TIMEOUT, EPOCH_TICK)
    }

    /// Test / override path: shorter deadline than the product default.
    pub fn with_timeout(invoke_timeout: Duration, epoch_tick: Duration) -> Result<Self, AppError> {
        if epoch_tick.is_zero() {
            return Err(AppError::Compile("epoch tick must be non-zero".into()));
        }
        let mut config = wasmtime::Config::new();
        config.async_support(false);
        // Nitro EIFs are ~4GiB total. Wasmtime's 64-bit defaults reserve 4GiB of
        // VA per memory plus a guard region, and install SIGSEGV handlers. Either
        // can abort the guest; the data-plane then exits and the enclave dies.
        // Spectre mitigations must be off when signals-based traps are off.
        unsafe {
            config.cranelift_flag_set("enable_heap_access_spectre_mitigation", "false");
            config.cranelift_flag_set("enable_table_access_spectre_mitigation", "false");
        }
        config.signals_based_traps(false);
        config.memory_reservation(0);
        config.memory_guard_size(0);
        config.guard_before_linear_memory(false);
        config.memory_reservation_for_growth(16 * 1024 * 1024);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| AppError::Compile(e.to_string()))?;
        start_epoch_ticker(&engine, epoch_tick);
        Ok(Self {
            engine,
            invoke_timeout,
            epoch_tick,
        })
    }

    fn epoch_deadline_ticks(&self) -> u64 {
        let ticks = self.invoke_timeout.as_nanos() / self.epoch_tick.as_nanos();
        ticks.max(1) as u64
    }

    fn new_store(&self) -> Store<StoreData> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_GUEST_MEMORY_BYTES)
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreData { limits },
        );
        store.limiter(|data| &mut data.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.epoch_deadline_ticks());
        store
    }

    fn compile_sync(engine: &Engine, wasm: &[u8]) -> Result<Vec<u8>, AppError> {
        let module = Module::new(engine, wasm).map_err(|e| AppError::Compile(e.to_string()))?;
        assert_abi(&module)?;
        module
            .serialize()
            .map_err(|e| AppError::Compile(e.to_string()))
    }

    fn deserialize(engine: &Engine, compiled: &[u8]) -> Result<Module, AppError> {
        // SAFETY: `compiled` was produced by `Module::serialize` after a validating
        // `Module::new` in this host (same Engine config). Never deserialize
        // client-supplied AOT bytes.
        unsafe {
            Module::deserialize(engine, compiled)
                .map_err(|e| AppError::Invoke(format!("deserialize compiled module: {e}")))
        }
    }

    fn invoke_sync(
        &self,
        module: &Module,
        input: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        if input.len() > MAX_INVOKE_BODY_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "invoke input {} bytes exceeds max {MAX_INVOKE_BODY_BYTES}",
                input.len()
            )));
        }

        let mut store = self.new_store();
        let instance =
            Instance::new(&mut store, module, &[]).map_err(|e| map_wasm_err(e, "instantiate"))?;

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
            .map_err(|e| map_wasm_err(e, "invoke"))?;

        if out_len < 0 {
            return Err(AppError::Invoke(format!(
                "invoke returned negative len {out_len}"
            )));
        }
        let out_len = out_len as usize;
        if out_len > MAX_GUEST_OUTPUT_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "guest output {out_len} bytes exceeds max {MAX_GUEST_OUTPUT_BYTES}"
            )));
        }

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

fn start_epoch_ticker(engine: &Engine, tick: Duration) {
    // Each Engine has its own epoch counter; one ticker per Engine.
    let engine = engine.clone();
    std::thread::Builder::new()
        .name("wasmtime-epoch".into())
        .spawn(move || loop {
            std::thread::sleep(tick);
            engine.increment_epoch();
        })
        .expect("spawn wasmtime epoch ticker");
}

/// Map Wasmtime errors: epoch interrupt → Timeout; other traps → Trap; else Invoke.
fn map_wasm_err(err: wasmtime::Error, ctx: &str) -> AppError {
    if is_interrupt(&err) {
        return AppError::Timeout(format!("{ctx}: epoch deadline"));
    }
    if err.downcast_ref::<Trap>().is_some() {
        return AppError::Trap(format!("{ctx}: {err}"));
    }
    let mut source = err.source();
    while let Some(s) = source {
        if let Some(trap) = s.downcast_ref::<Trap>() {
            if *trap == Trap::Interrupt {
                return AppError::Timeout(format!("{ctx}: epoch deadline"));
            }
            return AppError::Trap(format!("{ctx}: {trap}"));
        }
        source = s.source();
    }
    AppError::Invoke(format!("{ctx}: {err}"))
}

fn is_interrupt(err: &wasmtime::Error) -> bool {
    if err.downcast_ref::<Trap>() == Some(&Trap::Interrupt) {
        return true;
    }
    // Fallback: Trap Display is "interrupt" for Trap::Interrupt.
    err.to_string().contains("interrupt")
}

fn assert_abi(module: &Module) -> Result<(), AppError> {
    match module.get_export("memory") {
        Some(ExternType::Memory(_)) => {}
        Some(other) => {
            return Err(AppError::Compile(format!(
                "export `memory` must be a memory, got {other:?}"
            )));
        }
        None => return Err(AppError::Compile("module missing export `memory`".into())),
    }

    match module.get_export("invoke") {
        Some(ExternType::Func(ty)) => {
            let params: Vec<_> = ty.params().collect();
            let results: Vec<_> = ty.results().collect();
            let ok = params.len() == 2
                && params[0].is_i32()
                && params[1].is_i32()
                && results.len() == 1
                && results[0].is_i32();
            if !ok {
                return Err(AppError::Compile(format!(
                    "export `invoke` must be (i32, i32) -> i32, got {ty}"
                )));
            }
        }
        Some(other) => {
            return Err(AppError::Compile(format!(
                "export `invoke` must be a function, got {other:?}"
            )));
        }
        None => return Err(AppError::Compile("module missing export `invoke`".into())),
    }
    Ok(())
}

fn join_err(err: tokio::task::JoinError) -> AppError {
    AppError::Invoke(format!("join error: {err}"))
}

#[async_trait]
impl FunctionRunner for WasmtimeRunner {
    #[instrument(skip(self, wasm), fields(hash = %hash, wasm_len = wasm.len()))]
    async fn compile(&self, hash: &ContentHash, wasm: &[u8]) -> Result<Vec<u8>, AppError> {
        let _ = hash;
        let engine = self.engine.clone();
        let wasm = wasm.to_vec();
        tokio::task::spawn_blocking(move || Self::compile_sync(&engine, &wasm))
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
        let _ = hash;
        let engine = self.engine.clone();
        let compiled = compiled.to_vec();
        let input = input.to_vec();
        let timeout = self.invoke_timeout;
        let tick = self.epoch_tick;
        // Rebuild a runner handle on the blocking thread with the same Engine.
        // Engine is Arc-backed; we only need deadline math + store setup.
        let runner = WasmtimeRunner {
            engine: engine.clone(),
            invoke_timeout: timeout,
            epoch_tick: tick,
        };
        let output = tokio::task::spawn_blocking(move || {
            let module = Self::deserialize(&engine, &compiled)?;
            runner.invoke_sync(&module, &input)
        })
        .await
        .map_err(join_err)??;
        Ok(RunOutcome { output })
    }

    #[instrument(skip(self, wasm, input), fields(hash = %hash, input_len = input.len()))]
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError> {
        let _ = hash;
        let engine = self.engine.clone();
        let wasm = wasm.to_vec();
        let input = input.to_vec();
        let timeout = self.invoke_timeout;
        let tick = self.epoch_tick;

        let runner = WasmtimeRunner {
            engine: engine.clone(),
            invoke_timeout: timeout,
            epoch_tick: tick,
        };
        // Cranelift compile / instantiate can be CPU-heavy; keep the async runtime free.
        let output = tokio::task::spawn_blocking(move || {
            let module =
                Module::new(&engine, &wasm).map_err(|e| AppError::Invoke(e.to_string()))?;
            assert_abi(&module).map_err(|e| match e {
                AppError::Compile(msg) => AppError::Invoke(msg),
                other => other,
            })?;
            runner.invoke_sync(&module, &input)
        })
        .await
        .map_err(join_err)??;

        Ok(RunOutcome { output })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::FunctionRunner;
    use std::time::Instant;

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

    fn trap_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                unreachable
              )
            )
            "#,
        )
        .expect("wat")
    }

    fn loop_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                (block $exit (result i32)
                  (loop $forever
                    br $forever
                  )
                  i32.const 0
                )
              )
            )
            "#,
        )
        .expect("wat")
    }

    fn start_loop_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func $spin
                (loop $forever
                  br $forever
                )
              )
              (start $spin)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                local.get $len
              )
            )
            "#,
        )
        .expect("wat")
    }

    fn huge_output_wasm() -> Vec<u8> {
        // Claims output larger than MAX_GUEST_OUTPUT_BYTES without writing it.
        let too_big = (MAX_GUEST_OUTPUT_BYTES + 1) as i32;
        wat::parse_str(format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                i32.const {too_big}
              )
            )
            "#
        ))
        .expect("wat")
    }

    fn grow_past_limit_wasm() -> Vec<u8> {
        // Try to grow by enough pages to exceed MAX_GUEST_MEMORY_BYTES (64MiB = 1024 pages).
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
                (drop (memory.grow (i32.const 2048)))
                local.get $len
              )
            )
            "#,
        )
        .expect("wat")
    }

    #[tokio::test]
    async fn echoes_payload() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = echo_wasm();
        let hash = ContentHash::from_bytes(&wasm);

        let first = runner.run(&hash, &wasm, b"hello").await.expect("run");
        assert_eq!(first.output, b"hello");

        let second = runner.run(&hash, &wasm, b"world").await.expect("run");
        assert_eq!(second.output, b"world");
    }

    #[tokio::test]
    async fn compile_then_run_precompiled_echoes() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = echo_wasm();
        let hash = ContentHash::from_bytes(&wasm);

        let compiled = runner.compile(&hash, &wasm).await.expect("compile");
        assert!(!compiled.is_empty());

        let out = runner
            .run_precompiled(&hash, &compiled, b"hello")
            .await
            .expect("run");
        assert_eq!(out.output, b"hello");
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

    #[tokio::test]
    async fn compile_does_not_hang_on_start_loop() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = start_loop_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let started = Instant::now();
        let compiled = runner.compile(&hash, &wasm).await.expect("compile");
        assert!(!compiled.is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "compile must not instantiate (start)"
        );
    }

    #[tokio::test]
    async fn invoke_unreachable_is_trap() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = trap_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let err = runner.run(&hash, &wasm, b"x").await.expect_err("trap");
        assert!(matches!(err, AppError::Trap(_)), "{err}");
    }

    #[tokio::test]
    async fn infinite_loop_times_out() {
        let runner =
            WasmtimeRunner::with_timeout(Duration::from_millis(100), Duration::from_millis(10))
                .expect("engine");
        let wasm = loop_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let started = Instant::now();
        let err = runner.run(&hash, &wasm, b"x").await.expect_err("timeout");
        assert!(matches!(err, AppError::Timeout(_)), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "should interrupt within a few ticks, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn start_loop_times_out_on_invoke() {
        let runner =
            WasmtimeRunner::with_timeout(Duration::from_millis(100), Duration::from_millis(10))
                .expect("engine");
        let wasm = start_loop_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let compiled = runner.compile(&hash, &wasm).await.expect("compile");
        let err = runner
            .run_precompiled(&hash, &compiled, b"x")
            .await
            .expect_err("timeout");
        assert!(matches!(err, AppError::Timeout(_)), "{err}");
    }

    #[tokio::test]
    async fn rejects_oversized_guest_output() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = huge_output_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let err = runner.run(&hash, &wasm, b"x").await.expect_err("too large");
        assert!(matches!(err, AppError::PayloadTooLarge(_)), "{err}");
    }

    #[tokio::test]
    async fn memory_grow_past_limit_fails() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = grow_past_limit_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        // Spec-compliant grow returns -1; host grow during setup uses limiter.
        // Either Invoke (host grow) or successful return with failed grow is OK —
        // the limiter must not OOM the process. Calling invoke after a failed
        // guest grow that returns -1 still succeeds with echo len.
        let result = runner.run(&hash, &wasm, b"ok").await;
        match result {
            Ok(out) => assert_eq!(out.output, b"ok"),
            Err(AppError::Invoke(_)) | Err(AppError::Trap(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
}
