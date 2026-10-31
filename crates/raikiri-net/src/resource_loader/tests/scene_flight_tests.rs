use super::*;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct BuildGate {
    released: Mutex<bool>,
    changed: Condvar,
    builds: AtomicUsize,
}

impl BuildGate {
    fn wait(&self) {
        self.builds.fetch_add(1, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

struct Release<'a>(&'a BuildGate);
impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn build(bytes: &[u8], size: ImageRasterSize) -> Option<VectorImage> {
    SvgDocument::parse(bytes)
        .ok()?
        .scene(
            SvgViewport {
                width: size.width,
                height: size.height,
            },
            SvgRootStyle::default(),
        )
        .ok()
}

fn spin_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "flight participants did not arrive"
        );
        std::thread::yield_now();
    }
}

fn wait_for_participants(source: &ImageSource, workers: usize) -> Arc<SceneFlight> {
    let mut joined = None;
    spin_until(|| {
        joined = source
            .scene_cache
            .try_lock()
            .ok()
            .and_then(|cache| cache.loading.first().map(|(_, flight)| flight.clone()));
        joined.is_some()
    });
    let flight = joined.unwrap();
    // The registry, owner and this observer hold three references. Every
    // follower holds one more while waiting, without holding the cache mutex.
    spin_until(|| Arc::strong_count(&flight) >= workers + 2);
    flight
}

#[test]
fn eight_joined_requests_build_once_share_identity_and_charge_once() {
    for limit in [0, MAX_CACHE_BYTES] {
        let budget = test_budget(limit);
        let source = svg_source(&budget);
        let gate = BuildGate::default();
        let size = box_of(8.0);
        let scenes = std::thread::scope(|scope| {
            let _release = Release(&gate);
            let owner = scope.spawn(|| {
                source.scene_with(size, |bytes, size| {
                    gate.wait();
                    build(bytes, size)
                })
            });
            spin_until(|| gate.builds.load(Ordering::SeqCst) == 1);
            let followers = (0..7)
                .map(|_| {
                    scope.spawn(|| {
                        source.scene_with(size, |_, _| panic!("a follower built a scene"))
                    })
                })
                .collect::<Vec<_>>();
            let flight = wait_for_participants(&source, 8);
            gate.release();
            let mut scenes = vec![owner.join().unwrap().unwrap()];
            scenes.extend(
                followers
                    .into_iter()
                    .map(|thread| thread.join().unwrap().unwrap()),
            );
            assert!(matches!(
                *flight.outcome.lock().unwrap(),
                SceneOutcome::Complete(Some(_))
            ));
            scenes
        });
        assert_eq!(gate.builds.load(Ordering::SeqCst), 1);
        assert!(scenes.iter().all(|scene| Arc::ptr_eq(scene, &scenes[0])));
        let cached = source.scene_cache.lock().unwrap();
        assert!(cached.loading.is_empty());
        assert_eq!(cached.cached.len(), usize::from(limit > 0));
        assert_eq!(
            budget.lock().unwrap().used,
            if limit > 0 {
                scene_bytes(&scenes[0])
            } else {
                0
            }
        );
        drop(cached);
        if limit == 0 {
            assert!(!Arc::ptr_eq(&scenes[0], &source.scene(size).unwrap()));
        }
    }
}

#[test]
fn different_sizes_and_sources_build_while_the_first_owner_is_blocked() {
    let budget = test_budget(MAX_CACHE_BYTES);
    let a = svg_source(&budget);
    let b = svg_source(&budget);
    let gate = BuildGate::default();
    let (started, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        let _release = Release(&gate);
        let first = scope.spawn(|| {
            a.scene_with(box_of(4.0), |bytes, size| {
                gate.wait();
                build(bytes, size)
            })
        });
        spin_until(|| gate.builds.load(Ordering::SeqCst) == 1);
        let second = scope.spawn(|| {
            a.scene_with(box_of(8.0), |bytes, size| {
                started.send(1).unwrap();
                build(bytes, size)
            })
        });
        let third = scope.spawn(|| {
            b.scene_with(box_of(4.0), |bytes, size| {
                started.send(2).unwrap();
                build(bytes, size)
            })
        });
        let one = receiver.recv_timeout(DEADLINE);
        let two = receiver.recv_timeout(DEADLINE);
        gate.release();
        let scenes = [
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap(),
            third.join().unwrap().unwrap(),
        ];
        assert_eq!(
            [one.unwrap(), two.unwrap()]
                .into_iter()
                .collect::<HashSet<_>>(),
            HashSet::from([1, 2])
        );
        assert_eq!(
            budget.lock().unwrap().used,
            scenes.iter().map(|scene| scene_bytes(scene)).sum::<u64>()
        );
    });
}

#[test]
fn none_panic_and_poison_release_joined_waiters_and_clean_pending() {
    for outcome in [0, 1, 2, 3] {
        let budget = test_budget(MAX_CACHE_BYTES);
        let source = svg_source(&budget);
        let gate = BuildGate::default();
        std::thread::scope(|scope| {
            let _release = Release(&gate);
            let owner = scope.spawn(|| {
                source.scene_with(box_of(4.0), |bytes, size| {
                    gate.wait();
                    match outcome {
                        0 => None,
                        1 => panic!("injected scene construction panic"),
                        2 => {
                            let _ = catch_unwind(AssertUnwindSafe(|| {
                                let _budget = budget.lock().unwrap();
                                panic!("poison budget");
                            }));
                            build(bytes, size)
                        }
                        _ => {
                            let _ = catch_unwind(AssertUnwindSafe(|| {
                                let _cache = source.scene_cache.lock().unwrap();
                                panic!("poison cache");
                            }));
                            build(bytes, size)
                        }
                    }
                })
            });
            spin_until(|| gate.builds.load(Ordering::SeqCst) == 1);
            let followers = (0..7)
                .map(|_| {
                    scope.spawn(|| {
                        source.scene_with(box_of(4.0), |_, _| {
                            panic!("a follower retried within the flight")
                        })
                    })
                })
                .collect::<Vec<_>>();
            let flight = wait_for_participants(&source, 8);
            gate.release();
            let owner_result = owner.join();
            assert!(if outcome == 1 {
                owner_result.is_err()
            } else {
                owner_result.unwrap().is_none()
            });
            for follower in followers {
                assert!(follower.join().unwrap().is_none());
            }
            assert!(matches!(
                *flight.outcome.lock().unwrap(),
                SceneOutcome::Complete(None)
            ));
        });
        assert!(
            source
                .scene_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .loading
                .is_empty()
        );
        if outcome < 2 {
            assert!(source.scene(box_of(4.0)).is_some());
        } else {
            assert!(source.scene(box_of(4.0)).is_none());
        }
    }
}

#[test]
fn nan_cleanup_and_signed_zero_preserve_size_equality() {
    let source = svg_source(&test_budget(MAX_CACHE_BYTES));
    for _ in 0..2 {
        assert!(source.scene_with(box_of(f32::NAN), |_, _| None).is_none());
        assert!(source.scene_cache.lock().unwrap().loading.is_empty());
    }
    let gate = BuildGate::default();
    std::thread::scope(|scope| {
        let _release = Release(&gate);
        let owner = scope.spawn(|| {
            source.scene_with(box_of(-0.0), |_, _| {
                gate.wait();
                None
            })
        });
        spin_until(|| gate.builds.load(Ordering::SeqCst) == 1);
        let follower = scope.spawn(|| {
            source.scene_with(box_of(0.0), |_, _| {
                panic!("signed zero created a separate owner")
            })
        });
        let _flight = wait_for_participants(&source, 2);
        gate.release();
        assert!(owner.join().unwrap().is_none());
        assert!(follower.join().unwrap().is_none());
    });
    assert!(source.scene_cache.lock().unwrap().loading.is_empty());
}

#[test]
fn scene_cache_hits_refresh_lru_before_the_next_publication() {
    let budget = test_budget(MAX_CACHE_BYTES);
    let source = svg_source(&budget);
    let first = source.scene(box_of(1.0)).unwrap();
    let second = source.scene(box_of(2.0)).unwrap();
    source.scene(box_of(3.0)).unwrap();
    source.scene(box_of(4.0)).unwrap();
    assert!(Arc::ptr_eq(&first, &source.scene(box_of(1.0)).unwrap()));
    source.scene(box_of(5.0)).unwrap();
    assert!(Arc::ptr_eq(&first, &source.scene(box_of(1.0)).unwrap()));
    assert!(!Arc::ptr_eq(&second, &source.scene(box_of(2.0)).unwrap()));
}

#[test]
fn poisoned_flight_releases_waiters_without_poisoning_the_cache() {
    let budget = test_budget(MAX_CACHE_BYTES);
    let source = svg_source(&budget);
    let gate = BuildGate::default();
    std::thread::scope(|scope| {
        let _release = Release(&gate);
        let owner = scope.spawn(|| {
            source.scene_with(box_of(4.0), |bytes, size| {
                gate.wait();
                build(bytes, size)
            })
        });
        spin_until(|| gate.builds.load(Ordering::SeqCst) == 1);
        let follower = scope.spawn(|| {
            source.scene_with(box_of(4.0), |_, _| {
                panic!("a follower rebuilt a poisoned flight")
            })
        });
        let flight = wait_for_participants(&source, 2);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _outcome = flight.outcome.lock().unwrap();
            panic!("poison flight outcome");
        }));
        // A caller reaching the already-poisoned slot also fails closed.
        assert!(flight.wait().is_none());
        gate.release();
        let scene = owner.join().unwrap().unwrap();
        assert!(follower.join().unwrap().is_none());
        assert!(Arc::ptr_eq(&scene, &source.scene(box_of(4.0)).unwrap()));
        let cache = source.scene_cache.lock().unwrap();
        assert!(cache.loading.is_empty());
        assert_eq!(budget.lock().unwrap().used, scene_bytes(&scene));
        assert!(flight.outcome.is_poisoned());
    });
}
