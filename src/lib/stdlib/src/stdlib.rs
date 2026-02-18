use std::ptr;

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
pub extern "C" fn i64_to_string(val: i64) -> i64 {
    store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn float_to_string(val: f64) -> i64 {
    store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn bool_to_string(val: i32) -> i64 {
    let b = val != 0;
    store_string_result(b.to_string())
}

#[no_mangle]
pub extern "C" fn array_length(_ptr: i32, len: i32) -> i64 {
    len as i64
}

fn store_string_result(s: String) -> i64 {
    let bytes = s.into_bytes();
    let len = bytes.len();
    if len == 0 {
        return 0;
    }

    let ptr = allocate(len as i32);
    if ptr == 0 {
        return 0;
    }

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, len);
    }

    // ABI: upper 32 bits = ptr, lower 32 bits = len.
    ((ptr as u32 as u64) << 32 | (len as u32 as u64)) as i64
}
