use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Read;

thread_local! {
    static MAPS: RefCell<HashMap<i64, HashMap<i64, i64>>> = RefCell::new(HashMap::new());
    static SETS: RefCell<HashMap<i64, HashSet<i64>>> = RefCell::new(HashMap::new());
    static SMAPS: RefCell<HashMap<i64, HashMap<String, i64>>> = RefCell::new(HashMap::new());
    static BUFS: RefCell<HashMap<i64, Vec<u8>>> = RefCell::new(HashMap::new());
    static NEXT_MAP_ID: Cell<i64> = Cell::new(1);
    static NEXT_SET_ID: Cell<i64> = Cell::new(1);
    static NEXT_SMAP_ID: Cell<i64> = Cell::new(1);
    static NEXT_BUF_ID: Cell<i64> = Cell::new(1);

    // Index-order caches for O(1) random access by index.
    // Lazily populated on first __nx_*_at call after any mutation; invalidated
    // (entry removed) on put/del/free. Avoids the O(idx) cost of HashMap::iter().nth(idx).
    static MAP_KV_CACHE: RefCell<HashMap<i64, Vec<(i64, i64)>>> = RefCell::new(HashMap::new());
    static SMAP_V_CACHE: RefCell<HashMap<i64, Vec<i64>>> = RefCell::new(HashMap::new());
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

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_mark() -> i32 {
    nexus_wasm_alloc::mark()
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_reset(mark: i32) {
    nexus_wasm_alloc::reset_to(mark);
}

// ── HashMap ─────────────────────────────────────────────────────────

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_new() -> i64 {
    NEXT_MAP_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        MAPS.with(|maps| maps.borrow_mut().insert(id, HashMap::new()));
        id
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_put(id: i64, key: i64, value: i64) -> i64 {
    MAPS.with(|maps| {
        if let Some(map) = maps.borrow_mut().get_mut(&id) {
            map.insert(key, value);
        }
    });
    MAP_KV_CACHE.with(|c| c.borrow_mut().remove(&id));
    0
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_get(id: i64, key: i64, default_val: i64) -> i64 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(default_val)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_has(id: i64, key: i64) -> i32 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.contains_key(&key))
            .unwrap_or(false)
    }) as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_del(id: i64, key: i64) -> i32 {
    let removed = MAPS.with(|maps| {
        maps.borrow_mut()
            .get_mut(&id)
            .map(|map| map.remove(&key).is_some())
            .unwrap_or(false)
    });
    if removed {
        MAP_KV_CACHE.with(|c| c.borrow_mut().remove(&id));
    }
    removed as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_size(id: i64) -> i64 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.len() as i64)
            .unwrap_or(0)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_keys(id: i64) -> i64 {
    MAPS.with(|maps| {
        let result = maps
            .borrow()
            .get(&id)
            .map(|map| {
                map.keys()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(result)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_vals(id: i64) -> i64 {
    MAPS.with(|maps| {
        let result = maps
            .borrow()
            .get(&id)
            .map(|map| {
                map.values()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(result)
    })
}

/// Build (or refresh) the (key, value) snapshot for a HashMap and return the
/// requested index in O(1). Subsequent `_at` calls hit the cache directly.
fn hmap_kv_at(id: i64, idx: i64) -> Option<(i64, i64)> {
    if idx < 0 {
        return None;
    }
    MAP_KV_CACHE.with(|cache| {
        let mut cache_mut = cache.borrow_mut();
        if !cache_mut.contains_key(&id) {
            let snapshot: Vec<(i64, i64)> = MAPS.with(|maps| {
                maps.borrow()
                    .get(&id)
                    .map(|map| map.iter().map(|(&k, &v)| (k, v)).collect())
                    .unwrap_or_default()
            });
            cache_mut.insert(id, snapshot);
        }
        cache_mut
            .get(&id)
            .and_then(|v| v.get(idx as usize).copied())
    })
}

/// Returns the key at the given index (iteration order). Returns 0 if out of bounds.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_key_at(id: i64, idx: i64) -> i64 {
    hmap_kv_at(id, idx).map(|(k, _)| k).unwrap_or(0)
}

/// Returns the value at the given index (iteration order). Returns 0 if out of bounds.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_val_at(id: i64, idx: i64) -> i64 {
    hmap_kv_at(id, idx).map(|(_, v)| v).unwrap_or(0)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hmap_free(id: i64) -> i32 {
    MAP_KV_CACHE.with(|c| c.borrow_mut().remove(&id));
    MAPS.with(|maps| maps.borrow_mut().remove(&id).is_some()) as i32
}

// ── HashSet ─────────────────────────────────────────────────────────

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_new() -> i64 {
    NEXT_SET_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        SETS.with(|sets| sets.borrow_mut().insert(id, HashSet::new()));
        id
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_insert(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow_mut()
            .get_mut(&id)
            .map(|set| set.insert(val))
            .unwrap_or(false)
    }) as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_contains(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow()
            .get(&id)
            .map(|set| set.contains(&val))
            .unwrap_or(false)
    }) as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_remove(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow_mut()
            .get_mut(&id)
            .map(|set| set.remove(&val))
            .unwrap_or(false)
    }) as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_size(id: i64) -> i64 {
    SETS.with(|sets| {
        sets.borrow()
            .get(&id)
            .map(|set| set.len() as i64)
            .unwrap_or(0)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_to_list(id: i64) -> i64 {
    SETS.with(|sets| {
        let result = sets
            .borrow()
            .get(&id)
            .map(|set| {
                set.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(result)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_union(id_a: i64, id_b: i64) -> i64 {
    SETS.with(|sets| {
        NEXT_SET_ID.with(|next| {
            let new_id = next.get();
            next.set(new_id + 1);
            let borrowed = sets.borrow();
            let empty = HashSet::new();
            let a = borrowed.get(&id_a).unwrap_or(&empty);
            let b = borrowed.get(&id_b).unwrap_or(&empty);
            let result: HashSet<i64> = a.union(b).copied().collect();
            drop(borrowed);
            sets.borrow_mut().insert(new_id, result);
            new_id
        })
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_intersection(id_a: i64, id_b: i64) -> i64 {
    SETS.with(|sets| {
        NEXT_SET_ID.with(|next| {
            let new_id = next.get();
            next.set(new_id + 1);
            let borrowed = sets.borrow();
            let empty = HashSet::new();
            let a = borrowed.get(&id_a).unwrap_or(&empty);
            let b = borrowed.get(&id_b).unwrap_or(&empty);
            let result: HashSet<i64> = a.intersection(b).copied().collect();
            drop(borrowed);
            sets.borrow_mut().insert(new_id, result);
            new_id
        })
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_difference(id_a: i64, id_b: i64) -> i64 {
    SETS.with(|sets| {
        NEXT_SET_ID.with(|next| {
            let new_id = next.get();
            next.set(new_id + 1);
            let borrowed = sets.borrow();
            let empty = HashSet::new();
            let a = borrowed.get(&id_a).unwrap_or(&empty);
            let b = borrowed.get(&id_b).unwrap_or(&empty);
            let result: HashSet<i64> = a.difference(b).copied().collect();
            drop(borrowed);
            sets.borrow_mut().insert(new_id, result);
            new_id
        })
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_hset_free(id: i64) -> i32 {
    SETS.with(|sets| sets.borrow_mut().remove(&id).is_some()) as i32
}

// ── StringMap (String → i64) ────────────────────────────────────────

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_new() -> i64 {
    NEXT_SMAP_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        SMAPS.with(|maps| maps.borrow_mut().insert(id, HashMap::new()));
        id
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_put(id: i64, key_ptr: i32, key_len: i32, value: i64) -> i64 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        if let Some(map) = maps.borrow_mut().get_mut(&id) {
            map.insert(key, value);
        }
    });
    SMAP_V_CACHE.with(|c| c.borrow_mut().remove(&id));
    0
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_get(id: i64, key_ptr: i32, key_len: i32, default_val: i64) -> i64 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(default_val)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_has(id: i64, key_ptr: i32, key_len: i32) -> i32 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.contains_key(&key))
            .unwrap_or(false)
    }) as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_del(id: i64, key_ptr: i32, key_len: i32) -> i32 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    let removed = SMAPS.with(|maps| {
        maps.borrow_mut()
            .get_mut(&id)
            .map(|map| map.remove(&key).is_some())
            .unwrap_or(false)
    });
    if removed {
        SMAP_V_CACHE.with(|c| c.borrow_mut().remove(&id));
    }
    removed as i32
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_size(id: i64) -> i64 {
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.len() as i64)
            .unwrap_or(0)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_keys(id: i64) -> i64 {
    SMAPS.with(|maps| {
        let result = maps
            .borrow()
            .get(&id)
            .map(|map| map.keys().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(result)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_vals(id: i64) -> i64 {
    SMAPS.with(|maps| {
        let result = maps
            .borrow()
            .get(&id)
            .map(|map| {
                map.values()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(result)
    })
}

/// Returns the number of values (== size). Used for index-based iteration.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_val_count(id: i64) -> i64 {
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.len() as i64)
            .unwrap_or(0)
    })
}

/// Returns the value at the given index (iteration order). Returns 0 if out of bounds.
///
/// Builds (or refreshes) a side-cache of the value list on first access, so
/// repeated `_at` calls across an enumeration are O(1) per call rather than
/// O(idx) (the cost of HashMap::values().nth(idx)).
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_val_at(id: i64, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    SMAP_V_CACHE.with(|cache| {
        let mut cache_mut = cache.borrow_mut();
        if !cache_mut.contains_key(&id) {
            let snapshot: Vec<i64> = SMAPS.with(|maps| {
                maps.borrow()
                    .get(&id)
                    .map(|map| map.values().copied().collect())
                    .unwrap_or_default()
            });
            cache_mut.insert(id, snapshot);
        }
        cache_mut
            .get(&id)
            .and_then(|v| v.get(idx as usize).copied())
            .unwrap_or(0)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_smap_free(id: i64) -> i32 {
    SMAP_V_CACHE.with(|c| c.borrow_mut().remove(&id));
    SMAPS.with(|maps| maps.borrow_mut().remove(&id).is_some()) as i32
}

// ── ByteBuffer (Vec<u8>) ────────────────────────────────────────────

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_new() -> i64 {
    NEXT_BUF_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        BUFS.with(|bufs| bufs.borrow_mut().insert(id, Vec::new()));
        id
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_byte(id: i64, byte: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.push(byte as u8);
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_i32_le(id: i64, val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(&(val as i32).to_le_bytes());
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_i64_le(id: i64, val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_f64_str_le(id: i64, s_ptr: i32, s_len: i32) {
    let s = nexus_wasm_alloc::read_string(s_ptr, s_len);
    let val: f64 = s.trim().parse().unwrap_or(0.0);
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_leb128_u(id: i64, mut val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            let mut v = val as u64;
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    buf.push(byte);
                    break;
                } else {
                    buf.push(byte | 0x80);
                }
            }
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_leb128_s(id: i64, val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            let mut v = val;
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
                if done {
                    buf.push(byte);
                    break;
                } else {
                    buf.push(byte | 0x80);
                }
            }
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_string(id: i64, s_ptr: i32, s_len: i32) {
    let s = nexus_wasm_alloc::read_string(s_ptr, s_len);
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(s.as_bytes());
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_push_buf(dst_id: i64, src_id: i64) {
    BUFS.with(|bufs| {
        let borrowed = bufs.borrow();
        let src_data = borrowed.get(&src_id).cloned().unwrap_or_default();
        drop(borrowed);
        if let Some(dst) = bufs.borrow_mut().get_mut(&dst_id) {
            dst.extend_from_slice(&src_data);
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_length(id: i64) -> i64 {
    BUFS.with(|bufs| {
        bufs.borrow()
            .get(&id)
            .map(|buf| buf.len() as i64)
            .unwrap_or(0)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_get_byte(id: i64, idx: i64) -> i64 {
    BUFS.with(|bufs| {
        bufs.borrow()
            .get(&id)
            .and_then(|buf| buf.get(idx as usize).map(|&b| b as i64))
            .unwrap_or(-1)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_to_string(id: i64) -> i64 {
    BUFS.with(|bufs| {
        let s = bufs
            .borrow()
            .get(&id)
            .map(|buf| String::from_utf8_lossy(buf).to_string())
            .unwrap_or_default();
        nexus_wasm_alloc::store_string_result(s)
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_write_file(id: i64, path_ptr: i32, path_len: i32) -> i32 {
    let path = nexus_wasm_alloc::read_string(path_ptr, path_len);
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow().get(&id) {
            std::fs::write(&path, buf).is_ok() as i32
        } else {
            0
        }
    })
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_truncate(id: i64, len: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.truncate(len.max(0) as usize);
        }
    });
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_free(id: i64) -> i32 {
    BUFS.with(|bufs| bufs.borrow_mut().remove(&id).is_some()) as i32
}

/// Copy bytes [start..end_pos) from buffer src_id to buffer dst_id in one shot.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_copy_range(dst_id: i64, src_id: i64, start: i64, end_pos: i64) {
    BUFS.with(|bufs| {
        let borrowed = bufs.borrow();
        let src_slice = borrowed
            .get(&src_id)
            .map(|buf| {
                let s = start as usize;
                let e = (end_pos as usize).min(buf.len());
                if s < e {
                    buf[s..e].to_vec()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        drop(borrowed);
        if !src_slice.is_empty() {
            if let Some(dst) = bufs.borrow_mut().get_mut(&dst_id) {
                dst.extend_from_slice(&src_slice);
            }
        }
    });
}

/// Read a binary file into a new ByteBuffer. Returns buf_id or -1 on error.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_read_file(path_ptr: i32, path_len: i32) -> i64 {
    let path = nexus_wasm_alloc::read_string(path_ptr, path_len);
    match std::fs::read(&path) {
        Ok(data) => NEXT_BUF_ID.with(|next| {
            let id = next.get();
            next.set(id + 1);
            BUFS.with(|bufs| bufs.borrow_mut().insert(id, data));
            id
        }),
        Err(_) => -1,
    }
}

// ── Stdin → ByteBuffer ───────────────────────────────────────────────
// Logically a `stdio` op (exposed as `Console.read_bytes`), but the impl
// lives here because BUFS does. Returns the new buf id; on EOF before n
// bytes the buffer contains whatever was read — caller checks `length`.

fn read_into_new_buf<R: Read>(reader: &mut R, n: i64) -> i64 {
    let cap = if n <= 0 { 0 } else { n as usize };
    let mut data = vec![0u8; cap];
    let mut filled = 0usize;
    while filled < cap {
        match reader.read(&mut data[filled..]) {
            Ok(0) => break,
            Ok(k) => filled += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    data.truncate(filled);
    NEXT_BUF_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        BUFS.with(|bufs| bufs.borrow_mut().insert(id, data));
        id
    })
}

/// Read up to `n` bytes from stdin into a new ByteBuffer. Blocks until `n`
/// bytes are read or stdin closes; on EOF before `n`, returns a buffer
/// shorter than `n` (caller inspects `length`). Binary-clean: bytes are
/// preserved verbatim, no encoding interpretation.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_buf_read_stdin(n: i64) -> i64 {
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    read_into_new_buf(&mut handle, n)
}

#[cfg(test)]
mod read_bytes_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn full_read_returns_exact_bytes() {
        let payload: &[u8] = b"hello, world";
        let mut cur = Cursor::new(payload.to_vec());
        let id = read_into_new_buf(&mut cur, payload.len() as i64);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn eof_before_n_yields_short_buffer() {
        let payload: &[u8] = b"abc";
        let mut cur = Cursor::new(payload.to_vec());
        let id = read_into_new_buf(&mut cur, 16);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert_eq!(got, payload);
        assert!(got.len() < 16);
    }

    #[test]
    fn binary_clean_roundtrip_with_nul_and_high_bytes() {
        let payload: Vec<u8> = (0u8..=255).collect();
        let mut cur = Cursor::new(payload.clone());
        let id = read_into_new_buf(&mut cur, payload.len() as i64);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn multibyte_utf8_preserved_byte_for_byte() {
        let payload = "こんにちは🌍\nLSP body".as_bytes().to_vec();
        let mut cur = Cursor::new(payload.clone());
        let id = read_into_new_buf(&mut cur, payload.len() as i64);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn n_zero_returns_empty_buffer() {
        let mut cur = Cursor::new(b"unused".to_vec());
        let id = read_into_new_buf(&mut cur, 0);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn n_negative_returns_empty_buffer() {
        let mut cur = Cursor::new(b"unused".to_vec());
        let id = read_into_new_buf(&mut cur, -5);
        let got = BUFS.with(|b| b.borrow().get(&id).cloned()).unwrap();
        assert!(got.is_empty());
    }
}

#[cfg(test)]
mod index_cache_tests {
    use super::*;

    /// Iterating every index of a freshly-populated map must visit every
    /// (key, value) pair exactly once, regardless of HashMap iteration order.
    #[test]
    fn hmap_full_enumeration_via_index_at_visits_each_entry_once() {
        let id = __nx_hmap_new();
        let pairs: Vec<(i64, i64)> = (0..50).map(|i| (i, i * 1000 + 7)).collect();
        for &(k, v) in &pairs {
            __nx_hmap_put(id, k, v);
        }
        let n = __nx_hmap_size(id);
        let mut seen: Vec<(i64, i64)> = (0..n)
            .map(|i| (__nx_hmap_key_at(id, i), __nx_hmap_val_at(id, i)))
            .collect();
        seen.sort();
        let mut expected = pairs.clone();
        expected.sort();
        assert_eq!(seen, expected);
        __nx_hmap_free(id);
    }

    /// After put-invalidation the cache must rebuild — values must reflect the new entry.
    #[test]
    fn hmap_put_invalidates_index_cache() {
        let id = __nx_hmap_new();
        __nx_hmap_put(id, 1, 100);
        __nx_hmap_put(id, 2, 200);
        // Touch _at to populate the cache.
        let _ = __nx_hmap_val_at(id, 0);
        let _ = __nx_hmap_val_at(id, 1);
        // Mutate.
        __nx_hmap_put(id, 3, 300);
        let n = __nx_hmap_size(id);
        assert_eq!(n, 3);
        let mut vs: Vec<i64> = (0..n).map(|i| __nx_hmap_val_at(id, i)).collect();
        vs.sort();
        assert_eq!(vs, vec![100, 200, 300]);
        __nx_hmap_free(id);
    }

    /// After del-invalidation the cache must rebuild — index 0 must not return the deleted value.
    #[test]
    fn hmap_del_invalidates_index_cache() {
        let id = __nx_hmap_new();
        __nx_hmap_put(id, 1, 100);
        __nx_hmap_put(id, 2, 200);
        __nx_hmap_put(id, 3, 300);
        // Populate.
        for i in 0..3 {
            let _ = __nx_hmap_val_at(id, i);
        }
        // Remove key=2.
        let removed = __nx_hmap_del(id, 2);
        assert_eq!(removed, 1);
        let n = __nx_hmap_size(id);
        assert_eq!(n, 2);
        let mut vs: Vec<i64> = (0..n).map(|i| __nx_hmap_val_at(id, i)).collect();
        vs.sort();
        assert_eq!(vs, vec![100, 300]);
        __nx_hmap_free(id);
    }

    /// Out-of-bounds and negative indices return 0 without panicking.
    #[test]
    fn hmap_at_out_of_bounds_returns_zero() {
        let id = __nx_hmap_new();
        __nx_hmap_put(id, 42, 99);
        assert_eq!(__nx_hmap_val_at(id, -1), 0);
        assert_eq!(__nx_hmap_val_at(id, 1), 0);
        assert_eq!(__nx_hmap_val_at(id, 999_999), 0);
        __nx_hmap_free(id);
    }

    /// SMAP equivalent: enumeration via index must visit every value exactly once.
    #[test]
    fn smap_full_enumeration_via_index_at_visits_each_value_once() {
        let id = __nx_smap_new();
        let pairs: Vec<(String, i64)> =
            (0..30).map(|i| (format!("k{}", i), i * 11)).collect();
        for (k, v) in &pairs {
            __nx_smap_put(id, k.as_ptr() as i32, k.len() as i32, *v);
        }
        let n = __nx_smap_val_count(id);
        let mut seen: Vec<i64> = (0..n).map(|i| __nx_smap_val_at(id, i)).collect();
        seen.sort();
        let mut expected: Vec<i64> = pairs.iter().map(|(_, v)| *v).collect();
        expected.sort();
        assert_eq!(seen, expected);
        __nx_smap_free(id);
    }

    /// After put on smap the cache must reflect the new value.
    #[test]
    fn smap_put_invalidates_index_cache() {
        let id = __nx_smap_new();
        let k1 = "a".to_string();
        let k2 = "b".to_string();
        __nx_smap_put(id, k1.as_ptr() as i32, k1.len() as i32, 1);
        __nx_smap_put(id, k2.as_ptr() as i32, k2.len() as i32, 2);
        // Populate cache.
        let _ = __nx_smap_val_at(id, 0);
        let _ = __nx_smap_val_at(id, 1);
        // Insert new key.
        let k3 = "c".to_string();
        __nx_smap_put(id, k3.as_ptr() as i32, k3.len() as i32, 3);
        let n = __nx_smap_val_count(id);
        assert_eq!(n, 3);
        let mut vs: Vec<i64> = (0..n).map(|i| __nx_smap_val_at(id, i)).collect();
        vs.sort();
        assert_eq!(vs, vec![1, 2, 3]);
        __nx_smap_free(id);
    }
}

