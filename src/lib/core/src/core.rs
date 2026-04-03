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

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_array_length(_ptr: i32, len: i32) -> i64 {
    len as i64
}
