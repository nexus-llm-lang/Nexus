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

/// Snapshot the allocator's internal allocation count. Pair with
/// `__nx_alloc_reset` to bulk-free everything allocated after the mark.
#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_mark() -> i32 {
    nexus_wasm_alloc::mark()
}

/// Free every allocation past `mark`, in LIFO order. Pointers freed here
/// become invalid; the caller must hold no live references into them.
#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_reset(mark: i32) {
    nexus_wasm_alloc::reset_to(mark);
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_array_length(_ptr: i32, len: i32) -> i64 {
    len as i64
}
