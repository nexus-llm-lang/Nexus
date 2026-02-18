use std::io::{self, Write};

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let mut buf = Vec::<u8>::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as i32
}

#[no_mangle]
pub unsafe extern "C" fn print(ptr: i32, len: i32) {
    if ptr == 0 || len <= 0 {
        return;
    }
    let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    let _ = io::stdout().write_all(bytes);
    let _ = io::stdout().write_all(b"\n");
}
