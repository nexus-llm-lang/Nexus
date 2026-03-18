use nexus::runtime::string_heap::{StringHandle, StringHeap, StringHeapError};
use proptest::prelude::*;

#[test]
fn retain_and_release_updates_refcount_and_frees_slot() {
    let mut heap = StringHeap::new();
    let h = heap.alloc_str("hello");
    assert_eq!(heap.ref_count(h).unwrap(), 1);

    heap.retain(h).unwrap();
    assert_eq!(heap.ref_count(h).unwrap(), 2);

    heap.release(h).unwrap();
    assert_eq!(heap.ref_count(h).unwrap(), 1);

    heap.release(h).unwrap();
    assert!(matches!(
        heap.len(h),
        Err(StringHeapError::InvalidHandle(_))
    ));
}

#[test]
fn concat_produces_new_string_handle() {
    let mut heap = StringHeap::new();
    let a = heap.alloc_str("foo");
    let b = heap.alloc_str("bar");
    let c = heap.concat(a, b).unwrap();

    assert_eq!(heap.to_utf8_string(c).unwrap(), "foobar");
    assert_eq!(heap.ref_count(c).unwrap(), 1);
}

#[test]
fn freed_slot_is_reused() {
    let mut heap = StringHeap::new();
    let first = heap.alloc_str("x");
    heap.release(first).unwrap();
    let second = heap.alloc_str("y");

    assert_eq!(first.raw(), second.raw());
    assert_eq!(heap.to_utf8_string(second).unwrap(), "y");
}

#[test]
fn invalid_utf8_is_reported() {
    let mut heap = StringHeap::new();
    let h = heap.alloc_bytes(vec![0xff, 0xfe]);

    assert!(matches!(
        heap.to_utf8_string(h),
        Err(StringHeapError::InvalidUtf8(_))
    ));
}

#[test]
fn unknown_handle_is_rejected() {
    let heap = StringHeap::new();
    let unknown = StringHandle::new(9999);
    assert!(matches!(
        heap.bytes(unknown),
        Err(StringHeapError::InvalidHandle(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_alloc_retain_release_refcount(s in "\\PC{0,64}", n in 1usize..8) {
        let mut heap = StringHeap::new();
        let h = heap.alloc_str(&s);
        prop_assert_eq!(heap.ref_count(h).unwrap(), 1);

        for i in 0..n {
            heap.retain(h).unwrap();
            prop_assert_eq!(heap.ref_count(h).unwrap(), (2 + i) as u32);
        }

        for i in 0..n {
            heap.release(h).unwrap();
            prop_assert_eq!(heap.ref_count(h).unwrap(), (n - i) as u32);
        }

        // Final release frees the slot
        heap.release(h).unwrap();
        prop_assert!(matches!(
            heap.len(h),
            Err(StringHeapError::InvalidHandle(_))
        ));
    }

    #[test]
    fn prop_concat_is_string_concatenation(a in "\\PC{0,32}", b in "\\PC{0,32}") {
        let mut heap = StringHeap::new();
        let ha = heap.alloc_str(&a);
        let hb = heap.alloc_str(&b);
        let hc = heap.concat(ha, hb).unwrap();

        let expected = format!("{}{}", a, b);
        prop_assert_eq!(heap.to_utf8_string(hc).unwrap(), expected);
        prop_assert_eq!(heap.ref_count(hc).unwrap(), 1);
    }
}
