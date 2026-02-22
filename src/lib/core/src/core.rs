use std::collections::BTreeMap;
use std::io::{self, Write};
use std::ptr;
use std::sync::{Mutex, OnceLock};
static ALLOCATIONS: OnceLock<Mutex<BTreeMap<i32, usize>>> = OnceLock::new();

fn allocations() -> &'static Mutex<BTreeMap<i32, usize>> {
    ALLOCATIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn remember_allocation(ptr: i32, size: usize) {
    if ptr == 0 || size == 0 {
        return;
    }
    if let Ok(mut allocations) = allocations().lock() {
        allocations.insert(ptr, size);
    }
}

fn take_allocation(ptr: i32, size: usize) -> bool {
    let Ok(mut allocations) = allocations().lock() else {
        return false;
    };
    match allocations.get(&ptr).copied() {
        Some(expected) if expected == size => {
            allocations.remove(&ptr);
            true
        }
        _ => false,
    }
}

fn memory_end_is_valid(end: usize) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(memory_size_bytes) = core::arch::wasm32::memory_size(0).checked_mul(65_536) else {
            return false;
        };
        end <= memory_size_bytes
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = end;
        true
    }
}

fn checked_ptr_len(ptr: i32, len: i32) -> Option<(usize, usize)> {
    if ptr == 0 || len <= 0 {
        return None;
    }
    let offset = ptr as u32 as usize;
    let len = usize::try_from(len).ok()?;
    let end = offset.checked_add(len)?;
    if !memory_end_is_valid(end) {
        return None;
    }
    Some((offset, len))
}

fn checked_mut_ptr(ptr: i32, len: usize) -> Option<*mut u8> {
    if ptr == 0 || len == 0 {
        return None;
    }
    let offset = ptr as u32 as usize;
    let end = offset.checked_add(len)?;
    if !memory_end_is_valid(end) {
        return None;
    }
    Some(offset as *mut u8)
}

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let size = size as usize;
    let mut buf = Vec::<u8>::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    let ptr = ptr as i32;
    remember_allocation(ptr, size);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    let Some((offset, size)) = checked_ptr_len(ptr, size) else {
        return;
    };
    if !take_allocation(ptr, size) {
        return;
    }
    let _ = Vec::from_raw_parts(offset as *mut u8, 0, size);
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
pub extern "C" fn print(ptr: i32, len: i32) {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
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

    // ABI: upper 32 bits = ptr, lower 32 bits = len.
    ((ptr as u32 as u64) << 32 | (len as u32 as u64)) as i64
}
