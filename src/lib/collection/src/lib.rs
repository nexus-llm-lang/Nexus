use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

thread_local! {
    static MAPS: RefCell<HashMap<i64, HashMap<i64, i64>>> = RefCell::new(HashMap::new());
    static SETS: RefCell<HashMap<i64, HashSet<i64>>> = RefCell::new(HashMap::new());
    static NEXT_MAP_ID: Cell<i64> = Cell::new(1);
    static NEXT_SET_ID: Cell<i64> = Cell::new(1);
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
