//! Persistent WASI reactor. JSON facts cross the boundary; Rerail evaluates them.
#![no_main]

thread_local! {
    static RESPONSE: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Allocate a request. `facts_evaluate` takes ownership of this exact allocation.
#[unsafe(no_mangle)]
pub extern "C" fn facts_allocate(len: usize) -> *mut u8 {
    Box::into_raw(vec![0_u8; len].into_boxed_slice()) as *mut u8
}

/// Return the response pointer in the upper 32 bits, byte length in the lower 32.
/// The response remains valid until the next call; the caller copies it before then.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn facts_evaluate(ptr: *mut u8, len: usize) -> u64 {
    // SAFETY: the host passes the pointer and length returned by facts_allocate once.
    let request = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
    let result = (|| {
        let request: serde_json::Value = serde_json::from_slice(&request).map_err(|e| e.to_string())?;
        let source = request["source"].as_str().ok_or("source must be a string")?;
        rerail_engine::tools::evaluate_facts(source, request["trace"].as_bool().unwrap_or(false))
    })();
    let value = result.unwrap_or_else(|error: String| serde_json::json!({"error": error}));
    RESPONSE.with(|response| {
        let mut response = response.borrow_mut();
        *response = serde_json::to_vec(&value).expect("serialize facts");
        ((response.as_ptr() as u64) << 32) | response.len() as u64
    })
}
