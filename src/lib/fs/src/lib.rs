use nexus_wasm_alloc::{checked_mut_ptr, checked_ptr_len};
use std::fs::OpenOptions;

// Helper to allocate memory for string return (simple malloc)
#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn read_to_string(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    let content = std::fs::read_to_string(path).unwrap_or_default();
    store_string_result(content)
}

#[no_mangle]
pub extern "C" fn write_string(
    path_ptr: i32,
    path_len: i32,
    content_ptr: i32,
    content_len: i32,
) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    let Some(content) = read_optional_string(content_ptr, content_len) else {
        return 0;
    };
    match std::fs::write(path, content) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn append_string(
    path_ptr: i32,
    path_len: i32,
    content_ptr: i32,
    content_len: i32,
) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    let Some(content) = read_optional_string(content_ptr, content_len) else {
        return 0;
    };

    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => match std::io::Write::write_all(&mut f, content.as_bytes()) {
            Ok(_) => 1,
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn exists(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    if std::path::Path::new(&path).exists() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn remove_file(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    match std::fs::remove_file(path) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn create_dir_all(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    match std::fs::create_dir_all(path) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn read_dir(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };

    let mut names = Vec::new();
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    store_string_result(names.join("\n"))
}

fn read_required_string(ptr: i32, len: i32) -> Option<String> {
    let (offset, len) = checked_ptr_len(ptr, len)?;
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    Some(String::from_utf8_lossy(bytes).to_string())
}

fn read_optional_string(ptr: i32, len: i32) -> Option<String> {
    if len < 0 {
        return None;
    }
    if len == 0 {
        return Some(String::new());
    }
    let (offset, len) = checked_ptr_len(ptr, len)?;
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    Some(String::from_utf8_lossy(bytes).to_string())
}

fn read_string_lossy(ptr: i32, len: i32) -> String {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return String::new();
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    String::from_utf8_lossy(bytes).to_string()
}

#[no_mangle]
pub extern "C" fn __nx_string_length(s_ptr: i32, s_len: i32) -> i64 {
    read_string_lossy(s_ptr, s_len).len() as i64
}

#[no_mangle]
pub extern "C" fn __nx_string_index_of(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i64 {
    let s = read_string_lossy(s_ptr, s_len);
    let sub = read_string_lossy(sub_ptr, sub_len);
    s.find(sub.as_str()).map(|idx| idx as i64).unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn __nx_string_substring(s_ptr: i32, s_len: i32, start: i64, len: i64) -> i64 {
    let s = read_string_lossy(s_ptr, s_len);
    let start = start.max(0) as usize;
    let len = len.max(0) as usize;
    let result: String = s.chars().skip(start).take(len).collect();
    store_string_result(result)
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
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
    }

    ((ptr as u32 as u64) << 32 | (len as u32 as u64)) as i64
}
