use nexus_wasm_alloc::checked_ptr_len;
use std::slice;
const NX_MATH_ERROR_I64: i64 = i64::MIN;

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

fn read_string(ptr: i32, len: i32) -> String {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return String::new();
    };
    let bytes = unsafe { slice::from_raw_parts(offset as *const u8, len) };
    String::from_utf8_lossy(bytes).to_string()
}

fn math_error_i64(message: impl AsRef<str>) -> i64 {
    eprintln!("math error: {}", message.as_ref());
    NX_MATH_ERROR_I64
}

fn parse_i64_or_error(input: &str) -> Result<i64, String> {
    input
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("string_to_i64: invalid integer '{}'", input))
}

#[no_mangle]
pub extern "C" fn __nx_abs_i64(val: i64) -> i64 {
    val.abs()
}

#[no_mangle]
pub extern "C" fn __nx_max_i64(a: i64, b: i64) -> i64 {
    a.max(b)
}

#[no_mangle]
pub extern "C" fn __nx_min_i64(a: i64, b: i64) -> i64 {
    a.min(b)
}

#[no_mangle]
pub extern "C" fn __nx_mod_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        return math_error_i64("mod: division by zero");
    }
    a % b
}

#[no_mangle]
pub extern "C" fn __nx_abs_float(val: f64) -> f64 {
    val.abs()
}

#[no_mangle]
pub extern "C" fn __nx_sqrt(val: f64) -> f64 {
    val.sqrt()
}

#[no_mangle]
pub extern "C" fn __nx_floor(val: f64) -> f64 {
    val.floor()
}

#[no_mangle]
pub extern "C" fn __nx_ceil(val: f64) -> f64 {
    val.ceil()
}

#[no_mangle]
pub extern "C" fn __nx_pow(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

#[no_mangle]
pub extern "C" fn __nx_i64_to_float(val: i64) -> f64 {
    val as f64
}

#[no_mangle]
pub extern "C" fn __nx_float_to_i64(val: f64) -> i64 {
    val as i64
}

#[no_mangle]
pub extern "C" fn __nx_string_to_i64(s_ptr: i32, s_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    match parse_i64_or_error(&s) {
        Ok(n) => n,
        Err(err) => math_error_i64(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_i64_zero_returns_explicit_error_value() {
        assert_eq!(__nx_mod_i64(10, 0), NX_MATH_ERROR_I64);
    }

    #[test]
    fn parse_i64_or_error_reports_invalid_input() {
        let err = parse_i64_or_error("abc").expect_err("invalid input should return error");
        assert!(err.contains("invalid integer"), "unexpected error: {}", err);
    }
}
