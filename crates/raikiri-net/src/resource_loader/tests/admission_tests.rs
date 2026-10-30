use super::*;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, mpsc};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Released {
    all: bool,
    paths: HashSet<String>,
}

#[derive(Default)]
struct FetchGate {
    released: Mutex<Released>,
    changed: Condvar,
    calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    first_outcome: AtomicUsize,
    oversized: AtomicBool,
}

impl FetchGate {
    fn release(&self, path: &str) {
        self.released.lock().unwrap().paths.insert(path.into());
        self.changed.notify_all();
    }

    fn release_all(&self) {
        self.released.lock().unwrap().all = true;
        self.changed.notify_all();
    }
}

struct ReleaseAll<'a>(&'a FetchGate);
impl Drop for ReleaseAll<'_> {
    fn drop(&mut self) {
        self.0.release_all();
    }
}

struct ActiveFetch<'a>(&'a FetchGate);
impl Drop for ActiveFetch<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct ControlledProvider {
    gate: Arc<FetchGate>,
    started: mpsc::Sender<String>,
    bytes: &'static [u8],
}

impl NetworkProvider for ControlledProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.gate.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.gate.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = ActiveFetch(&self.gate);
        self.gate.peak.fetch_max(active, Ordering::SeqCst);
        let path = request.url.path().to_owned();
        self.started.send(path.clone()).unwrap();
        let mut released = self.gate.released.lock().unwrap();
        while !released.all && !released.paths.contains(&path) {
            released = self.gate.changed.wait(released).unwrap();
        }
        drop(released);
        let bytes = match self.gate.first_outcome.swap(0, Ordering::SeqCst) {
            1 => return Err(NetworkError::Other("injected fetch error".into())),
            2 => panic!("injected provider panic"),
            3 => b"invalid PNG".as_slice(),
            _ => self.bytes,
        };
        let bytes = if self.gate.oversized.load(Ordering::SeqCst) {
            vec![0; MAX_SVG_BYTES + 1]
        } else {
            bytes.to_vec()
        };
        Ok(FetchOutcome::Body(
            FetchedResource::new(bytes.into(), request.url).content_type(Some("image/png".into())),
        ))
    }
}

fn controlled() -> (ControlledProvider, Arc<FetchGate>, mpsc::Receiver<String>) {
    let (started, receiver) = mpsc::channel();
    let gate = Arc::new(FetchGate::default());
    (
        ControlledProvider {
            gate: gate.clone(),
            started,
            bytes: TINY_PNG,
        },
        gate,
        receiver,
    )
}

#[test]
fn independent_url_fetches_overlap_before_either_provider_returns() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::new(provider);
    let a = Url::parse("https://images.test/a.png").unwrap();
    let b = Url::parse("https://images.test/b.png").unwrap();
    let (first, second) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let a = scope.spawn(|| resolver.load_source(&a));
        let b = scope.spawn(|| resolver.load_source(&b));
        let first = started.recv_timeout(DEADLINE);
        let second = started.recv_timeout(DEADLINE);
        gate.release_all();
        assert!(a.join().unwrap().is_ok());
        assert!(b.join().unwrap().is_ok());
        (first, second)
    });
    assert!(first.is_ok(), "the first fetch must start");
    assert!(
        second.is_ok(),
        "another URL must start while the first fetch is held: {second:?}"
    );
    assert_eq!(gate.peak.load(Ordering::SeqCst), 2);
    assert_eq!(gate.active.load(Ordering::SeqCst), 0);
}

#[test]
fn successful_duplicate_waiters_share_one_source_key_and_permit() {
    let (provider, gate, started) = controlled();
    let url: Url = "https://images.test/a.png".parse().unwrap();
    let cost = url.as_str().len() as u64 + 8;
    let resolver = ResourceLoader::with_limits_and_concurrency(provider, cost, 1, 1);
    let barrier = std::sync::Barrier::new(9);
    let (first, extra, sources) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    resolver.load_source(&url)
                })
            })
            .collect();
        barrier.wait();
        let first = started.recv_timeout(DEADLINE);
        let extra = started.recv_timeout(Duration::from_millis(100));
        gate.release_all();
        let sources: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();
        (first, extra, sources)
    });
    assert!(first.is_ok());
    assert!(extra.is_err());
    assert!(
        sources
            .iter()
            .all(|source| Arc::ptr_eq(source, &sources[0]))
    );
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.peak.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.budget.lock().unwrap().used, cost);
    let state = resolver.state.lock().unwrap();
    assert_eq!(state.cache.len(), 1);
    assert!(state.loading.is_empty());
}

#[test]
fn configured_limit_holds_the_next_fetch_until_a_whole_load_completes() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::with_limits_and_concurrency(provider, MAX_CACHE_BYTES, 256, 2);
    let urls: Vec<Url> = (0..3)
        .map(|index| format!("https://images.test/{index}.png").parse().unwrap())
        .collect();
    let barrier = std::sync::Barrier::new(4);
    let (initial, extra, after_release) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let handles: Vec<_> = urls
            .iter()
            .map(|url| {
                scope.spawn(|| {
                    barrier.wait();
                    resolver.load_source(url)
                })
            })
            .collect();
        barrier.wait();
        let initial = [
            started.recv_timeout(DEADLINE),
            started.recv_timeout(DEADLINE),
        ];
        let extra = started.recv_timeout(Duration::from_millis(100));
        if let Ok(path) = &initial[0] {
            gate.release(path);
        }
        let after_release = started.recv_timeout(DEADLINE);
        gate.release_all();
        for handle in handles {
            assert!(handle.join().unwrap().is_ok());
        }
        (initial, extra, after_release)
    });
    assert!(initial.iter().all(Result::is_ok));
    assert!(
        extra.is_err(),
        "a third fetch cannot pass the two-load limit"
    );
    assert!(after_release.is_ok());
    assert_eq!(gate.peak.load(Ordering::SeqCst), 2);
    assert!(resolver.state.lock().unwrap().loading.is_empty());
}

#[test]
fn a_slow_url_allows_another_url_and_an_existing_cache_hit_to_finish() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::new(provider);
    let cached_url: Url = "https://images.test/cached.png".parse().unwrap();
    let slow_url: Url = "https://images.test/slow.png".parse().unwrap();
    let fast_url: Url = "https://images.test/fast.png".parse().unwrap();
    gate.release("/cached.png");
    let cached = resolver.load_source(&cached_url).unwrap();
    assert_eq!(started.recv_timeout(DEADLINE).unwrap(), "/cached.png");
    gate.release("/fast.png");
    let (slow_started, completed) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let slow = scope.spawn(|| resolver.load_source(&slow_url));
        let slow_started = started.recv_timeout(DEADLINE);
        let (done, receiver) = mpsc::channel();
        let resolver = &resolver;
        let fast_url = &fast_url;
        let cached_url = &cached_url;
        let cached = &cached;
        let fast_done = done.clone();
        let fast = scope.spawn(move || {
            fast_done
                .send(("fast", resolver.load_source(fast_url).is_ok()))
                .unwrap();
        });
        let hit = scope.spawn(move || {
            let source = resolver.load_source(cached_url).unwrap();
            done.send(("hit", Arc::ptr_eq(&source, cached))).unwrap();
        });
        let completed = [
            receiver.recv_timeout(DEADLINE),
            receiver.recv_timeout(DEADLINE),
        ];
        gate.release_all();
        assert!(slow.join().unwrap().is_ok());
        fast.join().unwrap();
        hit.join().unwrap();
        (slow_started, completed)
    });
    assert_eq!(slow_started.unwrap(), "/slow.png");
    assert!(
        completed
            .iter()
            .all(|result| result.as_ref().is_ok_and(|(_, ok)| *ok))
    );
    assert_eq!(gate.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn pending_keys_are_charged_before_another_url_can_fetch() {
    let (provider, gate, started) = controlled();
    let a: Url = "https://images.test/a.png".parse().unwrap();
    let b: Url = "https://images.test/b.png".parse().unwrap();
    let resolver = ResourceLoader::with_limits(
        provider,
        (a.as_str().len() + b.as_str().len() - 1) as u64,
        256,
    );
    let (first, denied, used) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let a = scope.spawn(|| resolver.load_source(&a));
        let first = started.recv_timeout(DEADLINE);
        let (done, receiver) = mpsc::channel();
        let resolver = &resolver;
        let url = &b;
        let b = scope.spawn(move || {
            let result = resolver.load_source(url);
            done.send(result.is_err()).unwrap();
            result
        });
        let denied = receiver.recv_timeout(DEADLINE);
        let used = resolver.budget.try_lock().ok().map(|budget| budget.used);
        gate.release_all();
        assert!(a.join().unwrap().is_ok());
        assert!(b.join().unwrap().is_err());
        (first, denied, used)
    });
    assert!(first.is_ok());
    assert!(denied.unwrap());
    assert_eq!(used, Some(a.as_str().len() as u64));
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        a.as_str().len() as u64 + 8
    );
    assert!(resolver.state.lock().unwrap().loading.is_empty());
}

#[test]
fn pending_entries_share_the_completed_entry_limit() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::with_limits(provider, MAX_CACHE_BYTES, 1);
    let a: Url = "https://images.test/a.png".parse().unwrap();
    let b: Url = "https://images.test/b.png".parse().unwrap();
    let extra = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let a = scope.spawn(|| resolver.load_source(&a));
        let first = started.recv_timeout(DEADLINE);
        let b = scope.spawn(|| resolver.load_source(&b));
        let extra = started.recv_timeout(Duration::from_millis(100));
        gate.release_all();
        assert!(a.join().unwrap().is_ok());
        assert!(b.join().unwrap().is_err());
        assert!(first.is_ok());
        extra
    });
    assert!(extra.is_err());
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
    let state = resolver.state.lock().unwrap();
    assert_eq!(state.cache.len(), 1);
    assert!(state.loading.is_empty());
}

#[test]
fn duplicate_waiters_retry_after_fetch_error_decode_error_and_provider_panic() {
    for outcome in [1, 2, 3] {
        let (provider, gate, started) = controlled();
        gate.first_outcome.store(outcome, Ordering::SeqCst);
        let resolver = ResourceLoader::new(provider);
        let url: Url = "https://images.test/a.png".parse().unwrap();
        let (first, results) = std::thread::scope(|scope| {
            let _release = ReleaseAll(&gate);
            let handles: Vec<_> = (0..2)
                .map(|_| scope.spawn(|| resolver.load_source(&url)))
                .collect();
            let first = started.recv_timeout(DEADLINE);
            gate.release_all();
            (
                first,
                handles
                    .into_iter()
                    .map(|handle| handle.join())
                    .collect::<Vec<_>>(),
            )
        });
        assert!(first.is_ok());
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(Ok(_))))
                .count(),
            1
        );
        assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
        assert!(!resolver.budget.is_poisoned());
        assert!(resolver.state.lock().unwrap().loading.is_empty());
        assert_eq!(
            resolver.budget.lock().unwrap().used,
            url.as_str().len() as u64 + 8
        );
        let other: Url = "https://images.test/other.png".parse().unwrap();
        assert!(resolver.load_source(&other).is_ok());
        assert_eq!(gate.calls.load(Ordering::SeqCst), 3);
    }
}

#[test]
fn concurrent_svg_sources_cannot_exceed_the_shared_byte_budget() {
    let (mut provider, gate, started) = controlled();
    provider.bytes = TWO_COLOR_SVG;
    let a: Url = "https://images.test/a.svg".parse().unwrap();
    let b: Url = "https://images.test/b.svg".parse().unwrap();
    let one = (a.as_str().len() + TWO_COLOR_SVG.len()) as u64;
    let resolver = ResourceLoader::with_limits(provider, 2 * one - 1, 256);
    let (starts, results) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let a = scope.spawn(|| resolver.load_source(&a));
        let b = scope.spawn(|| resolver.load_source(&b));
        let starts = [
            started.recv_timeout(DEADLINE),
            started.recv_timeout(DEADLINE),
        ];
        gate.release_all();
        (starts, [a.join().unwrap(), b.join().unwrap()])
    });
    assert!(starts.iter().all(Result::is_ok));
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(resolver.budget.lock().unwrap().used, one);
    let state = resolver.state.lock().unwrap();
    assert_eq!(state.cache.len(), 1);
    assert!(state.loading.is_empty());
}

#[test]
fn concurrent_oversized_inputs_release_every_key_and_permit() {
    let (provider, gate, started) = controlled();
    gate.oversized.store(true, Ordering::SeqCst);
    let resolver = ResourceLoader::new(provider);
    let a: Url = "https://images.test/a.png".parse().unwrap();
    let b: Url = "https://images.test/b.png".parse().unwrap();
    let (starts, results) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let a = scope.spawn(|| resolver.load_source(&a));
        let b = scope.spawn(|| resolver.load_source(&b));
        let starts = [
            started.recv_timeout(DEADLINE),
            started.recv_timeout(DEADLINE),
        ];
        gate.release_all();
        (starts, [a.join().unwrap(), b.join().unwrap()])
    });
    assert!(starts.iter().all(Result::is_ok));
    assert!(results.iter().all(Result::is_err));
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    assert!(resolver.state.lock().unwrap().loading.is_empty());
    gate.oversized.store(false, Ordering::SeqCst);
    assert!(resolver.load_source(&a).is_ok());
}

fn recording() -> ResourceLoader<RecordingProvider> {
    ResourceLoader::new(RecordingProvider {
        fetch_count: AtomicUsize::new(0),
        bytes: TINY_PNG,
    })
}

fn assert_empty<N: NetworkProvider>(resolver: &ResourceLoader<N>) {
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    let state = resolver.state.lock().unwrap();
    assert!(state.cache.is_empty());
    assert!(state.loading.is_empty());
}

#[test]
fn decoder_unwind_releases_the_budget_without_poison_and_allows_retry() {
    let resolver = recording();
    let url: Url = "https://images.test/a.png".parse().unwrap();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        resolver.load_source_with(&url, |_, _, _, _| {
            panic!("injected source construction panic")
        })
    }));
    assert!(panic.is_err());
    assert!(!resolver.budget.is_poisoned());
    assert!(!resolver.state.is_poisoned());
    assert_empty(&resolver);
    assert!(resolver.load_source(&url).is_ok());
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 2);
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        url.as_str().len() as u64 + 8
    );
}

#[test]
fn uncommitted_output_is_checked_again_before_publication() {
    let url: Url = "https://images.test/a.png".parse().unwrap();
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        url.as_str().len() as u64 + 15,
        256,
    );
    let result = resolver.load_source_with(&url, |_, _, _, available| {
        assert_eq!(available, 15);
        Ok(Arc::new(ImageSource::raster(
            Arc::new(DecodedImage::new(2, 2, vec![0; 16])),
            resolver.budget.clone(),
        )))
    });
    assert!(result.is_err());
    assert_empty(&resolver);
    assert!(resolver.load_source(&url).is_ok());
}

#[test]
fn cache_hits_do_not_wait_for_a_decoder_holding_the_budget() {
    let resolver = recording();
    let cached_url: Url = "https://images.test/cached.png".parse().unwrap();
    let slow_url: Url = "https://images.test/slow.png".parse().unwrap();
    let cached = resolver.load_source(&cached_url).unwrap();
    let (entered, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (started, hit) = std::thread::scope(|scope| {
        let resolver = &resolver;
        let slow_url = &slow_url;
        let cached_url = &cached_url;
        let cached = &cached;
        let owner = scope.spawn(move || {
            resolver.load_source_with(slow_url, |bytes, data, svg, available| {
                entered.send(()).unwrap();
                release_rx.recv().unwrap();
                resolver.build_source(bytes, data, svg, available)
            })
        });
        let started = entered_rx.recv_timeout(DEADLINE);
        let (done, receiver) = mpsc::channel();
        let hit = scope.spawn(move || {
            let result = resolver
                .load_source(cached_url)
                .is_ok_and(|source| Arc::ptr_eq(&source, cached));
            done.send(result).unwrap();
        });
        let completed = receiver.recv_timeout(DEADLINE);
        let _ = release.send(());
        assert!(owner.join().unwrap().is_ok());
        hit.join().unwrap();
        (started, completed)
    });
    assert!(started.is_ok());
    assert!(hit.unwrap());
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 2);
}

fn poison<T: Send>(mutex: &Mutex<T>) {
    std::thread::scope(|scope| {
        let result = scope
            .spawn(|| {
                let _guard = mutex.lock().unwrap();
                panic!("injected mutex poison");
            })
            .join();
        assert!(result.is_err());
    });
}

#[test]
fn cleanup_refunds_its_key_after_external_budget_poison_and_stays_fail_closed() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::new(provider);
    let url: Url = "https://images.test/a.png".parse().unwrap();
    let (entered, result) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let owner = scope.spawn(|| resolver.load_source(&url));
        let entered = started.recv_timeout(DEADLINE);
        if entered.is_ok() {
            poison(&resolver.budget);
        }
        gate.release_all();
        (entered, owner.join().unwrap())
    });
    assert!(entered.is_ok());
    assert!(result.is_err());
    assert!(resolver.budget.is_poisoned());
    assert_eq!(resolver.budget.lock().err().unwrap().into_inner().used, 0);
    assert!(resolver.state.lock().unwrap().loading.is_empty());
    assert!(resolver.load_source(&url).is_err());
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cleanup_removes_its_pending_entry_after_state_poison_and_stays_fail_closed() {
    let (provider, gate, started) = controlled();
    let resolver = ResourceLoader::new(provider);
    let url: Url = "https://images.test/a.png".parse().unwrap();
    let (entered, result) = std::thread::scope(|scope| {
        let _release = ReleaseAll(&gate);
        let owner = scope.spawn(|| resolver.load_source(&url));
        let entered = started.recv_timeout(DEADLINE);
        if entered.is_ok() {
            poison(&resolver.state);
        }
        gate.release_all();
        (entered, owner.join().unwrap())
    });
    assert!(entered.is_ok());
    assert!(result.is_err());
    assert!(resolver.state.is_poisoned());
    assert!(
        resolver
            .state
            .lock()
            .err()
            .unwrap()
            .into_inner()
            .loading
            .is_empty()
    );
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    assert!(resolver.load_source(&url).is_err());
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_poisoned_condvar_wait_returns_a_resource_error() {
    let resolver = recording();
    poison(&resolver.state);
    let state = resolver.state.lock().err().unwrap().into_inner();
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            drop(resolver.state.lock().err().unwrap().into_inner());
            resolver.changed.notify_all();
        });
        resolver.wait_for_change(state)
    });
    assert!(result.is_err());
    assert!(resolver.state.is_poisoned());
}
