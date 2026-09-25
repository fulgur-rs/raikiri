use super::*;

use std::net::TcpListener;
use std::thread;
use std::time::{Duration as StdDuration, Instant as StdInstant};

fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let addr = listener.local_addr().expect("local_addr");
    let client = TcpStream::connect(addr).expect("connect to loopback listener");
    let (server, _) = listener.accept().expect("accept loopback connection");
    (client, server)
}

#[test]
fn a_constant_next_timeout_still_expires_at_the_true_deadline() {
    // This is the exact bug this transport exists to fix: `ureq`'s own
    // rustls integration reuses one frozen `NextTimeout` across every raw
    // read it performs to service a single logical operation. Trickle a
    // byte every 100ms — well inside any *individual* read's own 500ms
    // window — while replaying that same constant `NextTimeout` on every
    // call, exactly like `ureq`'s stale-snapshot behavior does. A
    // transport with the bug would never see this trickle time out, since
    // every individual read keeps succeeding within its own window.
    let (client, mut server) = loopback_pair();
    thread::spawn(move || {
        for _ in 0..50 {
            if server.write_all(b"x").is_err() {
                return;
            }
            thread::sleep(StdDuration::from_millis(100));
        }
    });

    let mut transport = DeadlineTcpTransport::new(client, LazyBuffers::new(1024, 1024));
    let timeout = NextTimeout {
        after: UreqDuration::from_millis(500),
        reason: Timeout::Global,
    };

    let start = StdInstant::now();
    let err = loop {
        match transport.await_input(timeout) {
            Ok(_) => continue,
            Err(e) => break e,
        }
    };
    let elapsed = start.elapsed();

    assert!(
        matches!(err, Error::Timeout(Timeout::Global)),
        "expected Error::Timeout(Global), got {err:?}"
    );
    assert!(
        elapsed >= StdDuration::from_millis(450) && elapsed < StdDuration::from_millis(2000),
        "expected to time out close to the 500ms deadline regardless of how many \
         individual trickled reads succeeded along the way (each one arriving \
         well within its own window), got {elapsed:?}"
    );
}

#[test]
fn an_unchanged_next_timeout_does_not_extend_the_deadline_but_a_changed_one_anchors_a_fresh_one() {
    let (client, server) = loopback_pair();
    drop(server); // Unused: this test only exercises deadline bookkeeping, no real I/O.

    let mut transport = DeadlineTcpTransport::new(client, LazyBuffers::new(1024, 1024));

    let first = NextTimeout {
        after: UreqDuration::from_secs(10),
        reason: Timeout::Global,
    };
    transport
        .remaining(first)
        .expect("first call establishes a deadline");
    let first_deadline = transport.deadline;
    assert!(first_deadline.is_some());

    thread::sleep(StdDuration::from_millis(20));

    // Same `NextTimeout` value again: this is `ureq`'s own stale-snapshot
    // case (still inside the same logical operation from `ureq`'s point of
    // view) and must NOT reset the deadline — otherwise a trickling server
    // could extend it forever by keeping every raw read within its own
    // window, exactly the bug this transport fixes.
    transport
        .remaining(first)
        .expect("still within budget on the same operation");
    assert_eq!(
        transport.deadline, first_deadline,
        "an unchanged NextTimeout must not reset the deadline"
    );

    // A different `NextTimeout`: a new logical operation (or, in
    // production, a reused pooled connection's next logical request after
    // `is_open()` reset tracking) must anchor a fresh deadline.
    let second = NextTimeout {
        after: UreqDuration::from_secs(5),
        reason: Timeout::RecvBody,
    };
    transport
        .remaining(second)
        .expect("a new operation establishes a fresh deadline");
    assert_ne!(
        transport.deadline, first_deadline,
        "a changed NextTimeout must anchor a new deadline"
    );
}

#[test]
fn is_open_resets_deadline_tracking_for_pooled_connection_reuse() {
    let (client, server) = loopback_pair();
    drop(server);

    let mut transport = DeadlineTcpTransport::new(client, LazyBuffers::new(1024, 1024));
    let timeout = NextTimeout {
        after: UreqDuration::from_secs(10),
        reason: Timeout::Global,
    };
    transport
        .remaining(timeout)
        .expect("establish a deadline for the first logical request");
    assert!(transport.deadline.is_some());

    // `ureq` calls `is_open()` both when a pooled connection is checked
    // out for reuse and when it's checked back in. Simulate a reused
    // connection's next logical request starting.
    let _ = transport.is_open();

    assert!(
        transport.deadline.is_none() && transport.last_seen.is_none(),
        "is_open() must clear deadline tracking so a reused pooled connection's \
         next logical request starts fresh instead of inheriting the previous \
         request's deadline"
    );
}
