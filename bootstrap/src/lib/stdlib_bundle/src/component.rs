// wit-bindgen component model adapter.
//
// Generates export wrappers for all WIT interfaces defined in wit/world.wit.
// Each Guest trait delegates to the underlying sub-crate #[no_mangle] functions
// (which are suppressed from export via the `component` feature flag).

mod bindings {
    wit_bindgen::generate!({
        world: "stdlib",
        path: "wit",
        generate_all,
    });
}

use bindings::exports::nexus::r#std::{
    bytebuffer, clock, collections, core as nx_core, env, fs, math, network,
    proc, rand, stdio, str,
};

// ---------------------------------------------------------------------------
// Helpers: unpack packed i64 string result from sub-crate FFI functions
// ---------------------------------------------------------------------------

/// Read a packed `(ptr << 32) | len` result from an FFI function and return a String.
/// The pointer refers to the component's own linear memory.
fn unpack_string(packed: i64) -> String {
    if packed == 0 {
        return String::new();
    }
    let ptr = (packed as u64 >> 32) as usize;
    let len = (packed as u64 & 0xFFFF_FFFF) as usize;
    if ptr == 0 || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
        String::from_utf8_unchecked(bytes.to_vec())
    }
}

// ---------------------------------------------------------------------------
// Single component struct implementing all Guest traits
// ---------------------------------------------------------------------------

struct StdlibComponent;

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

impl math::Guest for StdlibComponent {
    fn abs_i64(val: i64) -> i64 {
        nexus_math_wasm::__nx_abs_i64(val)
    }
    fn max_i64(a: i64, b: i64) -> i64 {
        nexus_math_wasm::__nx_max_i64(a, b)
    }
    fn min_i64(a: i64, b: i64) -> i64 {
        nexus_math_wasm::__nx_min_i64(a, b)
    }
    fn mod_i64(a: i64, b: i64) -> i64 {
        nexus_math_wasm::__nx_mod_i64(a, b)
    }
    fn abs_float(val: f64) -> f64 {
        nexus_math_wasm::__nx_abs_float(val)
    }
    fn sqrt(val: f64) -> f64 {
        nexus_math_wasm::__nx_sqrt(val)
    }
    fn floor(val: f64) -> f64 {
        nexus_math_wasm::__nx_floor(val)
    }
    fn ceil(val: f64) -> f64 {
        nexus_math_wasm::__nx_ceil(val)
    }
    fn pow(base: f64, exp: f64) -> f64 {
        nexus_math_wasm::__nx_pow(base, exp)
    }
    fn i64_to_float(val: i64) -> f64 {
        nexus_math_wasm::__nx_i64_to_float(val)
    }
    fn float_to_i64(val: f64) -> i64 {
        nexus_math_wasm::__nx_float_to_i64(val)
    }
}

// ---------------------------------------------------------------------------
// Str
// ---------------------------------------------------------------------------

impl str::Guest for StdlibComponent {
    fn string_length(s: String) -> i64 {
        nexus_string_wasm::__nx_string_length(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_byte_length(s: String) -> i64 {
        nexus_string_wasm::__nx_string_byte_length(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_contains(s: String, sub: String) -> i32 {
        nexus_string_wasm::__nx_string_contains(
            s.as_ptr() as i32,
            s.len() as i32,
            sub.as_ptr() as i32,
            sub.len() as i32,
        )
    }
    fn string_substring(s: String, start: i64, len: i64) -> String {
        let packed =
            nexus_string_wasm::__nx_string_substring(s.as_ptr() as i32, s.len() as i32, start, len);
        unpack_string(packed)
    }
    fn string_index_of(s: String, sub: String) -> i64 {
        nexus_string_wasm::__nx_string_index_of(
            s.as_ptr() as i32,
            s.len() as i32,
            sub.as_ptr() as i32,
            sub.len() as i32,
        )
    }
    fn string_starts_with(s: String, prefix: String) -> i32 {
        nexus_string_wasm::__nx_string_starts_with(
            s.as_ptr() as i32,
            s.len() as i32,
            prefix.as_ptr() as i32,
            prefix.len() as i32,
        )
    }
    fn string_ends_with(s: String, suffix: String) -> i32 {
        nexus_string_wasm::__nx_string_ends_with(
            s.as_ptr() as i32,
            s.len() as i32,
            suffix.as_ptr() as i32,
            suffix.len() as i32,
        )
    }
    fn string_trim(s: String) -> String {
        let packed = nexus_string_wasm::__nx_string_trim(s.as_ptr() as i32, s.len() as i32);
        unpack_string(packed)
    }
    fn string_to_upper(s: String) -> String {
        let packed = nexus_string_wasm::__nx_string_to_upper(s.as_ptr() as i32, s.len() as i32);
        unpack_string(packed)
    }
    fn string_to_lower(s: String) -> String {
        let packed = nexus_string_wasm::__nx_string_to_lower(s.as_ptr() as i32, s.len() as i32);
        unpack_string(packed)
    }
    fn string_replace(s: String, pattern: String, replacement: String) -> String {
        let packed = nexus_string_wasm::__nx_string_replace(
            s.as_ptr() as i32,
            s.len() as i32,
            pattern.as_ptr() as i32,
            pattern.len() as i32,
            replacement.as_ptr() as i32,
            replacement.len() as i32,
        );
        unpack_string(packed)
    }
    fn string_char_at(s: String, idx: i64) -> i32 {
        nexus_string_wasm::__nx_string_char_at(s.as_ptr() as i32, s.len() as i32, idx)
    }
    fn string_from_char(c: i32) -> String {
        unpack_string(nexus_string_wasm::__nx_string_from_char(c))
    }
    fn string_from_i64(val: i64) -> String {
        unpack_string(nexus_string_wasm::__nx_string_from_i64(val))
    }
    fn string_from_float(val: f64) -> String {
        unpack_string(nexus_string_wasm::__nx_string_from_float(val))
    }
    fn string_from_bool(val: i32) -> String {
        unpack_string(nexus_string_wasm::__nx_string_from_bool(val))
    }
    fn string_to_i64(s: String) -> i64 {
        nexus_string_wasm::__nx_string_to_i64(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_repeat(s: String, n: i64) -> String {
        let packed = nexus_string_wasm::__nx_string_repeat(s.as_ptr() as i32, s.len() as i32, n);
        unpack_string(packed)
    }
    fn string_pad_left(s: String, width: i64, fill: String) -> String {
        let packed = nexus_string_wasm::__nx_string_pad_left(
            s.as_ptr() as i32,
            s.len() as i32,
            width,
            fill.as_ptr() as i32,
            fill.len() as i32,
        );
        unpack_string(packed)
    }
    fn string_pad_right(s: String, width: i64, fill: String) -> String {
        let packed = nexus_string_wasm::__nx_string_pad_right(
            s.as_ptr() as i32,
            s.len() as i32,
            width,
            fill.as_ptr() as i32,
            fill.len() as i32,
        );
        unpack_string(packed)
    }
    fn string_is_valid_i64(s: String) -> i32 {
        nexus_string_wasm::__nx_string_is_valid_i64(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_char_code(s: String, idx: i64) -> i64 {
        nexus_string_wasm::__nx_string_char_code(s.as_ptr() as i32, s.len() as i32, idx)
    }
    fn char_ord(c: i32) -> i64 {
        nexus_string_wasm::__nx_char_ord(c)
    }
    fn string_from_char_code(code: i64) -> String {
        unpack_string(nexus_string_wasm::__nx_string_from_char_code(code))
    }
    fn string_to_f64(s: String) -> f64 {
        nexus_string_wasm::__nx_string_to_f64(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_is_valid_f64(s: String) -> i32 {
        nexus_string_wasm::__nx_string_is_valid_f64(s.as_ptr() as i32, s.len() as i32)
    }
    fn string_byte_at(s: String, idx: i64) -> i32 {
        nexus_string_wasm::__nx_string_byte_at(s.as_ptr() as i32, s.len() as i32, idx)
    }
    fn string_scan_ident(s: String, start: i64) -> i64 {
        nexus_string_wasm::__nx_string_scan_ident(s.as_ptr() as i32, s.len() as i32, start)
    }
    fn string_scan_digits(s: String, start: i64) -> i64 {
        nexus_string_wasm::__nx_string_scan_digits(s.as_ptr() as i32, s.len() as i32, start)
    }
    fn string_skip_ws(s: String, start: i64) -> i64 {
        nexus_string_wasm::__nx_string_skip_ws(s.as_ptr() as i32, s.len() as i32, start)
    }
    fn string_count_newlines_in(s: String, start: i64, end: i64) -> i64 {
        nexus_string_wasm::__nx_string_count_newlines_in(
            s.as_ptr() as i32,
            s.len() as i32,
            start,
            end,
        )
    }
    fn string_last_newline_in(s: String, start: i64, end: i64) -> i64 {
        nexus_string_wasm::__nx_string_last_newline_in(
            s.as_ptr() as i32,
            s.len() as i32,
            start,
            end,
        )
    }
    fn string_find_byte(s: String, start: i64, ch: i32) -> i64 {
        nexus_string_wasm::__nx_string_find_byte(s.as_ptr() as i32, s.len() as i32, start, ch)
    }
    fn string_byte_substring(s: String, start: i64, len: i64) -> String {
        let packed = nexus_string_wasm::__nx_string_byte_substring(
            s.as_ptr() as i32,
            s.len() as i32,
            start,
            len,
        );
        unpack_string(packed)
    }
}

// ---------------------------------------------------------------------------
// Stdio
// ---------------------------------------------------------------------------

impl stdio::Guest for StdlibComponent {
    fn print(s: String) {
        nexus_stdio_wasm::__nx_print(s.as_ptr() as i32, s.len() as i32);
    }
    fn eprint(s: String) {
        nexus_stdio_wasm::__nx_eprint(s.as_ptr() as i32, s.len() as i32);
    }
    fn read_line() -> String {
        unpack_string(nexus_stdio_wasm::__nx_read_line())
    }
    fn getchar() -> String {
        unpack_string(nexus_stdio_wasm::__nx_getchar())
    }
}

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

impl fs::Guest for StdlibComponent {
    fn read_to_string(path: String) -> String {
        let packed = nexus_fs_wasm::__nx_read_to_string(path.as_ptr() as i32, path.len() as i32);
        unpack_string(packed)
    }
    fn write_string(path: String, content: String) -> i32 {
        nexus_fs_wasm::__nx_write_string(
            path.as_ptr() as i32,
            path.len() as i32,
            content.as_ptr() as i32,
            content.len() as i32,
        )
    }
    fn append_string(path: String, content: String) -> i32 {
        nexus_fs_wasm::__nx_append_string(
            path.as_ptr() as i32,
            path.len() as i32,
            content.as_ptr() as i32,
            content.len() as i32,
        )
    }
    fn exists(path: String) -> i32 {
        nexus_fs_wasm::__nx_exists(path.as_ptr() as i32, path.len() as i32)
    }
    fn is_file(path: String) -> i32 {
        nexus_fs_wasm::__nx_is_file(path.as_ptr() as i32, path.len() as i32)
    }
    fn remove_file(path: String) -> i32 {
        nexus_fs_wasm::__nx_remove_file(path.as_ptr() as i32, path.len() as i32)
    }
    fn create_dir_all(path: String) -> i32 {
        nexus_fs_wasm::__nx_create_dir_all(path.as_ptr() as i32, path.len() as i32)
    }
    fn read_dir(path: String) -> String {
        let packed = nexus_fs_wasm::__nx_read_dir(path.as_ptr() as i32, path.len() as i32);
        unpack_string(packed)
    }
    fn fd_open_read(path: String) -> i64 {
        nexus_fs_wasm::__nx_fd_open_read(path.as_ptr() as i32, path.len() as i32)
    }
    fn fd_open_write(path: String) -> i64 {
        nexus_fs_wasm::__nx_fd_open_write(path.as_ptr() as i32, path.len() as i32)
    }
    fn fd_open_append(path: String) -> i64 {
        nexus_fs_wasm::__nx_fd_open_append(path.as_ptr() as i32, path.len() as i32)
    }
    fn fd_close(fd: i64) -> i32 {
        nexus_fs_wasm::__nx_fd_close(fd)
    }
    fn fd_read(fd: i64) -> String {
        unpack_string(nexus_fs_wasm::__nx_fd_read(fd))
    }
    fn fd_write(fd: i64, content: String) -> i32 {
        nexus_fs_wasm::__nx_fd_write(fd, content.as_ptr() as i32, content.len() as i32)
    }
    fn fd_path(fd: i64) -> String {
        unpack_string(nexus_fs_wasm::__nx_fd_path(fd))
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

impl network::Guest for StdlibComponent {
    fn http_get(url: String) -> String {
        let packed = nexus_net_wasm::__nx_http_get(url.as_ptr() as i32, url.len() as i32);
        unpack_string(packed)
    }
    fn http_request(method: String, url: String, headers: String, body: String) -> String {
        let packed = nexus_net_wasm::__nx_http_request(
            method.as_ptr() as i32,
            method.len() as i32,
            url.as_ptr() as i32,
            url.len() as i32,
            headers.as_ptr() as i32,
            headers.len() as i32,
            body.as_ptr() as i32,
            body.len() as i32,
        );
        unpack_string(packed)
    }
    fn http_listen(addr: String) -> i64 {
        nexus_net_wasm::__nx_http_listen(addr.as_ptr() as i32, addr.len() as i32)
    }
    fn http_accept(server_id: i64) -> String {
        unpack_string(nexus_net_wasm::__nx_http_accept(server_id))
    }
    fn http_respond(req_id: i64, status: i64, headers: String, body: String) -> i32 {
        nexus_net_wasm::__nx_http_respond(
            req_id,
            status,
            headers.as_ptr() as i32,
            headers.len() as i32,
            body.as_ptr() as i32,
            body.len() as i32,
        )
    }
    fn http_stop(server_id: i64) -> i32 {
        nexus_net_wasm::__nx_http_stop(server_id)
    }
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

impl proc::Guest for StdlibComponent {
    fn exit(status: i64) {
        nexus_proc_wasm::__nx_exit(status);
    }
    fn argv() -> String {
        unpack_string(nexus_proc_wasm::__nx_argv())
    }
    fn exec(cmd: String, args: String) -> i64 {
        nexus_proc_wasm::__nx_exec(
            cmd.as_ptr() as i32,
            cmd.len() as i32,
            args.as_ptr() as i32,
            args.len() as i32,
        )
    }
    fn exec_exit_code(id: i64) -> i64 {
        nexus_proc_wasm::__nx_exec_exit_code(id)
    }
    fn exec_stdout(id: i64) -> String {
        unpack_string(nexus_proc_wasm::__nx_exec_stdout(id))
    }
    fn exec_stderr(id: i64) -> String {
        unpack_string(nexus_proc_wasm::__nx_exec_stderr(id))
    }
    fn exec_free(id: i64) -> i32 {
        nexus_proc_wasm::__nx_exec_free(id)
    }
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

impl env::Guest for StdlibComponent {
    fn get_env(key: String) -> String {
        let packed = nexus_proc_wasm::__nx_get_env(key.as_ptr() as i32, key.len() as i32);
        unpack_string(packed)
    }
    fn has_env(key: String) -> i32 {
        nexus_proc_wasm::__nx_has_env(key.as_ptr() as i32, key.len() as i32)
    }
    fn set_env(key: String, value: String) {
        nexus_proc_wasm::__nx_set_env(
            key.as_ptr() as i32,
            key.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        );
    }
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

impl clock::Guest for StdlibComponent {
    fn sleep(ms: i64) {
        nexus_clock_wasm::__nx_sleep(ms);
    }
    fn now() -> i64 {
        nexus_clock_wasm::__nx_now()
    }
}

// ---------------------------------------------------------------------------
// Random
// ---------------------------------------------------------------------------

impl rand::Guest for StdlibComponent {
    fn random_i64() -> i64 {
        nexus_random_wasm::__nx_random_i64()
    }
    fn random_range(min: i64, max: i64) -> i64 {
        nexus_random_wasm::__nx_random_range(min, max)
    }
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

impl collections::Guest for StdlibComponent {
    fn array_length(ptr: i32, len: i32) -> i64 {
        nexus_core_wasm::core::__nx_array_length(ptr, len)
    }
    fn hmap_new() -> i64 {
        nexus_collection_wasm::__nx_hmap_new()
    }
    fn hmap_put(id: i64, key: i64, value: i64) -> i64 {
        nexus_collection_wasm::__nx_hmap_put(id, key, value)
    }
    fn hmap_get(id: i64, key: i64, default_val: i64) -> i64 {
        nexus_collection_wasm::__nx_hmap_get(id, key, default_val)
    }
    fn hmap_has(id: i64, key: i64) -> i32 {
        nexus_collection_wasm::__nx_hmap_has(id, key)
    }
    fn hmap_del(id: i64, key: i64) -> i32 {
        nexus_collection_wasm::__nx_hmap_del(id, key)
    }
    fn hmap_size(id: i64) -> i64 {
        nexus_collection_wasm::__nx_hmap_size(id)
    }
    fn hmap_keys(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_hmap_keys(id))
    }
    fn hmap_vals(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_hmap_vals(id))
    }
    fn hmap_key_at(id: i64, idx: i64) -> i64 {
        nexus_collection_wasm::__nx_hmap_key_at(id, idx)
    }
    fn hmap_val_at(id: i64, idx: i64) -> i64 {
        nexus_collection_wasm::__nx_hmap_val_at(id, idx)
    }
    fn hmap_free(id: i64) -> i32 {
        nexus_collection_wasm::__nx_hmap_free(id)
    }
    fn hset_new() -> i64 {
        nexus_collection_wasm::__nx_hset_new()
    }
    fn hset_insert(id: i64, val: i64) -> i32 {
        nexus_collection_wasm::__nx_hset_insert(id, val)
    }
    fn hset_contains(id: i64, val: i64) -> i32 {
        nexus_collection_wasm::__nx_hset_contains(id, val)
    }
    fn hset_remove(id: i64, val: i64) -> i32 {
        nexus_collection_wasm::__nx_hset_remove(id, val)
    }
    fn hset_size(id: i64) -> i64 {
        nexus_collection_wasm::__nx_hset_size(id)
    }
    fn hset_to_list(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_hset_to_list(id))
    }
    fn hset_union(id_a: i64, id_b: i64) -> i64 {
        nexus_collection_wasm::__nx_hset_union(id_a, id_b)
    }
    fn hset_intersection(id_a: i64, id_b: i64) -> i64 {
        nexus_collection_wasm::__nx_hset_intersection(id_a, id_b)
    }
    fn hset_difference(id_a: i64, id_b: i64) -> i64 {
        nexus_collection_wasm::__nx_hset_difference(id_a, id_b)
    }
    fn hset_free(id: i64) -> i32 {
        nexus_collection_wasm::__nx_hset_free(id)
    }
    fn smap_new() -> i64 {
        nexus_collection_wasm::__nx_smap_new()
    }
    fn smap_put(id: i64, key: String, value: i64) -> i64 {
        nexus_collection_wasm::__nx_smap_put(id, key.as_ptr() as i32, key.len() as i32, value)
    }
    fn smap_get(id: i64, key: String, default_val: i64) -> i64 {
        nexus_collection_wasm::__nx_smap_get(id, key.as_ptr() as i32, key.len() as i32, default_val)
    }
    fn smap_has(id: i64, key: String) -> i32 {
        nexus_collection_wasm::__nx_smap_has(id, key.as_ptr() as i32, key.len() as i32)
    }
    fn smap_del(id: i64, key: String) -> i32 {
        nexus_collection_wasm::__nx_smap_del(id, key.as_ptr() as i32, key.len() as i32)
    }
    fn smap_size(id: i64) -> i64 {
        nexus_collection_wasm::__nx_smap_size(id)
    }
    fn smap_keys(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_smap_keys(id))
    }
    fn smap_vals(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_smap_vals(id))
    }
    fn smap_val_count(id: i64) -> i64 {
        nexus_collection_wasm::__nx_smap_val_count(id)
    }
    fn smap_val_at(id: i64, idx: i64) -> i64 {
        nexus_collection_wasm::__nx_smap_val_at(id, idx)
    }
    fn smap_free(id: i64) -> i32 {
        nexus_collection_wasm::__nx_smap_free(id)
    }
}

// ---------------------------------------------------------------------------
// ByteBuffer
// ---------------------------------------------------------------------------

impl bytebuffer::Guest for StdlibComponent {
    fn buf_new() -> i64 {
        nexus_collection_wasm::__nx_buf_new()
    }
    fn buf_push_byte(id: i64, byte: i64) {
        nexus_collection_wasm::__nx_buf_push_byte(id, byte);
    }
    fn buf_push_i32_le(id: i64, val: i64) {
        nexus_collection_wasm::__nx_buf_push_i32_le(id, val);
    }
    fn buf_push_i64_le(id: i64, val: i64) {
        nexus_collection_wasm::__nx_buf_push_i64_le(id, val);
    }
    fn buf_push_f64_str_le(id: i64, s: String) {
        nexus_collection_wasm::__nx_buf_push_f64_str_le(id, s.as_ptr() as i32, s.len() as i32);
    }
    fn buf_push_leb128_u(id: i64, val: i64) {
        nexus_collection_wasm::__nx_buf_push_leb128_u(id, val);
    }
    fn buf_push_leb128_s(id: i64, val: i64) {
        nexus_collection_wasm::__nx_buf_push_leb128_s(id, val);
    }
    fn buf_push_string(id: i64, s: String) {
        nexus_collection_wasm::__nx_buf_push_string(id, s.as_ptr() as i32, s.len() as i32);
    }
    fn buf_push_buf(id: i64, src_id: i64) {
        nexus_collection_wasm::__nx_buf_push_buf(id, src_id);
    }
    fn buf_length(id: i64) -> i64 {
        nexus_collection_wasm::__nx_buf_length(id)
    }
    fn buf_get_byte(id: i64, idx: i64) -> i64 {
        nexus_collection_wasm::__nx_buf_get_byte(id, idx)
    }
    fn buf_to_string(id: i64) -> String {
        unpack_string(nexus_collection_wasm::__nx_buf_to_string(id))
    }
    fn buf_write_file(id: i64, path: String) -> i32 {
        nexus_collection_wasm::__nx_buf_write_file(id, path.as_ptr() as i32, path.len() as i32)
    }
    fn buf_free(id: i64) -> i32 {
        nexus_collection_wasm::__nx_buf_free(id)
    }
    fn buf_read_file(path: String) -> i64 {
        nexus_collection_wasm::__nx_buf_read_file(path.as_ptr() as i32, path.len() as i32)
    }
    fn buf_read_stdin(n: i64) -> i64 {
        nexus_collection_wasm::__nx_read_bytes(n)
    }
    fn buf_copy_range(dst_id: i64, src_id: i64, start: i64, end_pos: i64) {
        nexus_collection_wasm::__nx_buf_copy_range(dst_id, src_id, start, end_pos);
    }
    fn buf_truncate(id: i64, len: i64) {
        nexus_collection_wasm::__nx_buf_truncate(id, len);
    }
}

// ---------------------------------------------------------------------------
// Core
// ---------------------------------------------------------------------------

impl nx_core::Guest for StdlibComponent {
    fn array_length(ptr: i32, len: i32) -> i64 {
        nexus_core_wasm::core::__nx_array_length(ptr, len)
    }
}

// ---------------------------------------------------------------------------
// Export registration
// ---------------------------------------------------------------------------

bindings::export!(StdlibComponent with_types_in bindings);
