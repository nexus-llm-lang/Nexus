// Component model build: wit-bindgen generates export adapters for all WIT interfaces.
// The Guest trait implementations delegate to the underlying sub-crate functions.
#[cfg(feature = "component")]
mod component;

// Non-component build: force-link all sub-crate symbols via extern crate.
#[cfg(not(feature = "component"))]
mod legacy;

// Common: allocate / deallocate / cabi_realloc always available for the component.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

/// cabi_realloc for canonical ABI. Delegates to the system allocator
/// (same as nexus_wasm_alloc). Allocations are tracked for proper dealloc.
#[no_mangle]
pub unsafe extern "C" fn cabi_realloc(old_ptr: i32, old_len: i32, align: i32, new_len: i32) -> i32 {
    use std::alloc::{alloc, realloc, Layout};

    if new_len <= 0 {
        return 0;
    }
    let align = align.max(1) as usize;
    let new_len = new_len as usize;

    if old_ptr == 0 || old_len <= 0 {
        let Ok(layout) = Layout::from_size_align(new_len, align) else {
            return 0;
        };
        let ptr = alloc(layout);
        ptr as i32
    } else {
        let old_len = old_len as usize;
        let Ok(old_layout) = Layout::from_size_align(old_len, align) else {
            return 0;
        };
        let ptr = realloc(old_ptr as *mut u8, old_layout, new_len);
        if ptr.is_null() {
            0
        } else {
            ptr as i32
        }
    }
}
