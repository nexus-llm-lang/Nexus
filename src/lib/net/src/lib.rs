use nexus_wasm_alloc::{checked_ptr_len, remember_allocation, take_allocation};
use std::alloc::{Layout, alloc, realloc};

const HOST_HTTP_MODULE: &str = "nexus:cli/nexus-host";
const HOST_HTTP_FUNC: &str = "host-http-request";

fn has_valid_optional_region(ptr: i32, len: i32) -> bool {
    if len < 0 {
        return false;
    }
    if len == 0 {
        return true;
    }
    checked_ptr_len(ptr, len).is_some()
}

#[link(wasm_import_module = "nexus:cli/nexus-host")]
extern "C" {
    #[link_name = "host-http-request"]
    fn host_http_request(
        method_ptr: i32,
        method_len: i32,
        url_ptr: i32,
        url_len: i32,
        headers_ptr: i32,
        headers_len: i32,
        body_ptr: i32,
        body_len: i32,
        ret_ptr: i32,
    );

    #[link_name = "host-http-listen"]
    fn host_http_listen(addr_ptr: i32, addr_len: i32) -> i64;

    #[link_name = "host-http-accept"]
    fn host_http_accept(server_id: i64, ret_ptr: i32);

    #[link_name = "host-http-respond"]
    fn host_http_respond(
        req_id: i64,
        status: i64,
        headers_ptr: i32,
        headers_len: i32,
        body_ptr: i32,
        body_len: i32,
    ) -> i32;

    #[link_name = "host-http-stop"]
    fn host_http_stop(server_id: i64) -> i32;
}

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
pub extern "C" fn __nx_http_get(url_ptr: i32, url_len: i32) -> i64 {
    const GET: &[u8] = b"GET";
    __nx_http_request(
        GET.as_ptr() as i32,
        GET.len() as i32,
        url_ptr,
        url_len,
        0,
        0,
        0,
        0,
    )
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_http_request(
    method_ptr: i32,
    method_len: i32,
    url_ptr: i32,
    url_len: i32,
    headers_ptr: i32,
    headers_len: i32,
    body_ptr: i32,
    body_len: i32,
) -> i64 {
    if checked_ptr_len(method_ptr, method_len).is_none() {
        return 0;
    }
    if checked_ptr_len(url_ptr, url_len).is_none() {
        return 0;
    }
    if !has_valid_optional_region(headers_ptr, headers_len) {
        return 0;
    }
    if !has_valid_optional_region(body_ptr, body_len) {
        return 0;
    }

    let mut ret = [0_i32; 2];
    unsafe {
        host_http_request(
            method_ptr,
            method_len,
            url_ptr,
            url_len,
            headers_ptr,
            headers_len,
            body_ptr,
            body_len,
            ret.as_mut_ptr() as i32,
        );
    }
    if !has_valid_optional_region(ret[0], ret[1]) {
        return 0;
    }
    pack_ptr_len(ret[0], ret[1])
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_http_listen(addr_ptr: i32, addr_len: i32) -> i64 {
    if checked_ptr_len(addr_ptr, addr_len).is_none() {
        return -1;
    }
    unsafe { host_http_listen(addr_ptr, addr_len) }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_http_accept(server_id: i64) -> i64 {
    let mut ret = [0_i32; 2];
    unsafe {
        host_http_accept(server_id, ret.as_mut_ptr() as i32);
    }
    if !has_valid_optional_region(ret[0], ret[1]) {
        return 0;
    }
    pack_ptr_len(ret[0], ret[1])
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_http_respond(
    req_id: i64,
    status: i64,
    headers_ptr: i32,
    headers_len: i32,
    body_ptr: i32,
    body_len: i32,
) -> i32 {
    if !has_valid_optional_region(headers_ptr, headers_len) {
        return 0;
    }
    if !has_valid_optional_region(body_ptr, body_len) {
        return 0;
    }
    unsafe { host_http_respond(req_id, status, headers_ptr, headers_len, body_ptr, body_len) }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_http_stop(server_id: i64) -> i32 {
    unsafe { host_http_stop(server_id) }
}

fn pack_ptr_len(ptr: i32, len: i32) -> i64 {
    ((ptr as u32 as u64) << 32 | (len as u32 as u64)) as i64
}

// Exported for component canonical ABI lowering of string returns.
#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: i32,
    old_len: i32,
    align: i32,
    new_len: i32,
) -> i32 {
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
        remember_allocation(ptr, new_len);
        return ptr;
    }

    let old_len = old_len as usize;
    if !take_allocation(old_ptr, old_len) {
        return 0;
    }
    let Ok(old_layout) = Layout::from_size_align(old_len, align) else {
        remember_allocation(old_ptr, old_len);
        return 0;
    };
    let ptr = realloc(old_ptr as *mut u8, old_layout, new_len);
    if ptr.is_null() {
        remember_allocation(old_ptr, old_len);
        return 0;
    }
    let ptr = ptr as i32;
    remember_allocation(ptr, new_len);
    ptr
}

