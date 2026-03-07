use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

thread_local! {
    static MAPS: RefCell<HashMap<i64, HashMap<i64, i64>>> = RefCell::new(HashMap::new());
    static SETS: RefCell<HashMap<i64, HashSet<i64>>> = RefCell::new(HashMap::new());
    static SMAPS: RefCell<HashMap<i64, HashMap<String, i64>>> = RefCell::new(HashMap::new());
    static BUFS: RefCell<HashMap<i64, Vec<u8>>> = RefCell::new(HashMap::new());
    static NEXT_MAP_ID: Cell<i64> = Cell::new(1);
    static NEXT_SET_ID: Cell<i64> = Cell::new(1);
    static NEXT_SMAP_ID: Cell<i64> = Cell::new(1);
    static NEXT_BUF_ID: Cell<i64> = Cell::new(1);
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

// ── HashMap ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __nx_hmap_new() -> i64 {
    NEXT_MAP_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        MAPS.with(|maps| maps.borrow_mut().insert(id, HashMap::new()));
        id
    })
}

#[no_mangle]
pub extern "C" fn __nx_hmap_put(id: i64, key: i64, value: i64) -> i64 {
    MAPS.with(|maps| {
        if let Some(map) = maps.borrow_mut().get_mut(&id) {
            map.insert(key, value);
        }
    });
    0
}

#[no_mangle]
pub extern "C" fn __nx_hmap_get(id: i64, key: i64, default_val: i64) -> i64 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(default_val)
    })
}

#[no_mangle]
pub extern "C" fn __nx_hmap_has(id: i64, key: i64) -> i32 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.contains_key(&key))
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_hmap_del(id: i64, key: i64) -> i32 {
    MAPS.with(|maps| {
        maps.borrow_mut()
            .get_mut(&id)
            .map(|map| map.remove(&key).is_some())
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_hmap_size(id: i64) -> i64 {
    MAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.len() as i64)
            .unwrap_or(0)
    })
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_hmap_free(id: i64) -> i32 {
    MAPS.with(|maps| maps.borrow_mut().remove(&id).is_some()) as i32
}

// ── HashSet ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __nx_hset_new() -> i64 {
    NEXT_SET_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        SETS.with(|sets| sets.borrow_mut().insert(id, HashSet::new()));
        id
    })
}

#[no_mangle]
pub extern "C" fn __nx_hset_insert(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow_mut()
            .get_mut(&id)
            .map(|set| set.insert(val))
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_hset_contains(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow()
            .get(&id)
            .map(|set| set.contains(&val))
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_hset_remove(id: i64, val: i64) -> i32 {
    SETS.with(|sets| {
        sets.borrow_mut()
            .get_mut(&id)
            .map(|set| set.remove(&val))
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_hset_size(id: i64) -> i64 {
    SETS.with(|sets| {
        sets.borrow()
            .get(&id)
            .map(|set| set.len() as i64)
            .unwrap_or(0)
    })
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_hset_free(id: i64) -> i32 {
    SETS.with(|sets| sets.borrow_mut().remove(&id).is_some()) as i32
}

// ── StringMap (String → i64) ────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __nx_smap_new() -> i64 {
    NEXT_SMAP_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        SMAPS.with(|maps| maps.borrow_mut().insert(id, HashMap::new()));
        id
    })
}

#[no_mangle]
pub extern "C" fn __nx_smap_put(id: i64, key_ptr: i32, key_len: i32, value: i64) -> i64 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        if let Some(map) = maps.borrow_mut().get_mut(&id) {
            map.insert(key, value);
        }
    });
    0
}

#[no_mangle]
pub extern "C" fn __nx_smap_get(id: i64, key_ptr: i32, key_len: i32, default_val: i64) -> i64 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(default_val)
    })
}

#[no_mangle]
pub extern "C" fn __nx_smap_has(id: i64, key_ptr: i32, key_len: i32) -> i32 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.contains_key(&key))
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_smap_del(id: i64, key_ptr: i32, key_len: i32) -> i32 {
    let key = nexus_wasm_alloc::read_string(key_ptr, key_len);
    SMAPS.with(|maps| {
        maps.borrow_mut()
            .get_mut(&id)
            .map(|map| map.remove(&key).is_some())
            .unwrap_or(false)
    }) as i32
}

#[no_mangle]
pub extern "C" fn __nx_smap_size(id: i64) -> i64 {
    SMAPS.with(|maps| {
        maps.borrow()
            .get(&id)
            .map(|map| map.len() as i64)
            .unwrap_or(0)
    })
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_smap_free(id: i64) -> i32 {
    SMAPS.with(|maps| maps.borrow_mut().remove(&id).is_some()) as i32
}

// ── ByteBuffer (Vec<u8>) ────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __nx_buf_new() -> i64 {
    NEXT_BUF_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        BUFS.with(|bufs| bufs.borrow_mut().insert(id, Vec::new()));
        id
    })
}

#[no_mangle]
pub extern "C" fn __nx_buf_push_byte(id: i64, byte: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.push(byte as u8);
        }
    });
}

#[no_mangle]
pub extern "C" fn __nx_buf_push_i32_le(id: i64, val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(&(val as i32).to_le_bytes());
        }
    });
}

#[no_mangle]
pub extern "C" fn __nx_buf_push_i64_le(id: i64, val: i64) {
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    });
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_buf_push_string(id: i64, s_ptr: i32, s_len: i32) {
    let s = nexus_wasm_alloc::read_string(s_ptr, s_len);
    BUFS.with(|bufs| {
        if let Some(buf) = bufs.borrow_mut().get_mut(&id) {
            buf.extend_from_slice(s.as_bytes());
        }
    });
}

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_buf_length(id: i64) -> i64 {
    BUFS.with(|bufs| {
        bufs.borrow()
            .get(&id)
            .map(|buf| buf.len() as i64)
            .unwrap_or(0)
    })
}

#[no_mangle]
pub extern "C" fn __nx_buf_get_byte(id: i64, idx: i64) -> i64 {
    BUFS.with(|bufs| {
        bufs.borrow()
            .get(&id)
            .and_then(|buf| buf.get(idx as usize).map(|&b| b as i64))
            .unwrap_or(-1)
    })
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_buf_free(id: i64) -> i32 {
    BUFS.with(|bufs| bufs.borrow_mut().remove(&id).is_some()) as i32
}
