use std::time::Duration;

use application::error::AppError;
use application::ports::{FunctionRunner, RunOutcome};
use async_trait::async_trait;
use domain::{
    ContentHash, EgressOrigin, EPOCH_TICK, INVOKE_TIMEOUT, MAX_GUEST_MEMORY_BYTES,
    MAX_GUEST_OUTPUT_BYTES, MAX_HTTP_URL_BYTES, MAX_INVOKE_BODY_BYTES,
};
use tracing::instrument;
use wasmtime::{
    Caller, Engine, ExternType, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
    Trap,
};

use crate::http_host::{default_http_client, fetch_url, SharedHttpClient, ERR_BAD_ARGS, ERR_TOO_LARGE};

/// Runs guest modules under the v0 `invoke(ptr, len) -> len` ABI.
///
/// Each invoke deserializes from artifacts. Publish uses `compile`; the enclave
/// invoke path is load-only.
pub struct WasmtimeRunner {
    engine: Engine,
    invoke_timeout: Duration,
    epoch_tick: Duration,
    http_client: SharedHttpClient,
}

struct StoreData {
    limits: StoreLimits,
    egress_allow: Vec<EgressOrigin>,
    http_client: SharedHttpClient,
}

impl WasmtimeRunner {
    pub fn new() -> Result<Self, AppError> {
        Self::with_timeout(INVOKE_TIMEOUT, EPOCH_TICK)
    }

    /// Test / override path: shorter deadline than the product default.
    pub fn with_timeout(invoke_timeout: Duration, epoch_tick: Duration) -> Result<Self, AppError> {
        Self::with_http_client(invoke_timeout, epoch_tick, default_http_client())
    }

    pub fn with_http_client(
        invoke_timeout: Duration,
        epoch_tick: Duration,
        http_client: SharedHttpClient,
    ) -> Result<Self, AppError> {
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
            http_client,
        })
    }

    fn epoch_deadline_ticks(&self) -> u64 {
        let ticks = self.invoke_timeout.as_nanos() / self.epoch_tick.as_nanos();
        ticks.max(1) as u64
    }

    fn new_store(&self, egress_allow: &[EgressOrigin]) -> Store<StoreData> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_GUEST_MEMORY_BYTES)
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreData {
                limits,
                egress_allow: egress_allow.to_vec(),
                http_client: self.http_client.clone(),
            },
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
        // `Module::new` in this host (same Engine config).
        unsafe {
            Module::deserialize(engine, compiled)
                .map_err(|e| AppError::Invoke(format!("deserialize compiled module: {e}")))
        }
    }

    fn instantiate(
        engine: &Engine,
        store: &mut Store<StoreData>,
        module: &Module,
    ) -> Result<wasmtime::Instance, AppError> {
        let mut linker = Linker::new(engine);
        linker
            .func_wrap(
                "nitrum",
                "http_get",
                |mut caller: Caller<'_, StoreData>,
                 url_ptr: i32,
                 url_len: i32,
                 out_ptr: i32,
                 out_cap: i32|
                 -> i32 {
                    http_get_host(&mut caller, url_ptr, url_len, out_ptr, out_cap)
                },
            )
            .map_err(|e| AppError::Invoke(format!("link http_get: {e}")))?;
        linker
            .instantiate(store, module)
            .map_err(|e| map_wasm_err(e, "instantiate"))
    }

    fn invoke_sync(
        &self,
        module: &Module,
        input: &[u8],
        egress_allow: &[EgressOrigin],
    ) -> Result<Vec<u8>, AppError> {
        if input.len() > MAX_INVOKE_BODY_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "invoke input {} bytes exceeds max {MAX_INVOKE_BODY_BYTES}",
                input.len()
            )));
        }

        let mut store = self.new_store(egress_allow);
        let instance = Self::instantiate(&self.engine, &mut store, module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| AppError::Invoke("module missing export `memory`".into()))?;

        let invoke = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "invoke")
            .map_err(|e| AppError::Invoke(format!("module missing `invoke`: {e}")))?;

        // Input starts at offset 64 so a small prologue stays clear of the buffer.
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

fn http_get_host(
    caller: &mut Caller<'_, StoreData>,
    url_ptr: i32,
    url_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    if url_ptr < 0 || url_len < 0 || out_ptr < 0 || out_cap < 0 {
        return ERR_BAD_ARGS;
    }
    let url_len = url_len as usize;
    let out_cap = out_cap as usize;
    if url_len > MAX_HTTP_URL_BYTES || out_cap == 0 {
        return ERR_BAD_ARGS;
    }

    let memory = match guest_memory(caller) {
        Some(m) => m,
        None => return ERR_BAD_ARGS,
    };

    let mut url_buf = vec![0u8; url_len];
    if memory
        .read(&mut *caller, url_ptr as usize, &mut url_buf)
        .is_err()
    {
        return ERR_BAD_ARGS;
    }
    let url = match std::str::from_utf8(&url_buf) {
        Ok(s) => s,
        Err(_) => return ERR_BAD_ARGS,
    };

    let (egress_allow, http_client) = {
        let data = caller.data();
        (data.egress_allow.clone(), data.http_client.clone())
    };

    let envelope = match fetch_url(http_client.as_ref(), url, &egress_allow) {
        Ok(bytes) => bytes,
        Err(code) => return code,
    };

    if envelope.len() > out_cap {
        return ERR_TOO_LARGE;
    }

    if memory
        .write(&mut *caller, out_ptr as usize, &envelope)
        .is_err()
    {
        return ERR_BAD_ARGS;
    }

    i32::try_from(envelope.len()).unwrap_or(i32::MAX)
}

fn guest_memory(caller: &mut Caller<'_, StoreData>) -> Option<Memory> {
    caller.get_export("memory").and_then(|e| e.into_memory())
}

fn start_epoch_ticker(engine: &Engine, tick: Duration) {
    let engine = engine.clone();
    std::thread::Builder::new()
        .name("wasmtime-epoch".into())
        .spawn(move || loop {
            std::thread::sleep(tick);
            engine.increment_epoch();
        })
        .expect("spawn wasmtime epoch ticker");
}

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

    for import in module.imports() {
        let module_name = import.module();
        let field = import.name();
        if module_name == "nitrum" && field == "http_get" {
            match import.ty() {
                ExternType::Func(ty) => {
                    let params: Vec<_> = ty.params().collect();
                    let results: Vec<_> = ty.results().collect();
                    let ok = params.len() == 4
                        && params.iter().all(|p| p.is_i32())
                        && results.len() == 1
                        && results[0].is_i32();
                    if !ok {
                        return Err(AppError::Compile(format!(
                            "import `nitrum.http_get` must be (i32, i32, i32, i32) -> i32, got {ty}"
                        )));
                    }
                }
                other => {
                    return Err(AppError::Compile(format!(
                        "import `nitrum.http_get` must be a function, got {other:?}"
                    )));
                }
            }
        } else {
            return Err(AppError::Compile(format!(
                "unsupported import `{module_name}.{field}`"
            )));
        }
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

    #[instrument(skip(self, compiled, input, egress_allow), fields(hash = %hash, input_len = input.len()))]
    async fn run_precompiled(
        &self,
        hash: &ContentHash,
        compiled: &[u8],
        input: &[u8],
        egress_allow: &[EgressOrigin],
    ) -> Result<RunOutcome, AppError> {
        let _ = hash;
        let engine = self.engine.clone();
        let compiled = compiled.to_vec();
        let input = input.to_vec();
        let egress_allow = egress_allow.to_vec();
        let timeout = self.invoke_timeout;
        let tick = self.epoch_tick;
        let http_client = self.http_client.clone();
        let runner = WasmtimeRunner {
            engine: engine.clone(),
            invoke_timeout: timeout,
            epoch_tick: tick,
            http_client,
        };
        let output = tokio::task::spawn_blocking(move || {
            let module = Self::deserialize(&engine, &compiled)?;
            runner.invoke_sync(&module, &input, &egress_allow)
        })
        .await
        .map_err(join_err)??;
        Ok(RunOutcome { output })
    }

    #[instrument(skip(self, wasm, input, egress_allow), fields(hash = %hash, input_len = input.len()))]
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
        egress_allow: &[EgressOrigin],
    ) -> Result<RunOutcome, AppError> {
        let _ = hash;
        let engine = self.engine.clone();
        let wasm = wasm.to_vec();
        let input = input.to_vec();
        let egress_allow = egress_allow.to_vec();
        let timeout = self.invoke_timeout;
        let tick = self.epoch_tick;
        let http_client = self.http_client.clone();

        let runner = WasmtimeRunner {
            engine: engine.clone(),
            invoke_timeout: timeout,
            epoch_tick: tick,
            http_client,
        };
        let output = tokio::task::spawn_blocking(move || {
            let module =
                Module::new(&engine, &wasm).map_err(|e| AppError::Invoke(e.to_string()))?;
            assert_abi(&module).map_err(|e| match e {
                AppError::Compile(msg) => AppError::Invoke(msg),
                other => other,
            })?;
            runner.invoke_sync(&module, &input, &egress_allow)
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

    fn wasi_import_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (import "wasi_snapshot_preview1" "fd_write" (func))
              (memory (export "memory") 1)
              (func (export "invoke") (param i32 i32) (result i32)
                i32.const 0
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

    const NO_EGRESS: &[EgressOrigin] = &[];

    #[tokio::test]
    async fn echoes_payload() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = echo_wasm();
        let hash = ContentHash::from_bytes(&wasm);

        let first = runner.run(&hash, &wasm, b"hello", NO_EGRESS).await.expect("run");
        assert_eq!(first.output, b"hello");

        let second = runner.run(&hash, &wasm, b"world", NO_EGRESS).await.expect("run");
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
            .run_precompiled(&hash, &compiled, b"hello", NO_EGRESS)
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
    async fn compile_rejects_unknown_import() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = wasi_import_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let err = runner.compile(&hash, &wasm).await.expect_err("abi");
        match err {
            AppError::Compile(msg) => assert!(msg.contains("unsupported import"), "{msg}"),
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
        let err = runner.run(&hash, &wasm, b"x", NO_EGRESS).await.expect_err("trap");
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
        let err = runner.run(&hash, &wasm, b"x", NO_EGRESS).await.expect_err("timeout");
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
            .run_precompiled(&hash, &compiled, b"x", NO_EGRESS)
            .await
            .expect_err("timeout");
        assert!(matches!(err, AppError::Timeout(_)), "{err}");
    }

    #[tokio::test]
    async fn rejects_oversized_guest_output() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = huge_output_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let err = runner.run(&hash, &wasm, b"x", NO_EGRESS).await.expect_err("too large");
        assert!(matches!(err, AppError::PayloadTooLarge(_)), "{err}");
    }

    #[tokio::test]
    async fn memory_grow_past_limit_fails() {
        let runner = WasmtimeRunner::new().expect("engine");
        let wasm = grow_past_limit_wasm();
        let hash = ContentHash::from_bytes(&wasm);
        let result = runner.run(&hash, &wasm, b"ok", NO_EGRESS).await;
        match result {
            Ok(out) => assert_eq!(out.output, b"ok"),
            Err(AppError::Invoke(_)) | Err(AppError::Trap(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
}
