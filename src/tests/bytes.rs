use super::super::Bytes;
use crate::{Buf, BytesMut};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::marker::PhantomPinned;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::vec::Vec;

// Observe the actual controller deallocation, rather than assuming owner Drop
// implies its surrounding Box has already been freed. Only the next allocation
// on the constructing thread is watched; other test-runner threads cannot arm it.
std::thread_local! {
    static WATCH_NEXT: Cell<bool> = Cell::new(false);
}
static SERIAL: Mutex<()> = Mutex::new(());
static WATCHED: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_SIZE: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_ALIGN: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATED: AtomicBool = AtomicBool::new(false);

struct ObservedAllocator;

unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if WATCH_NEXT
            .try_with(|next| next.replace(false))
            .unwrap_or(false)
        {
            WATCHED.store(pointer as usize, Ordering::SeqCst);
            ALLOCATED_SIZE.store(layout.size(), Ordering::SeqCst);
            ALLOCATED_ALIGN.store(layout.align(), Ordering::SeqCst);
            DEALLOCATED.store(false, Ordering::SeqCst);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let watched = pointer as usize == WATCHED.load(Ordering::SeqCst);
        System.dealloc(pointer, layout);
        if watched {
            // This store happens after the real allocator deallocation returns.
            DEALLOCATED.store(true, Ordering::SeqCst);
        }
    }
}

#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;

#[derive(Clone)]
struct Facts {
    address: Arc<AtomicUsize>,
    as_ref_calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
}

impl Facts {
    fn new() -> Self {
        Self {
            address: Arc::new(AtomicUsize::new(0)),
            as_ref_calls: Arc::new(AtomicUsize::new(0)),
            drops: Arc::new(AtomicUsize::new(0)),
            releases: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn release(&self) -> impl FnOnce() + Send + 'static {
        let facts = self.clone();
        move || {
            assert_eq!(facts.drops.load(Ordering::SeqCst), 1);
            assert!(DEALLOCATED.load(Ordering::SeqCst));
            assert_eq!(facts.releases.fetch_add(1, Ordering::SeqCst), 0);
        }
    }
}

#[repr(align(64))]
struct Owner {
    bytes: Vec<u8>,
    facts: Facts,
    panic_as_ref: bool,
    panic_drop: bool,
    _pin: PhantomPinned,
}

impl Owner {
    fn new(bytes: &[u8], facts: &Facts) -> Self {
        Self {
            bytes: bytes.to_vec(),
            facts: facts.clone(),
            panic_as_ref: false,
            panic_drop: false,
            _pin: PhantomPinned,
        }
    }
}

impl AsRef<[u8]> for Owner {
    fn as_ref(&self) -> &[u8] {
        assert_eq!(self.facts.as_ref_calls.fetch_add(1, Ordering::SeqCst), 0);
        self.facts
            .address
            .store(self as *const Self as usize, Ordering::SeqCst);
        if self.panic_as_ref {
            panic!("original owner AsRef panic");
        }
        &self.bytes
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        assert_eq!(
            self as *const Self as usize,
            self.facts.address.load(Ordering::SeqCst)
        );
        assert_eq!(self.facts.drops.fetch_add(1, Ordering::SeqCst), 0);
        if self.panic_drop {
            panic!("original owner destructor panic");
        }
    }
}

fn arm<T, F>(_: &F) -> Layout
where
    T: AsRef<[u8]> + Send + 'static,
    F: FnOnce() + Send + 'static,
{
    let layout = Bytes::from_owner_with_release_allocation_layout::<T, F>();
    WATCHED.store(0, Ordering::SeqCst);
    DEALLOCATED.store(false, Ordering::SeqCst);
    WATCH_NEXT.with(|next| next.set(true));
    layout
}

#[test]
fn controller_layout_and_last_slice_retirement_follow_real_deallocation() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let owner = Owner::new(b"one authoritative buffer", &facts);
    let pointer = owner.bytes.as_ptr();
    let release = facts.release();
    let expected = arm::<Owner, _>(&release);
    let mut bytes = Bytes::from_owner_with_release(owner, release);
    assert_eq!(ALLOCATED_SIZE.load(Ordering::SeqCst), expected.size());
    assert_eq!(ALLOCATED_ALIGN.load(Ordering::SeqCst), expected.align());
    assert_eq!(bytes.as_ptr(), pointer);
    let mut final_slice = bytes.slice(4..17);
    bytes.clear();
    drop(bytes);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
    assert!(!DEALLOCATED.load(Ordering::SeqCst));
    assert_eq!(final_slice.as_ref(), b"authoritative");
    final_slice.advance(3);
    final_slice.truncate(0);
    assert_eq!(facts.as_ref_calls.load(Ordering::SeqCst), 1);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
    drop(final_slice);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn empty_owned_handles_retire_while_detached_empty_slices_do_not_retain() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let owner = Owner::new(b"", &facts);
    let release = facts.release();
    arm::<Owner, _>(&release);
    let bytes = Bytes::from_owner_with_release(owner, release);
    let clone = bytes.clone();
    let detached = bytes.slice(0..0);
    let detached_ref = bytes.slice_ref(&[]);
    drop(bytes);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
    drop(clone);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
    assert!(detached.is_empty() && detached_ref.is_empty());
    drop((detached, detached_ref));
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn split_and_try_into_mut_keep_the_opaque_owner_until_last_handle() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let owner = Owner::new(b"abcdef", &facts);
    let release = facts.release();
    arm::<Owner, _>(&release);
    let mut tail = Bytes::from_owner_with_release(owner, release);
    let empty = tail.split_to(0);
    let head = tail.split_to(3);
    let tail = tail.try_into_mut().unwrap_err();
    assert_eq!(head.as_ref(), b"abc");
    assert_eq!(tail.as_ref(), b"def");
    drop(head);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
    drop(tail);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
    assert!(empty.is_empty());
}

#[test]
fn vector_and_mutable_conversions_copy_and_retire_the_consumed_reference() {
    let _serial = SERIAL.lock().unwrap();
    for mutable in [false, true] {
        let facts = Facts::new();
        let owner = Owner::new(b"copy this slice", &facts);
        let release = facts.release();
        arm::<Owner, _>(&release);
        let bytes = Bytes::from_owner_with_release(owner, release);
        let remaining = bytes.clone();
        if mutable {
            let mut copied = BytesMut::from(bytes);
            copied[0] = b'C';
            assert_eq!(copied.as_ref(), b"Copy this slice");
        } else {
            let copied = Vec::<u8>::from(bytes);
            assert_eq!(copied, b"copy this slice");
        }
        assert_eq!(remaining.as_ref(), b"copy this slice");
        assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
        drop(remaining);
        assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn as_ref_panic_destroys_the_pinned_owner_and_box_before_release() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let mut owner = Owner::new(b"panic before publishing", &facts);
    owner.panic_as_ref = true;
    let release = facts.release();
    arm::<Owner, _>(&release);
    let result = catch_unwind(AssertUnwindSafe(|| {
        Bytes::from_owner_with_release(owner, release)
    }));
    assert!(result.is_err());
    assert_eq!(facts.as_ref_calls.load(Ordering::SeqCst), 1);
    assert_eq!(facts.drops.load(Ordering::SeqCst), 1);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn owner_drop_panic_still_frees_the_controller_before_release() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let mut owner = Owner::new(b"panic during destructor", &facts);
    owner.panic_drop = true;
    let release = facts.release();
    arm::<Owner, _>(&release);
    let bytes = Bytes::from_owner_with_release(owner, release);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(bytes))).is_err());
    assert_eq!(facts.drops.load(Ordering::SeqCst), 1);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_last_clones_retire_exactly_once_after_controller_free() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let owner = Owner::new(b"concurrent immutable bytes", &facts);
    let release = facts.release();
    arm::<Owner, _>(&release);
    let bytes = Bytes::from_owner_with_release(owner, release);
    let barrier = Arc::new(std::sync::Barrier::new(9));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let cloned = bytes.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            assert_eq!(cloned.as_ref(), b"concurrent immutable bytes");
            barrier.wait();
            drop(cloned);
        }));
    }
    drop(bytes);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 0);
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(facts.as_ref_calls.load(Ordering::SeqCst), 1);
    assert_eq!(facts.drops.load(Ordering::SeqCst), 1);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn release_panic_is_not_retried_and_follows_actual_deallocation() {
    let _serial = SERIAL.lock().unwrap();
    let facts = Facts::new();
    let owner = Owner::new(b"release once", &facts);
    let retirement = facts.release();
    let release = move || {
        retirement();
        panic!("original release callback panic");
    };
    arm::<Owner, _>(&release);
    let bytes = Bytes::from_owner_with_release(owner, release);
    assert!(catch_unwind(AssertUnwindSafe(|| drop(bytes))).is_err());
    assert_eq!(facts.drops.load(Ordering::SeqCst), 1);
    assert_eq!(facts.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_sized_owner_and_send_only_release_still_have_one_real_controller() {
    let _serial = SERIAL.lock().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    // Cell is Send but not Sync. Immutable Bytes readers never access F, and
    // its unique terminal owner must not impose an unnecessary Sync bound.
    let send_only = Cell::new(0);
    let release = move || {
        send_only.set(1);
        assert!(DEALLOCATED.load(Ordering::SeqCst));
        assert_eq!(seen.fetch_add(1, Ordering::SeqCst), 0);
    };
    let layout = arm::<[u8; 0], _>(&release);
    let bytes = Bytes::from_owner_with_release([], release);
    assert!(layout.size() > 0);
    assert_eq!(ALLOCATED_SIZE.load(Ordering::SeqCst), layout.size());
    let clone = bytes.clone();
    drop(bytes);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    std::thread::spawn(move || drop(clone)).join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
