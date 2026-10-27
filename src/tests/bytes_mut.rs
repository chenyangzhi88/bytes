use crate::bytes_mut::{
    original_capacity_from_repr, original_capacity_to_repr, MAX_ORIGINAL_CAPACITY_WIDTH,
    MIN_ORIGINAL_CAPACITY_WIDTH,
};

#[test]
fn test_original_capacity_to_repr() {
    assert_eq!(original_capacity_to_repr(0), 0);

    let max_width = 32;

    for width in 1..(max_width + 1) {
        let cap = 1 << width - 1;

        let expected = if width < MIN_ORIGINAL_CAPACITY_WIDTH {
            0
        } else if width < MAX_ORIGINAL_CAPACITY_WIDTH {
            width - MIN_ORIGINAL_CAPACITY_WIDTH
        } else {
            MAX_ORIGINAL_CAPACITY_WIDTH - MIN_ORIGINAL_CAPACITY_WIDTH
        };

        assert_eq!(original_capacity_to_repr(cap), expected);

        if width > 1 {
            assert_eq!(original_capacity_to_repr(cap + 1), expected);
        }

        //  MIN_ORIGINAL_CAPACITY_WIDTH must be bigger than 7 to pass tests below
        if width == MIN_ORIGINAL_CAPACITY_WIDTH + 1 {
            assert_eq!(original_capacity_to_repr(cap - 24), expected - 1);
            assert_eq!(original_capacity_to_repr(cap + 76), expected);
        } else if width == MIN_ORIGINAL_CAPACITY_WIDTH + 2 {
            assert_eq!(original_capacity_to_repr(cap - 1), expected - 1);
            assert_eq!(original_capacity_to_repr(cap - 48), expected - 1);
        }
    }
}

#[test]
fn test_original_capacity_from_repr() {
    assert_eq!(0, original_capacity_from_repr(0));

    let min_cap = 1 << MIN_ORIGINAL_CAPACITY_WIDTH;

    assert_eq!(min_cap, original_capacity_from_repr(1));
    assert_eq!(min_cap * 2, original_capacity_from_repr(2));
    assert_eq!(min_cap * 4, original_capacity_from_repr(3));
    assert_eq!(min_cap * 8, original_capacity_from_repr(4));
    assert_eq!(min_cap * 16, original_capacity_from_repr(5));
    assert_eq!(min_cap * 32, original_capacity_from_repr(6));
    assert_eq!(min_cap * 64, original_capacity_from_repr(7));
}

#[test]
#[cfg(not(loom))]
fn backing_capacity_tracks_vec_offsets_shared_splits_and_reclaim() {
    use crate::{Buf, BytesMut};
    assert_eq!(BytesMut::new().backing_capacity(), 0);
    let mut input = BytesMut::with_capacity(64);
    let allocation = input.capacity();
    input.resize(32, 7);
    input.advance(16);
    assert_eq!(input.backing_capacity(), allocation);
    assert_eq!(input.capacity(), allocation - 16);
    assert!(input.try_reclaim(48));
    assert_eq!(input.backing_capacity(), allocation);
    assert_eq!(input.capacity(), allocation);
    let mut suffix = input.split_off(8);
    assert_eq!(input.capacity(), 8);
    assert_eq!(input.backing_capacity(), allocation);
    assert_eq!(suffix.backing_capacity(), allocation);
    let prefix = input.freeze();
    suffix.clear();
    assert_eq!(suffix.backing_capacity(), allocation);
    drop(prefix);
    assert!(suffix.try_reclaim(allocation));
    assert_eq!(suffix.capacity(), allocation);
    assert_eq!(suffix.backing_capacity(), allocation);
}

#[test]
#[cfg(not(loom))]
fn backing_capacity_reports_shared_relocation_without_changing_retained_aliases() {
    use crate::BytesMut;
    let mut input = BytesMut::with_capacity(64 * 1024);
    let allocation = input.capacity();
    input.resize(allocation - 16, 7);
    input.extend_from_slice(b"small suffix!!!!");
    let prefix = input.split_to(allocation - 16).freeze();
    assert_eq!(input.capacity(), 16);
    assert_eq!(input.backing_capacity(), allocation);
    let expected = input.to_vec();
    input.reserve(allocation + 1);
    assert!(input.capacity() >= allocation + 1 + input.len());
    assert_eq!(input.backing_capacity(), input.capacity());
    assert_eq!(input.as_ref(), expected);
    assert!(prefix.iter().all(|byte| *byte == 7));
    let original_view = input.backing_capacity();
    let split = input.split_to(8);
    assert_eq!(split.backing_capacity(), original_view);
    assert_eq!(input.backing_capacity(), original_view);
}
