//! Bounded regular-file read primitive.
//!
//! Shared defense stack for filesystem input taken from consumer-controlled
//! locations (WPT fixture tree, WPT bundled font dir).  Intended to consolidate
//! the two prior hand-rolls at `raikiri_dom::fonts::build_wpt_font_ctx`
//! and `raikiri_vrt::reference::load_fixture`
//! via the `+1-probe` TOCTOU-grow pattern.
//!
//! **Migration status**: raikiri-dom side is migrated; raikiri-vrt side
//! is deferred (walls.md §2 crate-list implications —
//! raikiri-vrt has never depended on raikiri-traits, adding the dep would
//! implicitly extend the wall/traits implementor list).  Until that lands the
//! raikiri-vrt hand-roll and this helper coexist with the same defense shape.
//!
//! Downstream-crate paths are shown as plain code (not intra-doc links)
//! because raikiri-dom / raikiri-vrt are not in scope from this crate.
//!
//! Callers layer domain-specific checks (path containment for the fixture
//! tree, platform-specific hardening like `O_NOFOLLOW`) on top of this
//! primitive.

use std::io::Read;
use std::path::Path;

/// Reason a regular-file read was rejected.
///
/// Callers map each variant to their domain-specific policy: propagate as a
/// hard error, or log-and-skip when a partial input is acceptable.
#[non_exhaustive]
#[derive(Debug)]
pub enum RejectReason {
    /// `symlink_metadata().file_type().is_symlink()` returned true.
    ///
    /// The read never followed the link.  Callers refusing to follow symlinks
    /// (fixture-tree, font dir) map this to their symlink-rejected error.
    Symlink,
    /// Path is neither a regular file nor a symlink (device, fifo, socket,
    /// directory, block/char device).
    ///
    /// `File::open` on a fifo with no writer blocks indefinitely, and
    /// `metadata.len()` reports 0 for devices — the size cap alone cannot
    /// close these vectors.  Rejecting at metadata time is the load-bearing
    /// defense against time-axis DoS.
    NotRegularFile,
    /// File size exceeds `size_cap`.  `phase` records whether the pre-open
    /// `metadata.len()` check tripped (PreOpen) or the post-read `+1-probe`
    /// tripped (DuringRead — TOCTOU-grow race between metadata and read).
    ///
    /// A single variant (rather than two peers) keeps the size-exceeded
    /// invariant one-source-of-truth for consumers: forgetting a match arm
    /// on a new phase cannot silently accept an oversized read.  The bound
    /// holds even on the reject path — at most `size_cap + 1` bytes are read
    /// before DuringRead surfaces.
    Oversized {
        /// Reported bytes.  `PreOpen`: `metadata.len()`.  `DuringRead`:
        /// `bytes.len() as u64` (always strictly `> cap`).
        size: u64,
        /// Configured cap in bytes.
        cap: u64,
        /// Phase in which the size-cap check tripped.
        phase: OversizePhase,
    },
    /// I/O error from `symlink_metadata`, `File::open`, or `read_to_end`.
    Io(std::io::Error),
}

/// Phase in which [`RejectReason::Oversized`] was detected.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OversizePhase {
    /// `metadata.len() > size_cap` at the pre-open check.  The file was
    /// never opened.
    PreOpen,
    /// The file grew past `size_cap` between the metadata check and the
    /// bounded read (TOCTOU-grow race), caught by the `+1-probe` pattern:
    /// `take(size_cap + 1)` then post-read `bytes.len() > size_cap`.
    DuringRead,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::Symlink => write!(f, "path is a symlink"),
            RejectReason::NotRegularFile => write!(f, "path is not a regular file"),
            RejectReason::Oversized {
                size,
                cap,
                phase: OversizePhase::PreOpen,
            } => write!(f, "file size {size} bytes exceeds cap {cap} bytes"),
            RejectReason::Oversized {
                size,
                cap,
                phase: OversizePhase::DuringRead,
            } => write!(
                f,
                "file grew past cap during read: {size} bytes read, cap {cap} bytes (TOCTOU-grow)"
            ),
            RejectReason::Io(source) => write!(f, "I/O error: {source}"),
        }
    }
}

impl std::error::Error for RejectReason {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RejectReason::Io(source) => Some(source),
            _ => None,
        }
    }
}

/// Read a regular file with a defense-in-depth bounded read.
///
/// The check order is:
///
/// 1. `symlink_metadata` — refuse to follow symlinks on this call frame.
/// 2. `file_type().is_file()` — reject device/fifo/socket/directory before
///    opening (load-bearing against `mkfifo`-style time DoS).
/// 3. `metadata.len() > size_cap` — reject oversized files up front.
/// 4. `File::open + take(size_cap + 1) + read_to_end` — bounded read.
/// 5. `bytes.len() > size_cap` — reject TOCTOU-grown files that expanded
///    between step 3 and step 4.
///
/// The `+1-probe` (step 4) is what makes silent truncation impossible: a file
/// that grew past `size_cap` after step 3 yields at least `size_cap + 1` bytes
/// and trips step 5 instead of returning a truncated buffer.
///
/// # Not covered
///
/// - **Path containment**: callers that need to bound the resolved path to a
///   canonical root (fixture-tree) must add `canonicalize().starts_with(root)`
///   on top.
/// - **Leaf-swap TOCTOU between `symlink_metadata` and `File::open`**: an
///   attacker with concurrent-write access to the path can swap a regular
///   file for a symlink between step 1 and step 4.  Callers needing this
///   defense must use `O_NOFOLLOW` (unix) or an inode-verify-after-open
///   pattern.  Not yet implemented here.
/// - **Aggregate caps across multiple files**: per-file only.  Callers doing
///   directory-walk aggregation (fixture-tree pages) must track running
///   totals themselves.
pub fn read_bounded_regular_file(path: &Path, size_cap: u64) -> Result<Vec<u8>, RejectReason> {
    let metadata = std::fs::symlink_metadata(path).map_err(RejectReason::Io)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(RejectReason::Symlink);
    }
    if !file_type.is_file() {
        return Err(RejectReason::NotRegularFile);
    }
    if metadata.len() > size_cap {
        return Err(RejectReason::Oversized {
            size: metadata.len(),
            cap: size_cap,
            phase: OversizePhase::PreOpen,
        });
    }

    let mut file = std::fs::File::open(path).map_err(RejectReason::Io)?;
    // Preallocate against the known-good `metadata.len()` upper bound to
    // avoid the log2(N) reallocations `Vec::new() + read_to_end` would incur
    // on a large happy-path file.  The `+1-probe` may push one byte past the
    // reservation on the TOCTOU-grow reject path — that is one memcpy, not
    // a full growth chain.
    let mut bytes = Vec::with_capacity(std::cmp::min(metadata.len(), size_cap) as usize);
    // `saturating_add(1)` avoids `size_cap + 1` overflow when a caller passes
    // `u64::MAX` as the cap.  With overflow the read limit would wrap to 0
    // and any non-empty file would be silently accepted as empty bytes; the
    // saturation keeps the bound at `u64::MAX` (effectively unlimited) so a
    // small file still reads through and the post-check below cannot
    // spuriously fire.
    let read_limit = size_cap.saturating_add(1);
    file.by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(RejectReason::Io)?;
    if bytes.len() as u64 > size_cap {
        return Err(RejectReason::Oversized {
            size: bytes.len() as u64,
            cap: size_cap,
            phase: OversizePhase::DuringRead,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
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

    // fifo/device rejection would exercise `NotRegularFile` on the time-DoS
    // vector (`mkfifo`), but adding a libc/nix dev-dep here just for the test
    // is out of proportion for a leaf helper.  `rejects_directory_as_not_regular_file`
    // already exercises the same `!file_type.is_file()` branch structurally;
    // fifo-specific coverage lives in the consumer crates (raikiri-dom fonts,
    // raikiri-vrt reference) which already depend on tempfile + unix APIs.

    #[test]
    fn reject_reason_display_and_source() {
        use std::error::Error;

        let symlink = RejectReason::Symlink;
        assert_eq!(symlink.to_string(), "path is a symlink");
        assert!(symlink.source().is_none());

        let nrf = RejectReason::NotRegularFile;
        assert_eq!(nrf.to_string(), "path is not a regular file");
        assert!(nrf.source().is_none());

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
}
