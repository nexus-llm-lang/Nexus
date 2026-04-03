use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static RNG_STATE: AtomicU64 = AtomicU64::new(0);

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_random_i64() -> i64 {
    (next_random_u64() & 0x7fff_ffff_ffff_ffff) as i64
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_random_range(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    let Some(span_i64) = max.checked_sub(min) else {
        return min;
    };
    if span_i64 <= 0 {
        return min;
    }
    let span = span_i64 as u64;
    let n = next_random_u64() % span;
    min + n as i64
}

fn next_random_u64() -> u64 {
    let mut current = RNG_STATE.load(Ordering::Relaxed);
    if current == 0 {
        let seed = initial_seed();
        match RNG_STATE.compare_exchange(0, seed, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => current = seed,
            Err(actual) => current = actual,
        }
    }

    loop {
        let next = current
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match RNG_STATE.compare_exchange(current, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

fn initial_seed() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mixed = now ^ 0xA409_3822_299F_31D0;
    if mixed == 0 {
        0x1319_8A2E_0370_7344
    } else {
        mixed
    }
}

