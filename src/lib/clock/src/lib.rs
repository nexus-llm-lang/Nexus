use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_sleep(ms: i64) {
    if ms > 0 {
        thread::sleep(Duration::from_millis(ms as u64));
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
