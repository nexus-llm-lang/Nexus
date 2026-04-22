use nexus_wasm_alloc::checked_ptr_len;
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

// --- fd table infrastructure ---

struct FdEntry {
    file: File,
    path: String,
    mode: u8, // 0=read, 1=write, 2=append
}

thread_local! {
    static FD_TABLE: RefCell<Vec<Option<FdEntry>>> = RefCell::new(Vec::new());
}

fn fd_alloc(entry: FdEntry) -> i64 {
    FD_TABLE.with(|table| {
        let mut table = table.borrow_mut();
        // Reuse a vacant slot if available.
        for (i, slot) in table.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(entry);
                return i as i64;
            }
        }
        let idx = table.len();
        table.push(Some(entry));
        idx as i64
    })
}

fn fd_take(fd: i64) -> Option<FdEntry> {
    if fd < 0 {
        return None;
    }
    FD_TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let idx = fd as usize;
        if idx < table.len() {
            table[idx].take()
        } else {
            None
        }
    })
}

fn fd_with<R>(fd: i64, f: impl FnOnce(&mut FdEntry) -> R) -> Option<R> {
    if fd < 0 {
        return None;
    }
    FD_TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let idx = fd as usize;
        if idx < table.len() {
            table[idx].as_mut().map(f)
        } else {
            None
        }
    })
}

// --- allocate / deallocate ---

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

// --- path-level operations (unchanged) ---

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_read_to_string(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    let content = std::fs::read_to_string(path).unwrap_or_default();
    nexus_wasm_alloc::store_string_result(content)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_write_string(
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

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_append_string(
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

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_exists(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    if std::path::Path::new(&path).exists() {
        1
    } else {
        0
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_is_file(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    if std::path::Path::new(&path).is_file() {
        1
    } else {
        0
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_remove_file(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    match std::fs::remove_file(path) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_create_dir_all(path_ptr: i32, path_len: i32) -> i32 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return 0;
    };
    match std::fs::create_dir_all(path) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_read_dir(path_ptr: i32, path_len: i32) -> i64 {
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
    nexus_wasm_alloc::store_string_result(names.join("\n"))
}

// --- fd-based operations ---

/// Opens a file for reading. Returns fd (>= 0) on success, -1 on error.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_open_read(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return -1;
    };
    match File::open(&path) {
        Ok(file) => fd_alloc(FdEntry {
            file,
            path,
            mode: 0,
        }),
        Err(_) => -1,
    }
}

/// Opens a file for writing (truncate+create). Returns fd (>= 0) on success, -1 on error.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_open_write(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return -1;
    };
    match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
    {
        Ok(file) => fd_alloc(FdEntry {
            file,
            path,
            mode: 1,
        }),
        Err(_) => -1,
    }
}

/// Opens a file for appending (create if not exists). Returns fd (>= 0) on success, -1 on error.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_open_append(path_ptr: i32, path_len: i32) -> i64 {
    let Some(path) = read_required_string(path_ptr, path_len) else {
        return -1;
    };
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => fd_alloc(FdEntry {
            file,
            path,
            mode: 2,
        }),
        Err(_) => -1,
    }
}

/// Closes an fd. Returns 1 on success, 0 on bad fd.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_close(fd: i64) -> i32 {
    match fd_take(fd) {
        Some(_entry) => 1, // File dropped here
        None => 0,
    }
}

/// Reads the entire file contents from fd. Seeks to 0 first.
/// Returns packed ptr|len as i64 (0 on failure).
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_read(fd: i64) -> i64 {
    let result = fd_with(fd, |entry| {
        let _ = entry.file.seek(SeekFrom::Start(0));
        let mut buf = String::new();
        match entry.file.read_to_string(&mut buf) {
            Ok(_) => Some(buf),
            Err(_) => None,
        }
    });
    match result {
        Some(Some(content)) => nexus_wasm_alloc::store_string_result(content),
        _ => 0,
    }
}

/// Writes content to fd. Returns 1 on success, 0 on failure.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_write(fd: i64, content_ptr: i32, content_len: i32) -> i32 {
    let Some(content) = read_optional_string(content_ptr, content_len) else {
        return 0;
    };
    let result = fd_with(fd, |entry| {
        entry.file.write_all(content.as_bytes()).is_ok()
    });
    match result {
        Some(true) => 1,
        _ => 0,
    }
}

/// Returns the path string for an fd. Returns packed ptr|len as i64 (0 on bad fd).
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_fd_path(fd: i64) -> i64 {
    let result = fd_with(fd, |entry| entry.path.clone());
    match result {
        Some(path) => nexus_wasm_alloc::store_string_result(path),
        None => 0,
    }
}

// --- string helpers (used by fs.nx) ---

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

