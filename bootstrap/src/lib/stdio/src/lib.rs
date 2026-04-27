use std::io::{self, BufRead, Read, Write};

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_mark() -> i32 {
    nexus_wasm_alloc::mark()
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_reset(mark: i32) {
    nexus_wasm_alloc::reset_to(mark);
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_print(ptr: i32, len: i32) {
    let Some((offset, len)) = nexus_wasm_alloc::checked_ptr_len(ptr, len) else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_eprint(ptr: i32, len: i32) {
    let Some((offset, len)) = nexus_wasm_alloc::checked_ptr_len(ptr, len) else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let mut out = io::stderr();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_read_line() -> i64 {
    let stdin = io::stdin();
    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(_) => {
            // Strip trailing newline
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            nexus_wasm_alloc::store_string_result(line)
        }
        Err(_) => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_getchar() -> i64 {
    let stdin = io::stdin();
    let mut buf = [0u8; 4]; // max UTF-8 bytes per char
    let mut handle = stdin.lock();
    match handle.read(&mut buf[..1]) {
        Ok(0) | Err(_) => nexus_wasm_alloc::store_string_result(String::new()),
        Ok(_) => {
            // Determine UTF-8 byte count from leading byte
            let width = if buf[0] < 0x80 { 1 }
                else if buf[0] < 0xE0 { 2 }
                else if buf[0] < 0xF0 { 3 }
                else { 4 };
            if width > 1 {
                let _ = handle.read_exact(&mut buf[1..width]);
            }
            let s = String::from_utf8_lossy(&buf[..width]).into_owned();
            nexus_wasm_alloc::store_string_result(s)
        }
    }
}
