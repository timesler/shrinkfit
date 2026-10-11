//! C ABI for shrinkfit, shared by every non-Rust binding.
//!
//! The surface is deliberately tiny and string-based, so bindings stay thin
//! and new options never require binding changes:
//!
//! * `shrinkfit_shrink(input, input_len, options, options_len, out_len)`
//!   takes UTF-8 JSON input and a UTF-8 JSON options object, and returns a
//!   UTF-8 JSON envelope: `{"ok":true,"text":"…","report":{…}}` or
//!   `{"ok":false,"error":"…"}`.
//! * The returned buffer is owned by the library; release it with
//!   `shrinkfit_free(ptr, len)`.
//! * `shrinkfit_alloc` / `shrinkfit_free` let hosts that share linear memory
//!   with the library (WebAssembly) pass input buffers in.
//!
//! The same exports are used natively (Python via ctypes) and when compiled
//! to `wasm32-unknown-unknown` (TypeScript), with no binding generator.

use shrinkfit::{json, shrink_str, Options, Value};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Version string, NUL-terminated.
#[no_mangle]
pub extern "C" fn shrinkfit_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Allocate `len` bytes for the caller to fill (e.g. from WebAssembly).
/// Release with `shrinkfit_free(ptr, len)`.
#[no_mangle]
pub extern "C" fn shrinkfit_alloc(len: usize) -> *mut u8 {
    let buf = vec![0u8; len].into_boxed_slice();
    Box::into_raw(buf) as *mut u8
}

/// Free a buffer returned by `shrinkfit_alloc` or `shrinkfit_shrink`.
///
/// # Safety
/// `ptr` must come from this library with exactly this `len`, and must not
/// be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn shrinkfit_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

fn envelope_ok(text: String, report: Value) -> String {
    json::to_string(
        &Value::Object(vec![
            ("ok".into(), Value::Bool(true)),
            ("text".into(), Value::String(text)),
            ("report".into(), report),
        ]),
        json::Layout::Compact,
    )
}

fn envelope_err(msg: String) -> String {
    json::to_string(
        &Value::Object(vec![
            ("ok".into(), Value::Bool(false)),
            ("error".into(), Value::String(msg)),
        ]),
        json::Layout::Compact,
    )
}

fn run(input: &[u8], options: &[u8]) -> String {
    let input = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => return envelope_err("input is not valid UTF-8".into()),
    };
    let options = match std::str::from_utf8(options) {
        Ok(s) => s,
        Err(_) => return envelope_err("options are not valid UTF-8".into()),
    };
    let opts_value = match json::parse(options) {
        Ok(v) => v,
        Err(e) => return envelope_err(format!("invalid options JSON: {e}")),
    };
    let opts = match Options::from_value(&opts_value) {
        Ok(o) => o,
        Err(e) => return envelope_err(format!("invalid options: {e}")),
    };
    let out = match shrink_str(input, &opts) {
        Ok(o) => o,
        Err(e) => return envelope_err(e.to_string()),
    };
    envelope_ok(out.text, out.report.to_value())
}

unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Shrink a JSON document. Returns a JSON envelope (see module docs) and
/// writes its length to `*out_len`. Never returns NULL.
///
/// # Safety
/// `input`/`options` must point to `input_len`/`options_len` readable bytes
/// (or be NULL with length 0), and `out_len` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn shrinkfit_shrink(
    input: *const u8,
    input_len: usize,
    options: *const u8,
    options_len: usize,
    out_len: *mut usize,
) -> *mut u8 {
    let input = slice(input, input_len);
    let options = slice(options, options_len);
    let result = catch_unwind(AssertUnwindSafe(|| run(input, options)))
        .unwrap_or_else(|_| envelope_err("internal error (panic) in shrinkfit".into()));
    let bytes = result.into_bytes().into_boxed_slice();
    *out_len = bytes.len();
    Box::into_raw(bytes) as *mut u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(input: &str, options: &str) -> String {
        let mut len = 0usize;
        unsafe {
            let p = shrinkfit_shrink(
                input.as_ptr(),
                input.len(),
                options.as_ptr(),
                options.len(),
                &mut len,
            );
            let s = std::str::from_utf8(std::slice::from_raw_parts(p, len))
                .unwrap()
                .to_string();
            shrinkfit_free(p, len);
            s
        }
    }

    #[test]
    fn ok_and_error_envelopes() {
        let ok = call(r#"[1,2,3,4,5,6,7,8,9,10]"#, r#"{"max_chars":15}"#);
        assert!(ok.starts_with(r#"{"ok":true,"text":"#), "{ok}");
        assert!(call("[1", r#"{"max_chars":15}"#).contains("invalid JSON input"));
        assert!(call("[1]", r#"{"bogus":1}"#).contains("unknown option"));
        assert!(call("[1]", "").contains("invalid options JSON"));
    }

    #[test]
    fn yaml_input() {
        let ok = call(
            "a: [1, 2, 3]
",
            r#"{"max_chars":100,"input_format":"yaml"}"#,
        );
        assert!(ok.contains(r#""text":"{\"a\":[1,2,3]}""#), "{ok}");
        assert!(call("a: [1", r#"{"max_chars":100,"input_format":"yaml"}"#)
            .contains("invalid YAML input"));
    }

    #[test]
    fn alloc_free_roundtrip() {
        let p = shrinkfit_alloc(32);
        assert!(!p.is_null());
        unsafe { shrinkfit_free(p, 32) };
    }
}
