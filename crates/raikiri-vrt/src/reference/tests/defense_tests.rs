//! Defense-stack regression tests for `load_fixture` (moved from inline `mod defense_tests`).

use super::*;
use std::io::Write;

/// 1x1 valid PNG (transparent black), for happy-path fixtures where
/// we only care that bytes are read faithfully (compare_png doesn't
/// run in these tests).  Bytes from a small hand-generated PNG.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // signature
    0x00, 0x00, 0x00, 0x0d, // IHDR length
    0x49, 0x48, 0x44, 0x52, // "IHDR"
    0x00, 0x00, 0x00, 0x01, // width 1
    0x00, 0x00, 0x00, 0x01, // height 1
    0x08, 0x06, 0x00, 0x00, 0x00, // bit depth 8, color type RGBA, ...
    0x1f, 0x15, 0xc4, 0x89, // IHDR CRC
    0x00, 0x00, 0x00, 0x0a, // IDAT length
    0x49, 0x44, 0x41, 0x54, // "IDAT"
    0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, // IDAT
    0x0d, 0x0a, 0x2d, 0xb4, // IDAT CRC
    0x00, 0x00, 0x00, 0x00, // IEND length
    0x49, 0x45, 0x4e, 0x44, // "IEND"
    0xae, 0x42, 0x60, 0x82, // IEND CRC
];

fn write_input_html(dir: &Path, body: &[u8]) {
    let mut f = std::fs::File::create(dir.join("input.html")).unwrap();
    f.write_all(body).unwrap();
}

fn write_expected_png(dir: &Path, idx: usize) {
    let expected = dir.join("expected");
    std::fs::create_dir_all(&expected).unwrap();
    let path = expected.join(format!("page-{idx:04}.png"));
    std::fs::write(&path, TINY_PNG).unwrap();
}

#[test]
fn happy_path_load_succeeds_after_defense_added() {
    // Baseline: a well-formed fixture still loads through the new
    // symlink / size / containment / bounded-read stack.  If this
    // fails, the defense over-rejects legitimate fixtures.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html><html><body>ok</body></html>");
    write_expected_png(tmp.path(), 0);
    write_expected_png(tmp.path(), 1);

    let fixture = load_fixture(tmp.path()).expect("well-formed fixture must load");
    assert!(fixture.input_html.starts_with(b"<!doctype html>"));
    assert_eq!(fixture.expected_pages.len(), 2);
    assert_eq!(fixture.expected_pages[0], TINY_PNG);
    assert_eq!(fixture.expected_pages[1], TINY_PNG);
}

#[test]
fn oversized_input_html_is_rejected() {
    // TOCTOU-independent path: an up-front `metadata.len() > cap`
    // check trips before we even open the file.  Using a sparse file
    // (`set_len(cap + 1)`) so we don't actually spend 100 MiB of disk.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("input.html");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(FIXTURE_SIZE_CAP + 1).unwrap();
    drop(f);
    write_expected_png(tmp.path(), 0);

    match load_fixture(tmp.path()) {
        Err(FixtureError::OversizedFixture { path: p, size, cap }) => {
            assert_eq!(p.file_name().and_then(|f| f.to_str()), Some("input.html"));
            assert_eq!(size, FIXTURE_SIZE_CAP + 1);
            assert_eq!(cap, FIXTURE_SIZE_CAP);
        }
        other => panic!("expected OversizedFixture, got {other:?}"),
    }
}

#[test]
fn oversized_expected_png_is_rejected() {
    // Same size cap applies to expected/page-*.png entries.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    let png_path = expected.join("page-0000.png");
    let f = std::fs::File::create(&png_path).unwrap();
    f.set_len(FIXTURE_SIZE_CAP + 1).unwrap();
    drop(f);

    match load_fixture(tmp.path()) {
        Err(FixtureError::OversizedFixture { path: p, size, cap }) => {
            assert_eq!(
                p.file_name().and_then(|f| f.to_str()),
                Some("page-0000.png")
            );
            assert_eq!(size, FIXTURE_SIZE_CAP + 1);
            assert_eq!(cap, FIXTURE_SIZE_CAP);
        }
        other => panic!("expected OversizedFixture, got {other:?}"),
    }
}

#[test]
fn size_cap_boundary_is_accepted() {
    // Silent over-reject canary (mirrors a similar boundary test elsewhere): a file
    // whose size is exactly FIXTURE_SIZE_CAP must load — the check is
    // `>` cap, not `>=`.  Sparse file keeps disk usage minimal.
    let tmp = tempfile::tempdir().unwrap();
    // Small input.html so we don't spend 100 MiB there too.
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    let png_path = expected.join("page-0000.png");
    let f = std::fs::File::create(&png_path).unwrap();
    f.set_len(FIXTURE_SIZE_CAP).unwrap();
    drop(f);

    let fixture =
        load_fixture(tmp.path()).expect("boundary-size (== FIXTURE_SIZE_CAP) fixture must load");
    // Sparse file: reads back as `FIXTURE_SIZE_CAP` zero bytes.  Verify
    // length only (allocating a 100 MiB assertion buffer just to
    // compare would double the memory footprint of the test needlessly).
    assert_eq!(fixture.expected_pages.len(), 1);
    assert_eq!(fixture.expected_pages[0].len() as u64, FIXTURE_SIZE_CAP);
}

// `read_bounded_from_open_file`'s `+1-probe` post-read length gate (the
// exact `cap + 1` bound, and the boundary-cap accept case) is now a
// primitive owned by `raikiri_traits::io` — see that crate's
// `rejects_oversized_during_read_via_plus1_probe` and
// `accepts_at_boundary_cap` tests. `map_reject_reason` above collapses
// both `RejectReason::Oversized` phases onto this crate's single
// `FixtureError::OversizedFixture` variant (which never distinguished
// the two phases to begin with).

#[test]
fn expected_dir_as_plain_file_is_ignored_not_read() {
    // If `expected/` is a plain regular file (not a dir), we treat it
    // the same as "no expected/ at all" — no pages to enumerate, no
    // read attempted on it.  This documents the fallback: the
    // filesystem type check is up-front, before any expensive read.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    std::fs::write(tmp.path().join("expected"), b"not a directory").unwrap();

    let fixture =
        load_fixture(tmp.path()).expect("fixture with plain-file `expected` should still load");
    assert!(fixture.expected_pages.is_empty());
}

#[test]
#[cfg(unix)]
fn symlink_input_html_to_dev_zero_is_rejected() {
    // Core threat: attacker replaces `input.html` with a symlink to
    // `/dev/zero`.  Old `std::fs::read` would follow it and read
    // forever, exhausting memory.  New defense: `symlink_metadata` +
    // `is_symlink()` rejects at the leaf, before any read.
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("input.html");
    std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
    write_expected_png(tmp.path(), 0);

    match load_fixture(tmp.path()) {
        Err(FixtureError::SymlinkRejected { path }) => {
            assert_eq!(
                path.file_name().and_then(|f| f.to_str()),
                Some("input.html")
            );
        }
        other => panic!("expected SymlinkRejected for /dev/zero symlink, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn symlink_expected_png_to_sensitive_file_is_rejected() {
    // Second-file variant: attacker replaces a page PNG with a symlink
    // to a sensitive local file.  Same leaf-symlink reject applies.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    // Target need only exist; use /etc/hostname (readable on virtually
    // any unix host, small, non-sensitive to actually leak in a test).
    std::os::unix::fs::symlink("/etc/hostname", expected.join("page-0000.png")).unwrap();

    match load_fixture(tmp.path()) {
        Err(FixtureError::SymlinkRejected { path }) => {
            assert_eq!(
                path.file_name().and_then(|f| f.to_str()),
                Some("page-0000.png")
            );
        }
        other => panic!("expected SymlinkRejected for expected/ png symlink, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn path_traversal_via_expected_symlink_is_rejected() {
    // `expected/` itself is a symlink pointing outside the fixture
    // root.  The `expected/` pre-check catches this as
    // SymlinkRejected before we even try to enumerate.  Per the
    // advisor: don't lock in a specific variant here — the important
    // property is "not accepted"; any of the three
    // security-related variants (SymlinkRejected / NotRegularFile /
    // PathEscape) is a valid rejection.
    let tmp_fixture = tempfile::tempdir().unwrap();
    let tmp_outside = tempfile::tempdir().unwrap();
    write_input_html(tmp_fixture.path(), b"<!doctype html>");
    // Populate outside/ with a PNG so if the reject didn't fire we'd
    // actually escape and read the outside file.
    std::fs::write(tmp_outside.path().join("page-0000.png"), TINY_PNG).unwrap();
    std::os::unix::fs::symlink(tmp_outside.path(), tmp_fixture.path().join("expected")).unwrap();

    match load_fixture(tmp_fixture.path()) {
        Err(FixtureError::SymlinkRejected { .. })
        | Err(FixtureError::NotRegularFile { .. })
        | Err(FixtureError::PathEscape { .. }) => {
            // any of the containment gates firing is a correct reject
        }
        other => {
            panic!("expected traversal reject (Symlink/NotRegular/PathEscape), got {other:?}")
        }
    }
}

/// Run `load_fixture(dir)` on a worker thread and fail-fast on timeout.
/// Prevents FIFO-regression tests from silently hanging the test suite
/// forever if the `!is_file()` gate ever regresses — in that case
/// `File::open`/`read_to_end` would block on the FIFO with no writer.
#[cfg(unix)]
fn load_fixture_with_watchdog(
    dir: PathBuf,
    timeout: std::time::Duration,
) -> Result<Fixture, FixtureError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(load_fixture(&dir));
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => {
            // Best-effort join; thread has already sent its result.
            let _ = handle.join();
            result
        }
        Err(_) => panic!(
            "load_fixture did not return within {:?} — FIFO gate has likely regressed \
                 (File::open on a FIFO with no writer blocks indefinitely). \
                 Leaving worker thread detached so the suite fails fast rather than hanging.",
            timeout
        ),
    }
}

#[test]
#[cfg(unix)]
fn fifo_as_input_html_is_rejected_not_blocked() {
    // If an attacker places `input.html` as a FIFO with no writer,
    // `File::open` + `read_to_end` blocks indefinitely.  The `!is_file()`
    // gate rejects the FIFO up-front so the test process cannot hang.
    // A watchdog wrapper converts a regression from "test hangs forever"
    // into "test panics after 2 s with a clear message".
    let tmp = tempfile::tempdir().unwrap();
    let fifo = tmp.path().join("input.html");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo(1) should be available on unix hosts");
    assert!(status.success(), "mkfifo failed for {}", fifo.display());
    write_expected_png(tmp.path(), 0);

    match load_fixture_with_watchdog(tmp.path().to_path_buf(), std::time::Duration::from_secs(2)) {
        Err(FixtureError::NotRegularFile { path }) => {
            assert_eq!(
                path.file_name().and_then(|f| f.to_str()),
                Some("input.html")
            );
        }
        other => panic!("expected NotRegularFile for FIFO input.html, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn fifo_at_expected_png_is_rejected_not_blocked() {
    // Parallel to `fifo_as_input_html_is_rejected_not_blocked`, but
    // pins the `!is_file()` gate for the second consumer of
    // `read_bounded_fixture_file` — the `expected/page-*.png`
    // enumeration path.  Same watchdog + same regression semantics:
    // a gate regression here would `File::open` the FIFO and block.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    let fifo = expected.join("page-0000.png");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo(1) should be available on unix hosts");
    assert!(status.success(), "mkfifo failed for {}", fifo.display());

    match load_fixture_with_watchdog(tmp.path().to_path_buf(), std::time::Duration::from_secs(2)) {
        Err(FixtureError::NotRegularFile { path }) => {
            assert_eq!(
                path.file_name().and_then(|f| f.to_str()),
                Some("page-0000.png")
            );
        }
        other => panic!("expected NotRegularFile for FIFO page-0000.png, got {other:?}"),
    }
}

#[test]
fn zero_byte_input_html_and_expected_png_load_successfully() {
    // Boundary canary at the low end: a zero-byte file should load
    // (`.take(cap + 1).read_to_end` yields 0 bytes; `is_file()` still
    // true; size 0 ≤ cap).  Pins current behavior so a future hygiene
    // check (`if metadata.len() == 0 { reject }`) doesn't silently
    // change fixture semantics.  Cheap counterpart to
    // `size_cap_boundary_is_accepted` at the high end.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    std::fs::write(expected.join("page-0000.png"), b"").unwrap();

    let fixture =
        load_fixture(tmp.path()).expect("zero-byte input.html + zero-byte page should load");
    assert!(fixture.input_html.is_empty());
    assert_eq!(fixture.expected_pages.len(), 1);
    assert!(fixture.expected_pages[0].is_empty());
}

#[test]
#[cfg(unix)]
fn fixture_root_as_symlink_is_rejected() {
    // Codex gate final review finding: `canonicalize(fixture_dir)`
    // silently follows a symlinked root, redirecting the containment
    // anchor to the target directory.  The declared blanket policy
    // ("symlinks anywhere in the fixture tree are refused") must
    // include the root itself.  Pre-check via `symlink_metadata` on
    // the fixture root rejects with SymlinkRejected before
    // canonicalize sees it.
    let tmp_real = tempfile::tempdir().unwrap();
    write_input_html(tmp_real.path(), b"<!doctype html>");
    write_expected_png(tmp_real.path(), 0);

    let tmp_parent = tempfile::tempdir().unwrap();
    let link_root = tmp_parent.path().join("root-link");
    std::os::unix::fs::symlink(tmp_real.path(), &link_root).unwrap();

    match load_fixture(&link_root) {
        Err(FixtureError::SymlinkRejected { path }) => {
            assert_eq!(path, link_root);
        }
        other => panic!("expected SymlinkRejected for symlinked root, got {other:?}"),
    }
}

#[test]
fn duplicate_page_indices_via_noncanonical_names_are_ignored() {
    // Codex gate final review finding: `page-0.png`, `page-00.png`,
    // `page-0000.png`, `page-00000.png` all parse to index 0.  Before
    // the canonical-form filter, an attacker could pack N copies with
    // different padding widths; each would pass per-file caps and be
    // fully read into `numbered` before duplicate detection surfaced
    // as NonContiguousPages — multiplying the per-file cap by the
    // padding-width space.  Fix: require exactly 4 zero-padded digits
    // in the enumeration filter; noncanonical forms are skipped
    // (never opened, never read).  Test: place `page-0000.png` +
    // several noncanonical duplicates; load should succeed with
    // exactly one page (the canonical one).
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    write_expected_png(tmp.path(), 0);
    let expected = tmp.path().join("expected");
    // Noncanonical padding widths — all would parse to n=0.
    // Populate with plausibly-huge fake bodies so a regression
    // (reading them) would be easy to spot in mem accounting.
    for name in ["page-0.png", "page-00.png", "page-00000.png"] {
        std::fs::write(expected.join(name), vec![0u8; 8]).unwrap();
    }
    // Non-digit padding — should also be skipped.
    std::fs::write(expected.join("page-000a.png"), vec![0u8; 8]).unwrap();

    let fixture = load_fixture(tmp.path())
        .expect("canonical page-0000.png with noncanonical siblings should load");
    assert_eq!(
        fixture.expected_pages.len(),
        1,
        "only the canonical page should enumerate"
    );
    assert_eq!(fixture.expected_pages[0], TINY_PNG);
}

#[test]
fn too_many_expected_pages_is_rejected_before_read() {
    // Codex gate final review finding: aggregate memory is
    // unbounded — an attacker packing `expected/` with many valid
    // regular files, each ≤ FIXTURE_SIZE_CAP, can OOM even with the
    // per-file cap.  Fix: MAX_EXPECTED_PAGES aggregate count cap
    // checked before the (cap+1)th page is read.
    //
    // Place MAX_EXPECTED_PAGES + 1 canonical zero-byte pages: the
    // walker enumerates them all (no I/O for content yet — just
    // `read_dir`), each canonical name passes the syntactic filter,
    // and the cap check `numbered.len() >= MAX_EXPECTED_PAGES` fires
    // when the loop attempts to read the (cap+1)th entry.  Zero-byte
    // files keep disk + total test cost minimal.
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    for n in 0..=MAX_EXPECTED_PAGES {
        std::fs::write(expected.join(format!("page-{n:04}.png")), b"").unwrap();
    }

    match load_fixture(tmp.path()) {
        Err(FixtureError::TooManyExpectedPages { fixture_dir, cap }) => {
            assert_eq!(fixture_dir, tmp.path());
            assert_eq!(cap, MAX_EXPECTED_PAGES);
        }
        other => panic!("expected TooManyExpectedPages, got {other:?}"),
    }
}

#[test]
fn oversized_fixture_aggregate_is_rejected() {
    // Codex gate final review round 2: the count cap alone still
    // admits ~102 GiB (MAX_EXPECTED_PAGES × FIXTURE_SIZE_CAP).  The
    // load-bearing aggregate defense is FIXTURE_AGGREGATE_BYTES_CAP.
    // Verify a running-total-exceeded fixture rejects with the
    // dedicated variant.
    //
    // Setup: 3 sparse files at 100 MiB each (well under per-file
    // cap).  Aggregate = 300 MiB > 256 MiB cap → OversizedAggregate
    // on the 3rd file's read.  Sparse `set_len` keeps disk usage
    // trivial; `read_to_end` materializes the zeros (~300 MiB peak
    // in this test's RAM — acceptable for a security regression
    // that would otherwise let an attacker exhaust orders of
    // magnitude more).
    const PAGE_SIZE: u64 = 100 * 1024 * 1024;
    let tmp = tempfile::tempdir().unwrap();
    write_input_html(tmp.path(), b"<!doctype html>");
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    for n in 0..=2u32 {
        let path = expected.join(format!("page-{n:04}.png"));
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(PAGE_SIZE).unwrap();
        drop(f);
    }
    // Sanity: our 3 files × 100 MiB = 300 MiB > 256 MiB cap.
    // Compile-time to avoid clippy::assertions_on_constants.
    const _: () = assert!(3 * PAGE_SIZE > FIXTURE_AGGREGATE_BYTES_CAP);

    match load_fixture(tmp.path()) {
        Err(FixtureError::OversizedFixtureAggregate {
            fixture_dir,
            total,
            cap,
        }) => {
            assert_eq!(fixture_dir, tmp.path());
            assert_eq!(cap, FIXTURE_AGGREGATE_BYTES_CAP);
            // Total tallied through the (rejected-on-add) 3rd page.
            assert!(
                total > FIXTURE_AGGREGATE_BYTES_CAP,
                "total {total} should exceed cap {cap}"
            );
        }
        other => panic!("expected OversizedFixtureAggregate, got {other:?}"),
    }
}

/// End-to-end check: read_bounded_fixture_file rejects a symlink at the
/// pre-open `symlink_metadata` check.  This test does NOT exercise the
/// `O_NOFOLLOW` path (the open never runs because the pre-open check
/// short-circuits) — that unit is covered by `raikiri_traits::io`'s own
/// `safe_open_rejects_symlink_at_open_time` test.  Kept to check the
/// full-path behavior against future refactors that might reorder the
/// checks.
#[cfg(unix)]
#[test]
fn read_bounded_fixture_file_rejects_symlink_via_pre_open_check() {
    let dir = tempfile::tempdir().unwrap();
    let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
    let target = dir.path().join("target.bin");
    std::fs::File::create(&target)
        .unwrap()
        .write_all(b"target")
        .unwrap();
    let link = dir.path().join("link.bin");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    match read_bounded_fixture_file(&link, &canonical_root) {
        Err(FixtureError::SymlinkRejected { path }) => {
            assert_eq!(path, link);
        }
        other => panic!("expected SymlinkRejected, got {other:?}"),
    }
}

/// Regression check: O_NOFOLLOW on the internal open path does not reject a
/// legitimate regular file.  Without this test, an implementation that
/// broke the safe_open fallback (e.g. accidentally always returning
/// ELOOP) would be missed by the symlink-only tests.
#[test]
fn read_bounded_fixture_file_accepts_regular_file_with_nofollow() {
    let dir = tempfile::tempdir().unwrap();
    let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
    let file_path = dir.path().join("regular.bin");
    std::fs::File::create(&file_path)
        .unwrap()
        .write_all(b"regular content")
        .unwrap();

    let bytes = read_bounded_fixture_file(&file_path, &canonical_root)
        .expect("regular file should be accepted");
    assert_eq!(bytes, b"regular content");
}

/// `map_reject_reason` translates every `raikiri_traits::io::RejectReason`
/// variant to its `FixtureError` counterpart, attaching `path`.
/// `NotRegularFilePostOpen` and `Oversized { phase: DuringRead }` are
/// only reachable through `read_bounded_fixture_file` via a genuine
/// TOCTOU race, so this is the sole deterministic check that a future
/// match reorder can't silently reroute a race-detected reject into the
/// wildcard `IoError` arm (every `FixtureError` this crate produces is
/// `?`-propagated — there is no warn+skip path to fall back to).
#[test]
fn map_reject_reason_maps_all_variants() {
    use raikiri_traits::io::{OversizePhase, RejectReason};

    let path = Path::new("/tmp/fake.bin");

    assert!(matches!(
        map_reject_reason(path, RejectReason::Symlink),
        FixtureError::SymlinkRejected { path: p } if p == path
    ));
    assert!(matches!(
        map_reject_reason(path, RejectReason::NotRegularFile),
        FixtureError::NotRegularFile { path: p } if p == path
    ));
    assert!(matches!(
        map_reject_reason(path, RejectReason::NotRegularFilePostOpen),
        FixtureError::NotRegularFilePostOpen { path: p } if p == path
    ));
    assert!(matches!(
        map_reject_reason(
            path,
            RejectReason::Oversized {
                size: 42,
                cap: 10,
                phase: OversizePhase::PreOpen,
            }
        ),
        FixtureError::OversizedFixture { path: p, size: 42, cap: 10 } if p == path
    ));
    assert!(matches!(
        map_reject_reason(
            path,
            RejectReason::Oversized {
                size: 101,
                cap: 100,
                phase: OversizePhase::DuringRead,
            }
        ),
        FixtureError::OversizedFixture { path: p, size: 101, cap: 100 } if p == path
    ));
    match map_reject_reason(
        path,
        RejectReason::PathEscape {
            canonical: PathBuf::from("/outside/a.png"),
            root: PathBuf::from("/fixture"),
        },
    ) {
        FixtureError::PathEscape { canonical, root } => {
            assert_eq!(canonical, Path::new("/outside/a.png"));
            assert_eq!(root, Path::new("/fixture"));
        }
        other => panic!("expected PathEscape, got {other:?}"),
    }
    match map_reject_reason(
        path,
        RejectReason::PathEscapePostOpen {
            canonical: PathBuf::from("/outside/b.png"),
            root: PathBuf::from("/fixture"),
        },
    ) {
        FixtureError::PathEscapePostOpen { canonical, root } => {
            assert_eq!(canonical, Path::new("/outside/b.png"));
            assert_eq!(root, Path::new("/fixture"));
        }
        other => panic!("expected PathEscapePostOpen, got {other:?}"),
    }
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    match map_reject_reason(path, RejectReason::Io(io_err)) {
        FixtureError::IoError { path: p, source } => {
            assert_eq!(p, path);
            assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => panic!("expected IoError, got {other:?}"),
    }
}

/// **Race regression**: races a writer against a reader across the exact TOCTOU window
/// `safe_open` closes — the pre-open `symlink_metadata` check vs. the
/// real open — and asserts the reader can never observe the symlink
/// target's ("evil") contents.
///
/// ## Design
///
/// A writer thread atomically swaps `target` back and forth between a
/// regular file and a symlink to `evil.bin`, both swaps done via
/// `rename(2)` of a pre-built source onto `target`. `rename` replaces
/// the destination in one syscall regardless of the entry's prior
/// type, so there is no "missing file" window to conflate with the
/// race under test — unlike the `rm target; symlink(evil, target)` (or
/// `rm target; File::create(target)`) sequence the issue sketches,
/// which opens an ENOENT gap that races something else (a plain
/// not-found error) rather than the metadata-vs-open TOCTOU this test
/// targets. A reader thread concurrently calls
/// `read_bounded_fixture_file(target, canonical_root)` in a tight loop.
///
/// `evil.bin` is placed **inside** `canonical_root`, deliberately — if
/// it lived outside the fixture root, a weakened build (no
/// `O_NOFOLLOW`) would still get caught by the *unrelated* `PathEscape`
/// containment check (a successful, symlink-following open would still
/// have its resolved target caught by the subsequent `canonicalize`
/// check), and this test would pass even with the
/// `O_NOFOLLOW` defense removed — zero detection power. Keeping
/// `evil.bin` in-root routes execution through `safe_open`, the one
/// line this test exists to check. **Do not relocate `evil.bin` outside
/// the tmp root** — that silently neuters the test.
///
/// ## Assertion
///
/// `Ok(bytes)` must always equal `REGULAR_CONTENTS` exactly;
/// `Ok(bytes) == EVIL_CONTENTS` is an immediate hard failure — the
/// property this test exists to guard. Any `Err` is accepted: by
/// design, the pre-open `symlink_metadata` rejection and the
/// open-time `O_NOFOLLOW` rejection both map to the same
/// `SymlinkRejected` variant (intentionally indistinguishable to the
/// caller), so this test does not attempt to prove which of the two
/// gates fired on a given iteration — only that neither ever leaks
/// `evil.bin`'s contents.
///
/// After the loop, `ok_regular > 0 && symlink_rejected > 0` proves the
/// writer was actually observed in both states (guards against a
/// vacuous pass where the writer thread never got scheduled, or the
/// target never actually changed type). It does **not** prove the
/// tight metadata-says-regular-then-open-sees-symlink window was hit —
/// that window is invisible from outside the function under test.
/// `symlink_rejected` fires overwhelmingly from the cheaper pre-open
/// check, since the target is a symlink for roughly half of
/// wall-clock time.
///
/// ## Flakiness / gating
///
/// Timing-dependent by nature (racing OS thread scheduling against a
/// rename is the whole point), so this is `#[ignore]`d to keep `cargo
/// test --workspace` deterministic. `#[ignore]` (rather than an
/// env-var early-return) is deliberate: an env-var gate that
/// early-returns success on a skipped run reports the test as green
/// without it ever having run — a silent-pass hazard. `#[ignore]`
/// reports "ignored" in the default run, which is honest signal that
/// distinguishes "ran and passed" from "didn't run".
///
/// Run manually:
///
/// ```text
/// cargo test -p raikiri-vrt --lib -- --ignored concurrent_leaf_swap_never_yields_evil_contents
/// ```
///
/// `ITERATIONS` was sized empirically: with `safe_open`'s `O_NOFOLLOW`
/// temporarily reverted to a plain `File::open`,
/// 8 repeated runs observed the first `Ok(evil contents)` at iteration
/// 6, 7, 7, 8, 8, 49, 82, and 171 — worst case 171. `ITERATIONS = 20_000`
/// here is ~100x that worst-observed margin, so a regression
/// reintroducing the bug reliably trips this test rather than getting
/// lucky.
#[cfg(unix)]
#[test]
#[ignore = "timing-dependent race simulation; run manually: \
                cargo test -p raikiri-vrt --lib -- --ignored \
                concurrent_leaf_swap_never_yields_evil_contents"]
fn concurrent_leaf_swap_never_yields_evil_contents() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const REGULAR_CONTENTS: &[u8] = b"regular contents";
    const EVIL_CONTENTS: &[u8] = b"evil contents";
    const ITERATIONS: usize = 20_000;

    let dir = tempfile::tempdir().unwrap();
    let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
    let target = dir.path().join("target.bin");
    // Deliberately inside canonical_root — see "Design" above. Moving
    // this outside the root would make the containment check (not
    // O_NOFOLLOW) the thing that rejects the race, neutering the test.
    let evil = dir.path().join("evil.bin");
    std::fs::write(&evil, EVIL_CONTENTS).unwrap();

    // Two swap sources the writer alternately `rename`s onto `target`.
    // Each is recreated after being consumed by a `rename` (rename
    // moves the source; it doesn't copy it).
    let regular_src = dir.path().join("regular_src.bin");
    let evil_link_src = dir.path().join("evil_link_src.bin");
    std::fs::write(&regular_src, REGULAR_CONTENTS).unwrap();
    std::os::unix::fs::symlink(&evil, &evil_link_src).unwrap();
    // Seed `target` so the reader always has something to open.
    std::fs::rename(&regular_src, &target).unwrap();
    std::fs::write(&regular_src, REGULAR_CONTENTS).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let writer_target = target.clone();
    let writer_evil = evil.clone();
    let writer_regular_src = regular_src.clone();
    let writer_evil_link_src = evil_link_src.clone();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        while !writer_stop.load(Ordering::Relaxed) {
            std::fs::rename(&writer_regular_src, &writer_target)?;
            std::fs::write(&writer_regular_src, REGULAR_CONTENTS)?;

            std::fs::rename(&writer_evil_link_src, &writer_target)?;
            std::os::unix::fs::symlink(&writer_evil, &writer_evil_link_src)?;
        }
        Ok(())
    });

    // The reader loop runs inside `catch_unwind` so that a failing
    // assertion (the exact case this test exists to catch) still lets
    // us signal `stop` and join the writer below, rather than leaking
    // a spinning background thread into the rest of the test binary's
    // process if this test is ever run non-isolated. The original
    // panic (with its diagnostic message) is re-raised via
    // `resume_unwind` afterward — this is bookkeeping around the
    // panic, not suppression of it.
    let loop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut ok_regular = 0usize;
        let mut symlink_rejected = 0usize;
        let mut other_err = 0usize;
        for i in 0..ITERATIONS {
            match read_bounded_fixture_file(&target, &canonical_root) {
                Ok(bytes) => {
                    assert_ne!(
                        bytes, EVIL_CONTENTS,
                        "SECURITY REGRESSION at iteration {i}: reader observed the \
                             symlink target's contents through the leaf-swap race \
                             (O_NOFOLLOW / pre-open check both failed to close the \
                             TOCTOU window)"
                    );
                    assert_eq!(
                        bytes, REGULAR_CONTENTS,
                        "iteration {i}: reader returned Ok with unexpected contents: {bytes:?}"
                    );
                    ok_regular += 1;
                }
                Err(FixtureError::SymlinkRejected { .. }) => symlink_rejected += 1,
                Err(_) => other_err += 1,
            }
        }
        (ok_regular, symlink_rejected, other_err)
    }));

    stop.store(true, Ordering::Relaxed);
    let writer_result = writer.join();

    let (ok_regular, symlink_rejected, other_err) = match loop_result {
        Ok(counts) => counts,
        Err(payload) => std::panic::resume_unwind(payload),
    };
    writer_result
        .expect("writer thread panicked")
        .expect("writer thread hit an unexpected IO error");

    assert!(
        ok_regular > 0,
        "reader never observed the regular-file state — race not exercised"
    );
    assert!(
        symlink_rejected > 0,
        "reader never observed the symlink state — writer thread may not have \
             been scheduled, or the swap never actually raced the reader"
    );
    eprintln!(
        "concurrent_leaf_swap_never_yields_evil_contents: {ITERATIONS} iterations \
             — ok_regular={ok_regular} symlink_rejected={symlink_rejected} other_err={other_err}"
    );
}
