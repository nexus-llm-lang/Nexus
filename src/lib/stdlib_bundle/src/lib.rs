// Force-link all sub-crate symbols so their #[no_mangle] exports appear in the
// final cdylib wasm module.  Each `extern crate` + `use` ensures the linker
// pulls in the rlib and preserves its exported functions.

extern crate nexus_stdio_wasm;
extern crate nexus_string_wasm;
extern crate nexus_net_wasm;
extern crate nexus_core_wasm;
extern crate nexus_math_wasm;
extern crate nexus_fs_wasm;
extern crate nexus_random_wasm;
extern crate nexus_clock_wasm;
extern crate nexus_proc_wasm;
extern crate nexus_collection_wasm;

// Reference at least one item from each crate to prevent the linker from
// discarding the crate entirely.  We use a single never-called function whose
// address is taken but never invoked.
#[used]
static _FORCE_LINK: [fn(); 0] = [];

// Single allocate / deallocate / cabi_realloc exported from the bundle.

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: i32,
    old_len: i32,
    align: i32,
    new_len: i32,
) -> i32 {
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
        let ptr = ptr as i32;
        nexus_wasm_alloc::remember_allocation(ptr, new_len);
        return ptr;
    }

    let old_len = old_len as usize;
    if !nexus_wasm_alloc::take_allocation(old_ptr, old_len) {
        return 0;
    }
    let Ok(old_layout) = Layout::from_size_align(old_len, align) else {
        nexus_wasm_alloc::remember_allocation(old_ptr, old_len);
        return 0;
    };
    let ptr = realloc(old_ptr as *mut u8, old_layout, new_len);
    if ptr.is_null() {
        nexus_wasm_alloc::remember_allocation(old_ptr, old_len);
        return 0;
    }
    let ptr = ptr as i32;
    nexus_wasm_alloc::remember_allocation(ptr, new_len);
    ptr
}
