//! Host ↔ guest byte ABI. Function authors use [`crate::run`], not this module.

use crate::register::call_handler;
use crate::wire::{decode_request, encode_response};
use crate::{Error, Response};

/// Exported entrypoint the host calls for each request.
#[no_mangle]
pub extern "C" fn invoke(ptr: i32, len: i32) -> i32 {
    if ptr < 0 || len < 0 {
        return -1;
    }

    let ptr = ptr as usize;
    let len = len as usize;

    // SAFETY: host places the wire JSON at `ptr` for `len` bytes.
    let input = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };

    let response = match handle(input) {
        Ok(res) => res,
        Err(err) => error_response(err),
    };

    let Ok(output) = encode_response(&response) else {
        return -1;
    };

    if ensure_memory(ptr.saturating_add(output.len())).is_err() {
        return -1;
    }

    // SAFETY: memory grown to fit `output`; host reads that many bytes from `ptr`.
    unsafe {
        core::ptr::copy_nonoverlapping(output.as_ptr(), ptr as *mut u8, output.len());
    }

    output.len() as i32
}

fn handle(input: &[u8]) -> Result<Response, Error> {
    let req = decode_request(input)?;
    call_handler(req)
}

fn error_response(err: Error) -> Response {
    let msg = serde_json::to_string(&err.to_string())
        .unwrap_or_else(|_| "\"internal error\"".into());
    Response::builder()
        .status(500)
        .header("content-type", "application/json")
        .body(format!(r#"{{"error":{msg}}}"#).into_bytes())
        .build()
}

fn ensure_memory(end: usize) -> Result<(), ()> {
    let pages_needed = end.div_ceil(65536);
    let current = core::arch::wasm32::memory_size(0);
    if pages_needed > current {
        let delta = pages_needed - current;
        if core::arch::wasm32::memory_grow(0, delta) == usize::MAX {
            return Err(());
        }
    }
    Ok(())
}
