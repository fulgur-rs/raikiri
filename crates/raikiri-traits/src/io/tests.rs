use super::*;
use std::io::Write;

#[test]
fn accepts_boundary_size_equal_to_cap() {
    let cap = 32u64;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boundary.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&vec![0u8; cap as usize])
        .unwrap();
    let bytes = read_bounded_regular_file(&path, cap).expect("boundary accepted");
    assert_eq!(bytes.len() as u64, cap);
}

#[test]
fn accepts_zero_byte_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.bin");
    std::fs::File::create(&path).unwrap();
    let bytes = read_bounded_regular_file(&path, 128).expect("empty accepted");
    assert!(bytes.is_empty());
}

#[test]
fn rejects_oversized_up_front_at_cap_plus_one() {
    let cap = 32u64;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&vec![0u8; (cap + 1) as usize])
        .unwrap();
    match read_bounded_regular_file(&path, cap) {
        Err(RejectReason::Oversized {
            size,
            cap: c,
            phase: OversizePhase::PreOpen,
        }) => {
            assert_eq!(size, cap + 1);
            assert_eq!(c, cap);
        }
        other => panic!("expected Oversized{{PreOpen}}, got {other:?}"),
    }
}

#[test]
fn rejects_directory_as_not_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    match read_bounded_regular_file(dir.path(), 128) {
        Err(RejectReason::NotRegularFile) => {}
        other => panic!("expected NotRegularFile, got {other:?}"),
    }
}

/// Pins the `+1-probe` post-read reject in `read_bounded_from_open_file`,
/// AND pins the `+1` bound itself.
///
/// The file is deliberately much larger than `cap + 1` (20 bytes for
/// cap=8, so 12 bytes past the probe limit) and the assertion is `size
/// == cap + 1` (exactly). This shape catches two regression classes at
/// once:
///
///   1. Removing the post-read `bytes.len() > cap` check
///      (`Oversized` never fires — test enters the panic arm).
///   2. Widening the read bound to more than `cap + 1` — e.g. a typo
///      `take(cap + 2)` or `take(u64::MAX)` — which would read 20 bytes
///      into `bytes`, still trigger the reject, but with `size = 20 ≠
///      cap + 1` — the equality assert fails. A cap+1-sized file would
///      let both regressions pass the reject arm silently.
///
/// Opens the file directly (bypassing `open_bounded_regular_file`'s
/// pre-open size check) so the cap mismatch is only discovered by the
/// post-read probe — the same isolation `open_bounded_regular_file`'s
/// own tests use to reach a post-open-only detection path.
#[test]
fn rejects_oversized_during_read_via_plus1_probe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many_bytes.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"aaaaaaaaaaaaaaaaaaaa") // 20 bytes, well over cap + 1
        .unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    let cap: u64 = 8;

    match read_bounded_from_open_file(&mut file, cap) {
        Err(RejectReason::Oversized {
            size,
            cap: c,
            phase: OversizePhase::DuringRead,
        }) => {
            // cov:ignore: panic-message literal only executed on
            // assertion failure, which doesn't happen while this test
            // passes.
            assert_eq!(
                size,
                cap + 1,
                "size must be exactly cap + 1 — pins the +1 bound; \
                 a widened read (e.g. take(cap + 2)) would report size > cap + 1"
            );
            assert_eq!(c, cap);
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => {
            panic!("expected Oversized{{DuringRead}} via +1-probe post-read reject, got {other:?}")
        }
    }
}

/// Companion check: silent over-reject canary. A file of exactly `cap`
/// bytes must load successfully — the post-read check is `>` cap, not
/// `>=`, and the `take(cap + 1)` read yields exactly `cap` bytes when
/// the file is not growing.
#[test]
fn accepts_at_boundary_cap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eight_bytes.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"aaaaaaaa") // 8 bytes
        .unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    let cap: u64 = 8;

    let bytes = read_bounded_from_open_file(&mut file, cap)
        .expect("boundary-size (== cap) read must accept");
    assert_eq!(bytes, b"aaaaaaaa");
}

#[test]
fn accepts_small_file_with_u64_max_cap() {
    // Regression: `size_cap + 1` overflow when `size_cap == u64::MAX`.
    // Prior to the `saturating_add(1)` guard, debug builds panicked and
    // release builds with overflow-checks off wrapped to 0 — the latter
    // silently accepted any non-empty file as `Ok(Vec::new())` via a
    // zero-byte `take` limit.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&[1, 2, 3, 4])
        .unwrap();
    let bytes = read_bounded_regular_file(&path, u64::MAX).expect("small file under max cap");
    assert_eq!(bytes, vec![1, 2, 3, 4]);
}

#[test]
fn io_error_for_missing_file_maps_to_io_variant() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.bin");
    match read_bounded_regular_file(&missing, 128) {
        Err(RejectReason::Io(e)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Io(NotFound), got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlink_without_following() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.bin");
    std::fs::File::create(&target)
        .unwrap()
        .write_all(b"secret contents")
        .unwrap();
    let link = dir.path().join("link.bin");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    match read_bounded_regular_file(&link, 128) {
        Err(RejectReason::Symlink) => {}
        other => panic!("expected Symlink, got {other:?}"),
    }
}

/// `safe_open` must reject a symlink at open time on unix. POSIX
/// mandates ELOOP; Linux and macOS comply, but FreeBSD deliberately
/// returns EMLINK instead at every version (not a legacy-only quirk),
/// and other BSDs (NetBSD, OpenBSD) may return EMLINK or EFTYPE too.
/// The test accepts any `Err` on unix, because a passing implementation must not
/// follow the symlink regardless of the exact errno. The `Ok` arm is the
/// regression check — an implementation that dropped `O_NOFOLLOW` would
/// silently follow the link and return `Ok(file)`, failing this test.
#[cfg(unix)]
#[test]
fn safe_open_rejects_symlink_at_open_time() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.bin");
    std::fs::File::create(&target)
        .unwrap()
        .write_all(b"target contents")
        .unwrap();
    let link = dir.path().join("link.bin");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    match safe_open(&link) {
        Err(_) => {
            let post = std::fs::symlink_metadata(&link)
                .expect("symlink still present after safe_open Err");
            // cov:ignore: panic-message literal only executed on
            // assertion failure, which doesn't happen while this test
            // passes.
            assert!(
                post.file_type().is_symlink(),
                "link at test observation time was not a symlink"
            );
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        Ok(_) => panic!("safe_open followed the symlink (O_NOFOLLOW not applied)"),
    }
}

/// `open_bounded_regular_file`'s `safe_open` error-classification branch
/// (a non-`ELOOP` error, and the leaf is still a regular file at
/// recheck time) must fall through to the generic `RejectReason::Io`
/// arm rather than being misclassified as a symlink swap. `chmod 000`
/// after creation revokes read permission without touching the leaf's
/// kind: `symlink_metadata` (step 1, a stat-only syscall gated on parent
/// directory traversal, not the file's own mode bits) still succeeds,
/// but `safe_open`'s actual `open()` (step 4) then fails with
/// `EACCES` — an error that is neither `ELOOP` nor accompanied by the
/// leaf becoming a symlink, so it must reach line `return
/// Err(RejectReason::Io(e))` at the end of the `Err` arm, not the
/// symlink-swap short-circuit above it.
///
/// Tolerates running as root: root bypasses the file-mode read check
/// entirely, so `chmod 000` would not make `safe_open` fail there —
/// the `Ok` arm below accepts that outcome instead of asserting it as
/// a failure (no `unsafe` euid probe; this crate forbids `unsafe`
/// code workspace-wide, so the test observes the actual behavior
/// rather than predicting it).
#[cfg(unix)]
#[test]
fn non_eloop_open_error_on_a_still_regular_file_maps_to_io_not_symlink() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unreadable.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"contents")
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    match open_bounded_regular_file(&path, 128) {
        Err(RejectReason::Io(e)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        }
        // cov:ignore: unreachable in this workspace's non-root CI/dev
        // environment — chmod 000 always blocks open() there. Kept for
        // portability against a hypothetical root-running environment,
        // which cannot be fabricated in a test.
        Ok(_) => {
            // chmod 000 didn't block open() — running with privileges
            // (e.g. root) that bypass the file-mode read check, so this
            // test's premise doesn't hold here. Not a failure.
            eprintln!(
                "skipping non_eloop_open_error_on_a_still_regular_file_maps_to_io_not_symlink: \
                 chmod 000 did not block open() (running as root or another \
                 privileged account) — this test's premise doesn't hold here"
            );
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => panic!("expected Io(PermissionDenied) or an Ok root-bypass, got {other:?}"),
    }

    // Restore write permission so tempdir's own Drop cleanup can remove it.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// Windows counterpart of `safe_open_rejects_symlink_at_open_time`:
/// `safe_open` must not hand back a readable handle bound to a
/// symlink's target.
///
/// `std::os::windows::fs::symlink_file` requires either Developer Mode
/// or `SeCreateSymbolicLinkPrivilege` on the running account; a
/// restricted CI/sandbox account without either denies the symlink
/// creation itself (`ERROR_PRIVILEGE_NOT_HELD`) before this test can
/// exercise `safe_open` at all. That setup failure is orthogonal to the
/// property under test, so it is treated as a skip (via `eprintln!` +
/// early return) rather than a failure — this test has no way to grant
/// the privilege it does not have.
///
/// Compile-checked via `cargo check --target x86_64-pc-windows-gnu`; not
/// executed — its actual runtime behavior on Windows is unverified
/// until it runs there.
#[cfg(windows)]
#[test]
fn safe_open_rejects_symlink_at_open_time_windows() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.bin");
    std::fs::File::create(&target)
        .unwrap()
        .write_all(b"target contents")
        .unwrap();
    let link = dir.path().join("link.bin");
    if std::os::windows::fs::symlink_file(&target, &link).is_err() {
        eprintln!(
            "skipping safe_open_rejects_symlink_at_open_time_windows: \
             symlink creation requires Developer Mode or \
             SeCreateSymbolicLinkPrivilege"
        );
        return;
    }

    match safe_open(&link) {
        Err(_) => {
            let post = std::fs::symlink_metadata(&link)
                .expect("symlink still present after safe_open Err");
            assert!(
                post.file_type().is_symlink(),
                "link at test observation time was not a symlink"
            );
        }
        Ok(_) => panic!("safe_open followed the symlink (reparse-point check not applied)"),
    }
}

// ------------------------------------------------------------------
// Post-open fd-based fstat tests (FIFO/device swap TOCTOU defense).
// The FIFO/device swap window between the pre-open `symlink_metadata`
// check and `safe_open` is not deterministically reachable through
// `open_bounded_regular_file` (the pre-open `!is_file()` gate rejects a
// FIFO/device placed directly at `path` before `safe_open` ever runs).
// The defense is instead exercised at the primitive level: `safe_open` +
// `check_open_handle_regular` called directly on FIFO / char device
// paths, isolating the post-open detection path that in the composed
// pipeline only fires on a real TOCTOU race.
// ------------------------------------------------------------------

/// `check_open_handle_regular` accepts a regular file opened via
/// `safe_open` — the primitive-level happy-path regression check (also
/// pins that `O_NONBLOCK` does not break a regular-file open).
#[test]
fn check_open_handle_regular_accepts_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("regular.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"regular content")
        .unwrap();

    let file = safe_open(&path).expect("safe_open should succeed on regular file");
    check_open_handle_regular(&file).expect("regular file must pass post-open fstat");
}

/// `safe_open` + `check_open_handle_regular` rejects `/dev/null`
/// (character device). `/dev/null` opens immediately even without
/// `O_NONBLOCK` (no writer-attachment blocking like a FIFO), so this
/// exercises the fd-based fstat's non-regular-kind rejection without
/// needing `O_NONBLOCK`.
#[cfg(unix)]
#[test]
fn check_open_handle_regular_rejects_char_device() {
    let path = Path::new("/dev/null");
    let file = safe_open(path)
        .expect("safe_open on /dev/null should succeed (regular open semantics on char device)");
    match check_open_handle_regular(&file) {
        Err(RejectReason::NotRegularFilePostOpen) => {}
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => {
            panic!("expected NotRegularFilePostOpen for /dev/null post-open fstat, got {other:?}")
        }
    }
}

/// `safe_open` + `check_open_handle_regular` rejects a FIFO. Two
/// asserts check the composite defense:
///
/// 1. `safe_open` returns `Ok` (without hanging) — proves `O_NONBLOCK`
///    prevents `open()` from blocking on a writer-less FIFO.
/// 2. `check_open_handle_regular` returns
///    `Err(RejectReason::NotRegularFilePostOpen)` — proves the fd-based
///    `File::metadata()` fstat correctly identifies the FIFO kind and
///    would reject before any read from the pipe.
#[cfg(unix)]
#[test]
fn check_open_handle_regular_rejects_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("evil.bin");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo(1) should be available on unix hosts");
    assert!(status.success(), "mkfifo failed for {}", fifo.display());

    let file = safe_open(&fifo).expect(
        "safe_open on writer-less FIFO should return Ok immediately via O_NONBLOCK, \
         not block or Err — if this fails the O_NONBLOCK flag was dropped or the \
         open path regressed",
    );

    match check_open_handle_regular(&file) {
        Err(RejectReason::NotRegularFilePostOpen) => {}
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => {
            panic!("expected NotRegularFilePostOpen for FIFO post-open fstat, got {other:?}")
        }
    }
}

#[test]
fn reject_reason_display_and_source() {
    use std::error::Error;

    let symlink = RejectReason::Symlink;
    assert_eq!(symlink.to_string(), "path is a symlink");
    assert!(symlink.source().is_none());

    let nrf = RejectReason::NotRegularFile;
    assert_eq!(nrf.to_string(), "path is not a regular file");
    assert!(nrf.source().is_none());

    let nrf_post = RejectReason::NotRegularFilePostOpen;
    assert!(
        nrf_post
            .to_string()
            .contains("opened fd resolves to a non-regular file")
    );
    assert!(nrf_post.source().is_none());

    let over_pre = RejectReason::Oversized {
        size: 200,
        cap: 100,
        phase: OversizePhase::PreOpen,
    };
    assert_eq!(
        over_pre.to_string(),
        "file size 200 bytes exceeds cap 100 bytes"
    );
    assert!(over_pre.source().is_none());

    let over_during = RejectReason::Oversized {
        size: 101,
        cap: 100,
        phase: OversizePhase::DuringRead,
    };
    assert_eq!(
        over_during.to_string(),
        "file grew past cap during read: 101 bytes read, cap 100 bytes (TOCTOU-grow)"
    );
    assert!(over_during.source().is_none());

    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let io_reason = RejectReason::Io(io_err);
    assert!(io_reason.to_string().starts_with("I/O error: "));
    assert!(io_reason.source().is_some());
}

#[test]
fn read_bounded_contained_file_reads_file_inside_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let path = root.join("inside.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"inside content")
        .unwrap();

    let bytes = read_bounded_contained_file(&path, &root, 1024).expect("inside root");
    assert_eq!(bytes, b"inside content");
}

/// An intermediate directory symlinked outside the root leaves a
/// regular-file leaf, so it passes the leaf-symlink gate and must be
/// caught by the `canonicalize` containment check.
#[cfg(unix)]
#[test]
fn read_bounded_contained_file_rejects_intermediate_symlink_escape() {
    let root_tmp = tempfile::tempdir().unwrap();
    let outside_tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root_tmp.path()).unwrap();
    let outside = std::fs::canonicalize(outside_tmp.path()).unwrap();
    let outside_leaf = outside.join("leaf.bin");
    std::fs::File::create(&outside_leaf)
        .unwrap()
        .write_all(b"outside content")
        .unwrap();
    let sub_link = root_tmp.path().join("sub");
    std::os::unix::fs::symlink(&outside, &sub_link).unwrap();
    let leaf_via_intermediate = sub_link.join("leaf.bin");
    assert!(
        !std::fs::symlink_metadata(&leaf_via_intermediate)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the leaf itself must not be a symlink, or this exercises the leaf-symlink gate"
    );

    match read_bounded_contained_file(&leaf_via_intermediate, &root, 1024) {
        Err(RejectReason::PathEscape { canonical, root: r }) => {
            assert_eq!(canonical, outside_leaf);
            assert_eq!(r, root);
        }
        other => panic!("expected PathEscape, got {other:?}"),
    }
}

/// The fd-derived recheck accepts a descriptor that resolves inside the
/// root (no spurious rejection of the happy path).
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
#[test]
fn check_open_handle_containment_accepts_fd_inside_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let path = root.join("inside.bin");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"x")
        .unwrap();

    let (file, _len) = open_bounded_regular_file(&path, 1024).unwrap();
    check_open_handle_containment(&file, &root).expect("fd inside root must pass");
}

/// The fd-derived recheck rejects a descriptor that resolves outside
/// the root even though the open itself succeeded: it trusts only the
/// path bound to the fd, which is what an intermediate-directory swap
/// between the pathname checks and the open would produce.
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
#[test]
fn check_open_handle_containment_rejects_fd_outside_root() {
    let root_tmp = tempfile::tempdir().unwrap();
    let outside_tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root_tmp.path()).unwrap();
    let outside = std::fs::canonicalize(outside_tmp.path()).unwrap();
    let outside_leaf = outside.join("leaf.bin");
    std::fs::File::create(&outside_leaf)
        .unwrap()
        .write_all(b"x")
        .unwrap();

    let (file, _len) = open_bounded_regular_file(&outside_leaf, 1024).unwrap();
    match check_open_handle_containment(&file, &root) {
        Err(RejectReason::PathEscapePostOpen { canonical, root: r }) => {
            assert_eq!(canonical, outside_leaf);
            assert_eq!(r, root);
        }
        other => panic!("expected PathEscapePostOpen, got {other:?}"),
    }
}

#[test]
fn reject_reason_path_escape_display_contains_paths() {
    for reason in [
        RejectReason::PathEscape {
            canonical: std::path::PathBuf::from("/outside/leaf"),
            root: std::path::PathBuf::from("/the/root"),
        },
        RejectReason::PathEscapePostOpen {
            canonical: std::path::PathBuf::from("/outside/leaf"),
            root: std::path::PathBuf::from("/the/root"),
        },
    ] {
        let s = reason.to_string();
        assert!(s.contains("/outside/leaf"), "missing canonical: {s}");
        assert!(s.contains("/the/root"), "missing root: {s}");
        assert!(std::error::Error::source(&reason).is_none());
    }
}
