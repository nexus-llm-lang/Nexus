use std::io::{self, Write};
use nexus_wasm_alloc::checked_ptr_len;

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn __nx_i64_to_string(val: i64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_float_to_string(val: f64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_bool_to_string(val: i32) -> i64 {
    let b = val != 0;
    nexus_wasm_alloc::store_string_result(b.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_print(ptr: i32, len: i32) {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

#[no_mangle]
pub extern "C" fn __nx_array_length(_ptr: i32, len: i32) -> i64 {
    len as i64
}
