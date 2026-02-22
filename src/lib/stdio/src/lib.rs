use nexus_wasm_alloc::checked_ptr_len;
use std::io::{self, Write};

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn print(ptr: i32, len: i32) {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}
