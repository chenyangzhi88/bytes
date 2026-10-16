mod immutable {
    use loom::sync::Arc;
    use loom::thread;

    use crate::Bytes;
    #[test]
    fn bytes_cloning_vec() {
        loom::model(|| {
            let a = Bytes::from(b"abcdefgh".to_vec());
            let addr = a.as_ptr() as usize;

            // test the Bytes::clone is Sync by putting it in an Arc
            let a1 = Arc::new(a);
            let a2 = a1.clone();

            let t1 = thread::spawn(move || {
                let b: Bytes = (*a1).clone();
                assert_eq!(b.as_ptr() as usize, addr);
            });

            let t2 = thread::spawn(move || {
                let b: Bytes = (*a2).clone();
                assert_eq!(b.as_ptr() as usize, addr);
            });

            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}

mod mutable {
    use loom::sync::Arc;
    use loom::thread;

    use crate::Bytes;
    use crate::BytesMut;

    #[test]
    fn bytes_mut_cloning_frozen() {
        loom::model(|| {
            let a = BytesMut::from(&b"abcdefgh"[..]).split().freeze();
            let addr = a.as_ptr() as usize;

            // test the Bytes::clone is Sync by putting it in an Arc
            let a1 = Arc::new(a);
            let a2 = a1.clone();

            let t1 = thread::spawn(move || {
                let b: Bytes = (*a1).clone();
                assert_eq!(b.as_ptr() as usize, addr);
            });

            let t2 = thread::spawn(move || {
                let b: Bytes = (*a2).clone();
                assert_eq!(b.as_ptr() as usize, addr);
            });

            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}

mod terminal {
    use crate::Bytes;
    use loom::sync::atomic::{AtomicUsize, Ordering};
    use loom::sync::Arc;

    struct Owner {
        data: [u8; 3],
        drops: Arc<AtomicUsize>,
        panic_as_ref: bool,
    }

    impl AsRef<[u8]> for Owner {
        fn as_ref(&self) -> &[u8] {
            if self.panic_as_ref {
                panic!("original AsRef panic in model");
            }
            &self.data
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn concurrent_released_owner_has_one_terminal_winner() {
        loom::model(|| {
            let drops = Arc::new(AtomicUsize::new(0));
            let releases = Arc::new(AtomicUsize::new(0));
            let observed_drops = drops.clone();
            let observed_releases = releases.clone();
            let bytes = Bytes::from_owner_with_release(
                Owner {
                    data: [1, 2, 3],
                    drops: drops.clone(),
                    panic_as_ref: false,
                },
                move || {
                    assert_eq!(observed_drops.load(Ordering::SeqCst), 1);
                    assert_eq!(observed_releases.fetch_add(1, Ordering::SeqCst), 0);
                },
            );
            let left = bytes.clone();
            let right = bytes.slice(1..);
            let first = loom::thread::spawn(move || {
                assert_eq!(left.as_ref(), &[1, 2, 3]);
                drop(left);
            });
            let second = loom::thread::spawn(move || {
                assert_eq!(right.as_ref(), &[2, 3]);
                drop(right);
            });
            drop(bytes);
            first.join().unwrap();
            second.join().unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            assert_eq!(releases.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn as_ref_unwind_retains_the_original_terminal_authority() {
        loom::model(|| {
            let drops = Arc::new(AtomicUsize::new(0));
            let releases = Arc::new(AtomicUsize::new(0));
            let observed_drops = drops.clone();
            let observed_releases = releases.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Bytes::from_owner_with_release(
                    Owner {
                        data: [1, 2, 3],
                        drops: drops.clone(),
                        panic_as_ref: true,
                    },
                    move || {
                        assert_eq!(observed_drops.load(Ordering::SeqCst), 1);
                        assert_eq!(observed_releases.fetch_add(1, Ordering::SeqCst), 0);
                    },
                )
            }));
            assert!(result.is_err());
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            assert_eq!(releases.load(Ordering::SeqCst), 1);
        });
    }
}
