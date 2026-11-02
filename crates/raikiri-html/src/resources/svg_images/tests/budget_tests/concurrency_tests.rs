use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Condvar};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Gate {
    calls: AtomicUsize,
    released: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn wait(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
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

struct Release<'a>(&'a Gate);
impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn spin_until(mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !ready() {
        assert!(
            Instant::now() < end,
            "data flight participants did not arrive"
        );
        std::thread::yield_now();
    }
}

fn joined(images: &RenderSvgImages<'_, '_>, workers: usize) -> Arc<DataFlight> {
    let flight = images.state().data[0].pending.as_ref().unwrap().clone();
    // Registry, owner guard and observer, plus one Arc per waiting caller.
    spin_until(|| Arc::strong_count(&flight) >= workers + 2);
    flight
}

#[test]
fn joined_success_failure_and_panic_charge_the_encoded_key_once() {
    let bytes = b"encoded";
    for outcome in [0, 1, 2] {
        let (resources, process) = isolated(u64::MAX, u64::MAX);
        let images = resources.svg_images(None);
        let gate = Gate::default();
        std::thread::scope(|scope| {
            let _release = Release(&gate);
            let owner = scope.spawn(|| {
                images.data_with("image/png", bytes, |_, _, _| {
                    gate.wait();
                    match outcome {
                        0 => Ok(fresh()),
                        1 => Err(Box::new(RenderWarning::new(
                            WarningKind::ResourceFallback {
                                kind: ResourceKind::Image,
                                url: None,
                            },
                            "injected decode failure".into(),
                        ))),
                        _ => panic!("injected decoder panic"),
                    }
                })
            });
            spin_until(|| gate.calls.load(Ordering::SeqCst) == 1);
            let followers = (0..7)
                .map(|_| {
                    scope.spawn(|| {
                        images.data_with("image/png", bytes, |_, _, _| panic!("a follower decoded"))
                    })
                })
                .collect::<Vec<_>>();
            let _flight = joined(&images, 8);
            gate.release();
            let owner = owner.join();
            if outcome == 2 {
                assert!(owner.is_err());
            } else if outcome == 0 {
                let image = owner.unwrap().unwrap();
                for follower in followers {
                    assert!(Arc::ptr_eq(&image, &follower.join().unwrap().unwrap()));
                }
                return;
            } else {
                assert!(owner.unwrap().is_none());
            }
            for follower in followers {
                assert!(follower.join().unwrap().is_none());
            }
        });
        assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
        assert_eq!(images.state().attempts, 1);
        assert_eq!(images.state().data.len(), 1);
        assert!(images.state().data[0].pending.is_none());
        assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
        assert_eq!(
            process.retained(),
            key_bytes(bytes) + if outcome == 0 { 4 } else { 0 }
        );
        let repeated = images.data_with("image/png", bytes, |_, _, _| {
            panic!("a completed key decoded")
        });
        assert_eq!(repeated.is_some(), outcome == 0);
        // A new key can acquire a permit even after the previous owner panicked.
        assert!(
            images
                .data_with("image/png", b"next", |_, _, _| Ok(fresh()))
                .is_some()
        );
        drop(repeated);
        drop(images);
        assert_eq!(process.retained(), 0);
    }
}

#[test]
fn registered_owner_reentry_never_waits_for_itself() {
    let bytes = b"encoded";
    let (resources, process) = isolated(u64::MAX, u64::MAX);
    let images = resources.svg_images(None);
    let image = images
        .data_with("image/png", bytes, |_, _, _| {
            assert!(
                images
                    .data_with("image/png", bytes, |_, _, _| panic!(
                        "recursive key decoded"
                    ))
                    .is_none()
            );
            Ok(fresh())
        })
        .unwrap();
    assert_eq!(image.rgba, [255, 0, 0, 255]);
    assert_eq!(images.state().attempts, 1);
    assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
    assert_eq!(process.retained(), key_bytes(bytes) + 4);
}

#[test]
fn simultaneous_pixel_admission_obeys_shared_cache_and_process_caps() {
    let key = key_bytes(b"a");
    for separate_caches in [false, true] {
        let process = Arc::new(ProcessImageBudget::new(
            if separate_caches {
                2 * key + 4
            } else {
                u64::MAX
            },
            4,
        ));
        let a = RenderResources::new().with_test_process_budget(Arc::clone(&process));
        a.image_cache.lock().unwrap().max_bytes = if separate_caches {
            u64::MAX
        } else {
            2 * key + 4
        };
        let b = if separate_caches {
            RenderResources::new().with_test_process_budget(Arc::clone(&process))
        } else {
            a.clone()
        };
        let first = a.svg_images(None);
        let second = b.svg_images(None);
        let barrier = Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                first.data_with("image/png", b"a", |_, _, limit| {
                    assert!(limit >= 4);
                    barrier.wait();
                    Ok(fresh())
                })
            });
            let second = scope.spawn(|| {
                second.data_with("image/png", b"b", |_, _, limit| {
                    assert!(limit >= 4);
                    barrier.wait();
                    Ok(fresh())
                })
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.iter().filter(|result| result.is_some()).count(), 1);
        assert_eq!(process.retained(), 2 * key + 4);
        for (images, bytes, result) in [
            (&first, b"a".as_slice(), &results[0]),
            (&second, b"b".as_slice(), &results[1]),
        ] {
            assert_eq!(
                images
                    .data_with("image/png", bytes, |_, _, _| panic!(
                        "pixel admission outcome retried"
                    ))
                    .is_some(),
                result.is_some()
            );
        }
        drop(first);
        drop(second);
        assert_eq!(process.retained(), 0);
    }
}

struct PolicyGate(Gate);

impl raikiri_traits::ResourcePolicy for PolicyGate {
    fn is_scheme_allowed(&self, _: &str, _: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _: &str, _: ResourceKind) -> bool {
        true
    }
    fn allow_redirect(&self, _: &Url, _: &Url, _: u32) -> bool {
        true
    }
    fn max_redirect_hops(&self, _: ResourceKind) -> u32 {
        4
    }
    fn max_fetch_bytes(&self, _: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, _: ResourceKind) -> Option<u64> {
        None
    }
    fn fetch_timeout(&self, _: ResourceKind) -> Duration {
        Duration::from_secs(60)
    }
    fn decode_timeout(&self, _: ResourceKind) -> Duration {
        Duration::from_secs(60)
    }
    fn allowed_mime_types(&self, _: ResourceKind) -> Vec<String> {
        self.0.wait();
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        16
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        16
    }
}

#[test]
fn requests_racing_in_policy_callbacks_recheck_before_aggregate_charge() {
    let bytes = b"encoded";
    let gate = PolicyGate(Gate::default());
    let (resources, process) = isolated(u64::MAX, u64::MAX);
    let resources = resources.network_policy(&gate);
    let images = resources.svg_images(None);
    let decodes = AtomicUsize::new(0);
    let results = std::thread::scope(|scope| {
        let _release = Release(&gate.0);
        let threads = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    images.data_with("image/png", bytes, |_, _, _| {
                        decodes.fetch_add(1, Ordering::SeqCst);
                        Ok(fresh())
                    })
                })
            })
            .collect::<Vec<_>>();
        spin_until(|| gate.0.calls.load(Ordering::SeqCst) == 2);
        gate.0.release();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(Arc::ptr_eq(&results[0], &results[1]));
    assert_eq!(decodes.load(Ordering::SeqCst), 1);
    assert_eq!(images.state().data.len(), 1);
    assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
    assert_eq!(process.retained(), key_bytes(bytes) + 4);
}

#[test]
fn data_completion_returns_its_decode_permit_before_waiting_for_url_state() {
    let process = Arc::new(ProcessImageBudget::new(u64::MAX, 2));
    let resources = RenderResources::new().with_test_process_budget(Arc::clone(&process));
    let images = resources.svg_images(None);
    let gate = Gate::default();
    let (done, received) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let _release = Release(&gate);
        // Keep one permit as a rescue path: a regression can be released and
        // joined after the timeout, rather than hanging the test process.
        let spare = process.acquire();
        let owner = scope.spawn(|| {
            let image = images.data_with("image/png", b"encoded", |_, _, _| {
                gate.wait();
                Ok(fresh())
            });
            done.send(()).unwrap();
            image
        });
        spin_until(|| gate.calls.load(Ordering::SeqCst) == 1);
        let url = scope.spawn(|| images.url(concat!(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEElEQVR4nGP5z8Dw",
            "nxFIAAAQIQMDHl0gUAAAAABJRU5ErkJggg=="
        )));
        // The data decoder is paused without State, so only the URL request
        // can hold this lock while waiting for the unavailable permit.
        spin_until(|| images.state.try_lock().is_err());
        gate.release();
        let completed_without_rescue = received.recv_timeout(Duration::from_secs(3)).is_ok();
        drop(spare);
        assert!(owner.join().unwrap().is_some());
        assert!(url.join().unwrap().is_some());
        assert!(
            completed_without_rescue,
            "data completion held its permit while waiting for URL State"
        );
    });
}
