use std::alloc::{alloc as raw_alloc, dealloc as raw_dealloc, Layout};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Allocate-time and free-time layouts must match exactly for
/// `std::alloc::dealloc` — host allocators that over-allocate (e.g.
/// macOS system_malloc) abort if the dealloc layout differs from the
/// alloc layout.
fn byte_layout(size: usize) -> Option<Layout> {
    Layout::array::<u8>(size).ok()
}

#[derive(Default)]
struct AllocState {
    sizes: BTreeMap<i32, usize>,
    order: Vec<i32>,
}

static ALLOCATIONS: OnceLock<Mutex<AllocState>> = OnceLock::new();

fn allocations() -> &'static Mutex<AllocState> {
    ALLOCATIONS.get_or_init(|| Mutex::new(AllocState::default()))
}

pub fn remember_allocation(ptr: i32, size: usize) {
    if ptr == 0 || size == 0 {
        return;
    }
    if let Ok(mut state) = allocations().lock() {
        if state.sizes.insert(ptr, size).is_none() {
            state.order.push(ptr);
        }
    }
}

pub fn take_allocation(ptr: i32, size: usize) -> bool {
    let Ok(mut state) = allocations().lock() else {
        report_failure(
            "take_allocation",
            &format!("allocations mutex poisoned: ptr=0x{ptr:08x} size={size}"),
        );
        return false;
    };
    match state.sizes.get(&ptr).copied() {
        Some(expected) if expected == size => {
            state.sizes.remove(&ptr);
            if let Some(pos) = state.order.iter().rposition(|&p| p == ptr) {
                state.order.remove(pos);
            }
            true
        }
        Some(expected) => {
            drop(state);
            report_misuse("take_allocation", ptr, Some(expected), size);
            false
        }
        None => {
            drop(state);
            report_misuse("take_allocation", ptr, None, size);
            false
        }
    }
}

/// Counter incremented every time `wasm_alloc` detects caller misuse
/// (size mismatch / unknown ptr in `take_allocation`) or an internal
/// failure (e.g. layout error, OOM). Test-only observability — production
/// code should not gate logic on this value.
static FAILURE_EVENTS: AtomicU64 = AtomicU64::new(0);

pub fn observed_failure_count() -> u64 {
    FAILURE_EVENTS.load(Ordering::Relaxed)
}

/// Caller-bug observation: panics in debug builds (caller passed wrong
/// size or an untracked pointer — almost certainly a programmer error)
/// and emits a `eprintln` + counter bump in release builds so the misuse
/// is at least visible in logs instead of silently leaking memory.
pub fn report_misuse(context: &str, ptr: i32, expected: Option<usize>, actual: usize) {
    FAILURE_EVENTS.fetch_add(1, Ordering::Relaxed);
    let detail = match expected {
        Some(exp) => format!(
            "ptr=0x{ptr:08x} size mismatch (tracked={exp}, caller-passed={actual})"
        ),
        None => format!(
            "ptr=0x{ptr:08x} not tracked (caller-passed size={actual})"
        ),
    };
    eprintln!("wasm_alloc misuse: {context}: {detail}");
    debug_assert!(false, "wasm_alloc misuse: {context}: {detail}");
}

/// System-failure observation (OOM, layout error). Never traps — these
/// are not caller bugs. Counter + eprintln so the failure is visible.
pub fn report_failure(context: &str, detail: &str) {
    FAILURE_EVENTS.fetch_add(1, Ordering::Relaxed);
    eprintln!("wasm_alloc failure: {context}: {detail}");
}

/// Snapshot the count of currently-tracked allocations. Pair with [`reset_to`]
/// to bulk-free everything allocated after the mark.
pub fn mark() -> i32 {
    let Ok(state) = allocations().lock() else {
        return 0;
    };
    state.order.len() as i32
}

/// Free every tracked allocation past `mark`, in LIFO order. Pointers freed
/// here become invalid; callers must hold no live references into them.
pub fn reset_to(mark: i32) {
    let mark = mark.max(0) as usize;
    loop {
        let Ok(mut state) = allocations().lock() else {
            return;
        };
        if state.order.len() <= mark {
            return;
        }
        let Some(ptr) = state.order.pop() else {
            return;
        };
        let Some(size) = state.sizes.remove(&ptr) else {
            continue;
        };
        drop(state);
        let Ok(size_i32) = i32::try_from(size) else {
            continue;
        };
        unsafe { drop_raw(ptr, size_i32) };
    }
}

/// Number of allocations currently outstanding. Test-only observability —
/// production code should not rely on this.
pub fn outstanding() -> i32 {
    let Ok(state) = allocations().lock() else {
        return 0;
    };
    state.order.len() as i32
}

/// # Safety
/// Caller must pass a pointer and size previously returned by [`allocate`]
/// whose bookkeeping was already removed from `ALLOCATIONS`.
unsafe fn drop_raw(ptr: i32, size: i32) {
    let Some((offset, size)) = checked_ptr_len(ptr, size) else {
        return;
    };
    let Some(layout) = byte_layout(size) else {
        return;
    };
    raw_dealloc(offset as *mut u8, layout);
}

pub fn memory_end_is_valid(end: usize) -> bool {
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

pub fn checked_ptr_len(ptr: i32, len: i32) -> Option<(usize, usize)> {
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

pub fn checked_mut_ptr(ptr: i32, len: usize) -> Option<*mut u8> {
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

pub fn read_string(ptr: i32, len: i32) -> String {
    let Some((offset, len)) = checked_ptr_len(ptr, len) else {
        return String::new();
    };
    let bytes = unsafe { std::slice::from_raw_parts(offset as *const u8, len) };
    String::from_utf8_lossy(bytes).to_string()
}

pub fn pack_ptr_len(ptr: i32, len: i32) -> i64 {
    (((ptr as u32 as u64) << 32) | (len as u32 as u64)) as i64
}

pub fn store_string_result(s: String) -> i64 {
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
        // SAFETY: `ptr` was returned by `allocate(len_i32)` above.
        unsafe { deallocate(ptr, len_i32) };
        return 0;
    };

    // SAFETY: `dst` points to an allocated region of `len` bytes and source is valid for `len`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
    }

    pack_ptr_len(ptr, len_i32)
}

pub fn allocate(size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let size = size as usize;
    let Some(layout) = byte_layout(size) else {
        return 0;
    };
    // SAFETY: layout has nonzero size — `size > 0` guard above.
    let raw = unsafe { raw_alloc(layout) };
    if raw.is_null() {
        return 0;
    }
    let ptr = raw as usize as i32;
    remember_allocation(ptr, size);
    ptr
}

/// # Safety
/// Caller must pass a pointer and size previously returned by [`allocate`].
pub unsafe fn deallocate(ptr: i32, size: i32) {
    let Some((offset, size)) = checked_ptr_len(ptr, size) else {
        return;
    };
    if !take_allocation(ptr, size) {
        return;
    }
    let Some(layout) = byte_layout(size) else {
        return;
    };
    raw_dealloc(offset as *mut u8, layout);
}

// Bookkeeping-only tests exercise the mark/reset/deallocate state machine
// without touching `allocate` / `raw_dealloc` (which require wasm32 pointer
// width — the host process truncates 64-bit pointers when stuffing them into
// `i32` and aborts in dealloc). For end-to-end coverage that DOES exercise
// the allocator, see the wasm-integration regression test
// `bootstrap/tests/runtime/strings.rs::heap_reset_reclaims_string_allocations`.
#[cfg(test)]
mod bookkeeping_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static SERIAL: StdMutex<()> = StdMutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn mark_reset_pops_in_lifo_order() {
        let _g = lock();
        let m = mark();
        remember_allocation(0x1000, 64);
        remember_allocation(0x2000, 128);
        remember_allocation(0x3000, 256);
        assert_eq!(outstanding(), m + 3);
        reset_to_bookkeeping(m);
        assert_eq!(outstanding(), m);
    }

    #[test]
    fn take_allocation_removes_from_order() {
        let _g = lock();
        let m = mark();
        remember_allocation(0x4000, 48);
        assert!(take_allocation(0x4000, 48));
        reset_to_bookkeeping(m);
        assert_eq!(outstanding(), m);
    }

    #[test]
    fn take_allocation_size_mismatch_reports_misuse() {
        let _g = lock();
        remember_allocation(0x8000, 64);
        let before = observed_failure_count();
        assert!(!take_allocation(0x8000, 32));
        assert!(observed_failure_count() > before);
        // Bookkeeping for 0x8000 is untouched on mismatch — clean up with
        // the correct size so the entry doesn't leak into other tests.
        assert!(take_allocation(0x8000, 64));
    }

    #[test]
    fn take_allocation_unknown_ptr_reports_misuse() {
        let _g = lock();
        let before = observed_failure_count();
        assert!(!take_allocation(0x9000, 16));
        assert!(observed_failure_count() > before);
    }

    /// `deallocate` is the public misuse surface — verify the mismatched-size
    /// path goes through `take_allocation` and bumps the counter without
    /// invoking `raw_dealloc` on the (synthetic) pointer.
    #[test]
    fn deallocate_size_mismatch_reports_misuse() {
        let _g = lock();
        remember_allocation(0xC000, 64);
        let before = observed_failure_count();
        unsafe { deallocate(0xC000, 32) };
        assert!(observed_failure_count() > before);
        assert_eq!(outstanding_for(0xC000), Some(64));
        assert!(take_allocation(0xC000, 64));
    }

    fn outstanding_for(ptr: i32) -> Option<usize> {
        allocations().lock().ok()?.sizes.get(&ptr).copied()
    }

    #[test]
    fn reset_keeps_pre_mark_entries() {
        let _g = lock();
        let pre = outstanding();
        remember_allocation(0x5000, 16);
        let m = mark();
        remember_allocation(0x6000, 32);
        remember_allocation(0x7000, 64);
        assert_eq!(outstanding(), m + 2);
        reset_to_bookkeeping(m);
        assert_eq!(outstanding(), pre + 1);
        // Manual cleanup — these synthetic pointers must not leak into other
        // tests' baselines.
        assert!(take_allocation(0x5000, 16));
    }

    /// Variant of `reset_to` that walks the bookkeeping vec without invoking
    /// the host allocator. The synthetic pointers used in these tests are not
    /// real heap pointers, so calling `raw_dealloc` on them would abort.
    fn reset_to_bookkeeping(mark_value: i32) {
        let mark = mark_value.max(0) as usize;
        let Ok(mut state) = allocations().lock() else {
            return;
        };
        while state.order.len() > mark {
            if let Some(ptr) = state.order.pop() {
                state.sizes.remove(&ptr);
            }
        }
    }
}
