use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use nexus_wasm_alloc::{checked_ptr_len, store_string_result};

struct ExecResult {
    exit_code: i64,
    stdout: String,
    stderr: String,
}

thread_local! {
    static EXEC_RESULTS: RefCell<HashMap<i64, ExecResult>> = RefCell::new(HashMap::new());
    static NEXT_EXEC_ID: Cell<i64> = Cell::new(1);
}

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn __nx_exit(status: i64) {
    std::process::exit(status as i32);
}

#[no_mangle]
pub extern "C" fn __nx_get_env(key_ptr: i32, key_len: i32) -> i64 {
    let Some((offset, len)) = checked_ptr_len(key_ptr, key_len) else {
        return 0;
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    let key = String::from_utf8_lossy(bytes);
    match std::env::var(key.as_ref()) {
        Ok(val) => store_string_result(val),
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn __nx_set_env(
    key_ptr: i32,
    key_len: i32,
    value_ptr: i32,
    value_len: i32,
) {
    let Some((k_offset, k_len)) = checked_ptr_len(key_ptr, key_len) else {
        return;
    };
    let key_bytes = unsafe { std::slice::from_raw_parts(k_offset as *const u8, k_len) };
    let key = String::from_utf8_lossy(key_bytes);

    let Some((v_offset, v_len)) = checked_ptr_len(value_ptr, value_len) else {
        return;
    };
    let val_bytes = unsafe { std::slice::from_raw_parts(v_offset as *const u8, v_len) };
    let val = String::from_utf8_lossy(val_bytes);

    unsafe { std::env::set_var(key.as_ref(), val.as_ref()) };
}

#[no_mangle]
pub extern "C" fn __nx_argv() -> i64 {
    let args: Vec<String> = std::env::args().collect();
    let joined = args.join("\n");
    store_string_result(joined)
}

// ── Subprocess execution ────────────────────────────────────────────

/// Executes a command with newline-separated args. Returns a handle ID.
#[no_mangle]
pub extern "C" fn __nx_exec(cmd_ptr: i32, cmd_len: i32, args_ptr: i32, args_len: i32) -> i64 {
    let Some((c_off, c_len)) = checked_ptr_len(cmd_ptr, cmd_len) else {
        return store_exec_result(-1, String::new(), "invalid command pointer".into());
    };
    let cmd_bytes = unsafe { std::slice::from_raw_parts(c_off as *const u8, c_len) };
    let cmd = String::from_utf8_lossy(cmd_bytes);

    let args: Vec<String> = if args_len > 0 {
        let Some((a_off, a_len)) = checked_ptr_len(args_ptr, args_len) else {
            return store_exec_result(-1, String::new(), "invalid args pointer".into());
        };
        let args_bytes = unsafe { std::slice::from_raw_parts(a_off as *const u8, a_len) };
        let args_str = String::from_utf8_lossy(args_bytes);
        args_str.split('\n').map(|s| s.to_string()).collect()
    } else {
        vec![]
    };

    match std::process::Command::new(cmd.as_ref())
        .args(&args)
        .output()
    {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1) as i64;
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            store_exec_result(exit_code, stdout, stderr)
        }
        Err(e) => store_exec_result(-1, String::new(), e.to_string()),
    }
}

fn store_exec_result(exit_code: i64, stdout: String, stderr: String) -> i64 {
    let id = NEXT_EXEC_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    });
    EXEC_RESULTS.with(|r| {
        r.borrow_mut().insert(
            id,
            ExecResult {
                exit_code,
                stdout,
                stderr,
            },
        );
    });
    id
}

/// Returns the exit code of an exec result.
#[no_mangle]
pub extern "C" fn __nx_exec_exit_code(id: i64) -> i64 {
    EXEC_RESULTS.with(|r| r.borrow().get(&id).map_or(-1, |r| r.exit_code))
}

/// Returns the stdout of an exec result.
#[no_mangle]
pub extern "C" fn __nx_exec_stdout(id: i64) -> i64 {
    EXEC_RESULTS.with(|r| {
        r.borrow()
            .get(&id)
            .map_or(0, |r| store_string_result(r.stdout.clone()))
    })
}

/// Returns the stderr of an exec result.
#[no_mangle]
pub extern "C" fn __nx_exec_stderr(id: i64) -> i64 {
    EXEC_RESULTS.with(|r| {
        r.borrow()
            .get(&id)
            .map_or(0, |r| store_string_result(r.stderr.clone()))
    })
}

/// Frees an exec result handle.
#[no_mangle]
pub extern "C" fn __nx_exec_free(id: i64) -> i32 {
    EXEC_RESULTS.with(|r| if r.borrow_mut().remove(&id).is_some() { 1 } else { 0 })
}
