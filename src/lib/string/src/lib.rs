use nexus_wasm_alloc::{checked_mut_ptr, checked_ptr_len};
use std::ptr;
use std::slice;

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

fn store_string_result(s: String) -> i64 {
    let bytes = s.into_bytes();
    let len = bytes.len();
    if len == 0 {
        return 0;
    }

    let Ok(len_i32) = i32::try_from(len) else {
        return 0;
    };

    let ptr = allocate(len_i32);
    if ptr == 0 {
        return 0;
    }

    let Some(dst) = checked_mut_ptr(ptr, len) else {
        unsafe { deallocate(ptr, len_i32) };
        return 0;
    };

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
    }

    ((ptr as u32 as u64) << 32 | (len as u32 as u64)) as i64
}

#[no_mangle]
pub extern "C" fn __nx_string_length(s_ptr: i32, s_len: i32) -> i64 {
    read_string(s_ptr, s_len).len() as i64
}

#[no_mangle]
pub extern "C" fn __nx_string_contains(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    if s.contains(sub.as_str()) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_substring(s_ptr: i32, s_len: i32, start: i64, len: i64) -> i64 {
    let s = read_string(s_ptr, s_len);
    let start = start.max(0) as usize;
    let len = len.max(0) as usize;
    let result: String = s.chars().skip(start).take(len).collect();
    store_string_result(result)
}

#[no_mangle]
pub extern "C" fn __nx_string_index_of(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn __nx_string_starts_with(
    s_ptr: i32,
    s_len: i32,
    prefix_ptr: i32,
    prefix_len: i32,
) -> i32 {
    let s = read_string(s_ptr, s_len);
    let prefix = read_string(prefix_ptr, prefix_len);
    if s.starts_with(prefix.as_str()) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_ends_with(
    s_ptr: i32,
    s_len: i32,
    suffix_ptr: i32,
    suffix_len: i32,
) -> i32 {
    let s = read_string(s_ptr, s_len);
    let suffix = read_string(suffix_ptr, suffix_len);
    if s.ends_with(suffix.as_str()) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_trim(s_ptr: i32, s_len: i32) -> i64 {
    store_string_result(read_string(s_ptr, s_len).trim().to_string())
}

#[no_mangle]
pub extern "C" fn __nx_string_to_upper(s_ptr: i32, s_len: i32) -> i64 {
    store_string_result(read_string(s_ptr, s_len).to_uppercase())
}

#[no_mangle]
pub extern "C" fn __nx_string_to_lower(s_ptr: i32, s_len: i32) -> i64 {
    store_string_result(read_string(s_ptr, s_len).to_lowercase())
}

#[no_mangle]
pub extern "C" fn __nx_string_replace(
    s_ptr: i32,
    s_len: i32,
    from_ptr: i32,
    from_len: i32,
    to_ptr: i32,
    to_len: i32,
) -> i64 {
    let s = read_string(s_ptr, s_len);
    let from = read_string(from_ptr, from_len);
    let to = read_string(to_ptr, to_len);
    store_string_result(s.replace(from.as_str(), to.as_str()))
}

#[no_mangle]
pub extern "C" fn __nx_string_char_at(s_ptr: i32, s_len: i32, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    let s = read_string(s_ptr, s_len);
    match s.chars().nth(idx as usize) {
        Some(c) => store_string_result(c.to_string()),
        None => 0,
    }
}
