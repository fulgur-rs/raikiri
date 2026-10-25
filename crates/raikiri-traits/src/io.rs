//! Bounded regular-file read primitives with a leaf-swap TOCTOU defense
//! stack.
//!
//! Shared defense stack for filesystem input taken from consumer-controlled
//! locations (WPT fixture tree, WPT bundled font dir).
//!
//! - [`open_bounded_regular_file`] + [`read_bounded_from_open_file`]: the
//!   leaf-swap defense (pre-open kind and size gates, `O_NOFOLLOW` open,
//!   post-open fd kind recheck) and the `+1-probe` bounded read.
//! - [`read_bounded_regular_file`]: both of the above in one call, for a
//!   caller with no path-containment requirement.
//! - [`read_bounded_contained_file`]: the same, plus path containment under
//!   a canonical root, including a post-open recheck derived from the
//!   opened descriptor itself (`/proc/self/fd/<fd>` readlink on Linux,
//!   `fcntl(fd, F_GETPATH, ..)` on Apple platforms) so the checked file and
//!   the read file are the same fd.
//!
//! # What this module does not cover
//!
//! - **Aggregate caps across multiple files**: per-file only.  Callers doing
//!   directory-walk aggregation (fixture-tree pages, font directories) must
//!   track running totals themselves.

use std::io::Read;
use std::path::Path;

/// Reason a regular-file read was rejected.
///
/// Callers map each variant to their domain-specific policy: propagate as a
/// hard error, or log-and-skip when a partial input is acceptable.
#[non_exhaustive]
#[derive(Debug)]
#[doc(hidden)]
pub enum RejectReason {
    /// `symlink_metadata().file_type().is_symlink()` returned `true` at the
    /// pre-open check, **or** the open-time leaf-swap defense in
    /// `open_bounded_regular_file` (`O_NOFOLLOW` on unix, a
    /// reparse-point-aware open on Windows) refused a symlink leaf that was
    /// swapped in after the pre-open check ran.  Both detection points map
    /// to the same variant: the caller-facing contract is "this path was
    /// rejected because it is a symlink", independent of *when* that was
    /// discovered, and the read never followed the link either way.
    Symlink,
    /// Path is neither a regular file nor a symlink (device, fifo, socket,
    /// directory, block/char device), observed at the pre-open
    /// `symlink_metadata` check.
    ///
    /// `File::open` on a fifo with no writer blocks indefinitely, and
    /// `metadata.len()` reports 0 for devices — the size cap alone cannot
    /// close these vectors.  Rejecting at metadata time is the load-bearing
    /// defense against time-axis DoS.
    NotRegularFile,
    /// The descriptor `open_bounded_regular_file` actually opened does not
    /// resolve to a regular file (FIFO / character device / block device
    /// swapped in between the pre-open `symlink_metadata` check and the
    /// open).  Race-free complement to `NotRegularFile`: the pre-open
    /// path-based check can observe a regular file that gets swapped for a
    /// non-regular kind before the open runs; this variant is instead
    /// derived from a `File::metadata()` fstat on the descriptor the open
    /// already bound, so no second path lookup can be raced.
    ///
    /// Kept as a peer variant rather than folded into `NotRegularFile` with
    /// a `phase` field (the way [`RejectReason::Oversized`] folds its two
    /// detection points) because the two `NotRegularFile*` cases carry no
    /// extra data to distinguish by phase, and a caller may reasonably want
    /// a different diagnostic for "never opened" versus "opened, but the
    /// descriptor resolved to the wrong kind" (a stronger TOCTOU-swap
    /// signal than the pre-open case). Adding a peer variant to a
    /// `#[non_exhaustive]` enum is non-breaking for existing `match`
    /// arms with a wildcard.
    NotRegularFilePostOpen,
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
    /// The path's `canonicalize` result is not under the caller's
    /// canonical root (an intermediate directory resolved outside it).
    /// Produced only by `read_bounded_contained_file`.
    PathEscape {
        /// `canonicalize(path)` result.
        canonical: std::path::PathBuf,
        /// Canonical root the path had to stay under.
        root: std::path::PathBuf,
    },
    /// The path the *opened descriptor* resolves to is not under the
    /// caller's canonical root: an intermediate directory was swapped
    /// between the pathname checks and the open. Derived from the fd
    /// itself, so unlike [`RejectReason::PathEscape`] it cannot be raced by
    /// a further swap. Produced only by `read_bounded_contained_file`.
    PathEscapePostOpen {
        /// Path the opened descriptor resolves to.
        canonical: std::path::PathBuf,
        /// Canonical root the descriptor had to stay under.
        root: std::path::PathBuf,
    },
    /// I/O error from `symlink_metadata`, the open, `canonicalize`, the
    /// fd-to-path lookup, or `read_to_end`.
    Io(std::io::Error),
}

/// Phase in which [`RejectReason::Oversized`] was detected.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
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
            RejectReason::NotRegularFilePostOpen => write!(
                f,
                "opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open)"
            ),
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
            RejectReason::PathEscape { canonical, root } => write!(
                f,
                "path resolves to {} outside root {}",
                canonical.display(),
                root.display()
            ),
            RejectReason::PathEscapePostOpen { canonical, root } => write!(
                f,
                "opened fd resolves to {} outside root {} (intermediate-directory swap)",
                canonical.display(),
                root.display()
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

/// Open a regular file with the leaf-swap TOCTOU defense stack.
///
/// - **unix**: `O_NOFOLLOW | O_NONBLOCK`.
///   - `O_NOFOLLOW`: a symlink swapped in between the pre-open
///     `symlink_metadata` check and this open call cannot cause the resolver
///     to follow a fresh target. POSIX mandates `ELOOP` for
///     `open(O_NOFOLLOW)` on a symlink; Linux and macOS comply, but FreeBSD
///     deliberately does not — its own `open(2)` documents returning
///     `EMLINK` instead of `ELOOP` for this case as an intentional POSIX
///     deviation, at every supported version, not a legacy-only quirk.
///     Other BSDs (NetBSD, OpenBSD) may return `EMLINK` or `EFTYPE` as well
///     — [`open_bounded_regular_file`] falls back to a `symlink_metadata`
///     recheck on any `Err` to cover all of these portably.
///   - `O_NONBLOCK`: load-bearing time-DoS defense paired with the post-open
///     fstat in [`open_bounded_regular_file`]. If a regular file is swapped
///     for a **writer-less FIFO** between the pre-open `symlink_metadata`
///     check and this `open()`, a bare `open(O_RDONLY | O_NOFOLLOW)` would
///     block indefinitely at the syscall itself — `O_NOFOLLOW` does not fire
///     (a FIFO is not a symlink), so the post-open fstat would never be
///     reached. `O_NONBLOCK` makes FIFO opens return immediately (POSIX:
///     read-side `O_RDONLY | O_NONBLOCK` on a FIFO succeeds even with no
///     writer), letting the post-open fstat inspect the fd and reject
///     non-regular kinds. POSIX specifies `O_NONBLOCK` has no effect on
///     regular files, so happy-path reads are unaffected.
///
/// - **windows**: Win32 has no direct `O_NOFOLLOW` equivalent — `CreateFile`
///   normally resolves a reparse point (symlink, junction, mount point, ...)
///   and hands back a handle to whatever it points at, rather than failing
///   the open. `FILE_FLAG_OPEN_REPARSE_POINT` changes that: it opens the
///   reparse point itself instead of following it. Because the open can
///   still *succeed* on a symlink (unlike `O_NOFOLLOW`, which fails it
///   outright), the symlink check has to move to *after* the open: the
///   handle's `dwFileAttributes` (read via
///   `std::os::windows::fs::MetadataExt::file_attributes`) is inspected for
///   `FILE_ATTRIBUTE_REPARSE_POINT`, and the handle is discarded with a
///   synthesized error if the bit is set. Both the open and the check
///   operate on the same handle — no path is re-resolved in between — so a
///   leaf swapped in between the pre-open `symlink_metadata` check and this
///   call cannot cause a fresh reparse-point target to be read.
///
///   `FILE_FLAG_BACKUP_SEMANTICS` is deliberately **not** added alongside
///   `FILE_FLAG_OPEN_REPARSE_POINT`: it is only needed to open a
///   *directory-type* reparse point, and in a process holding
///   `SeBackupPrivilege` it bypasses normal access checks — a real behavior
///   change this function does not need, since the pre-open `!is_file()`
///   gate already rejects a directory-type leaf before this function runs.
///   Without `FILE_FLAG_BACKUP_SEMANTICS`, `CreateFile` fails outright
///   (`ERROR_ACCESS_DENIED`) when the target is a directory-type reparse
///   point, so even a leaf swapped in after that pre-open gate races is
///   still rejected by the open call itself — a second, independent
///   fail-closed layer, not the sole line of defense against that race.
///
/// - **any other platform**: plain follow-at-open `File::open` — no
///   leaf-swap defense is applied there.
fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
    imp::safe_open(path)
}

#[cfg(unix)]
mod imp {
    use std::path::Path;

    pub(super) fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
    }
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    // Win32 `CreateFile` flag/attribute bits (winnt.h). Not exposed as
    // constants by std, but part of the stable Win32 ABI — these values
    // have been unchanged since Windows NT and are not expected to move.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    pub(super) fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;

        // Post-open, same-handle check: reject if what CreateFile actually
        // bound the handle to is a reparse point. See `safe_open`'s doc
        // comment for why this has to run after the open rather than
        // before it.
        let attrs = file.metadata()?.file_attributes();
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(std::io::Error::other(format!(
                "refusing to open {}: leaf resolved to a reparse point (symlink/junction) \
                 rather than a plain file",
                path.display()
            )));
        }
        Ok(file)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    pub(super) fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::File::open(path)
    }
}

/// Post-open fd-based fstat: verify the handle [`safe_open`] returned still
/// resolves to a regular file.  Load-bearing supplement to the pre-open
/// path-based `symlink_metadata` + `is_file()` gate: that check is
/// race-vulnerable (a regular file can be swapped for a FIFO / character
/// device / block device between the pre-open metadata call and the
/// subsequent open), and `O_NOFOLLOW` does not filter file kind (a FIFO is
/// not a symlink), so the swapped-in kind slips through `safe_open`.
/// Calling `File::metadata()` on the returned fd consults the inode already
/// bound to the descriptor, so no path lookup re-runs and the race window is
/// closed by construction.
///
/// Portable: the fstat is via `std::fs::File::metadata`, which delegates to
/// the platform's fd-based stat (Linux `fstat`, Windows
/// `GetFileInformationByHandle`).  Called unconditionally after a successful
/// `safe_open`, so non-unix builds get the defense-in-depth too, even where
/// `safe_open`'s `O_NOFOLLOW` / `O_NONBLOCK` custom flags are absent.
fn check_open_handle_regular(file: &std::fs::File) -> Result<(), RejectReason> {
    let metadata = file.metadata().map_err(RejectReason::Io)?;
    if !metadata.file_type().is_file() {
        return Err(RejectReason::NotRegularFilePostOpen);
    }
    Ok(())
}

/// Run the pre-open gates, open `path` through the leaf-swap-hardened
/// `safe_open`, and run the post-open kind recheck — everything
/// [`read_bounded_regular_file`] does short of the actual bounded read.
///
/// Split out from [`read_bounded_regular_file`] so a caller that also needs
/// a post-open, fd-derived check (such as the path containment
/// [`read_bounded_contained_file`] performs) can run it against the same
/// descriptor before handing it to [`read_bounded_from_open_file`]. "Checked" and "used" then refer to the
/// same fd, so no second pathname lookup remains to race.
///
/// Returns the open handle together with `metadata.len()` observed at the
/// pre-open check, for a caller's own bookkeeping (e.g. diagnostics).
/// [`read_bounded_from_open_file`] does not need this value — it derives
/// its own read-buffer preallocation hint independently via an fd-based
/// stat on the handle it is given.
///
/// The check order is:
///
/// 1. `symlink_metadata` — refuse to follow symlinks on this call frame.
/// 2. `file_type().is_file()` — reject device/fifo/socket/directory before
///    opening (load-bearing against `mkfifo`-style time DoS).
/// 3. `metadata.len() > size_cap` — reject oversized files up front.
/// 4. `safe_open` — `O_NOFOLLOW` (unix) / reparse-point-aware open
///    (Windows) so a leaf swapped to a symlink between step 1 and this open
///    cannot be followed.  On error, an errno consistent with the open
///    refusing to follow a symlink (`ELOOP`, POSIX-mandated and honored on
///    Linux/macOS, but not on FreeBSD, which deliberately returns `EMLINK`
///    instead at every version) — or, on platforms/errno values that don't
///    distinguish this case, a `symlink_metadata` recheck finding the leaf
///    is now a symlink — maps to [`RejectReason::Symlink`]; anything else
///    maps to [`RejectReason::Io`]. Not race-perfect (an attacker could swap
///    the symlink back to a regular file between the failed open and this
///    recheck), but covers the common attack shape while remaining simple.
/// 5. A post-open, fd-based fstat — reject a kind other than a regular
///    file bound to the opened descriptor (FIFO/device swapped in between
///    step 1 and step 4; `safe_open`'s `O_NONBLOCK` on unix keeps a
///    writer-less FIFO from blocking `open()` before this check runs).
#[doc(hidden)]
pub fn open_bounded_regular_file(
    path: &Path,
    size_cap: u64,
) -> Result<(std::fs::File, u64), RejectReason> {
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

    let file = match safe_open(path) {
        Ok(f) => f,
        Err(e) => {
            #[cfg(unix)]
            {
                let looks_like_symlink_swap = e.raw_os_error() == Some(libc::ELOOP)
                    // cov:ignore: the symlink_metadata fallback below this
                    // line exists for platforms/errno combinations that
                    // don't return ELOOP for O_NOFOLLOW-vs-symlink —
                    // FreeBSD (which returns EMLINK at every version, not
                    // just older ones) plus other non-Linux/macOS BSDs.
                    // Unreachable on this workspace's CI, which runs Linux
                    // only, where ELOOP always fires first.
                    || std::fs::symlink_metadata(path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                if looks_like_symlink_swap {
                    // cov:ignore: only reached when `open_bounded_regular_file`
                    // itself is called against a path that gets symlink-swapped
                    // during the call — a genuine open-time TOCTOU race. The
                    // only test that drives a real race through this
                    // function is the `#[ignore]`d concurrent stress test;
                    // `safe_open_rejects_symlink_at_open_time` does not
                    // exercise this line, since it calls `safe_open`
                    // directly and never goes through
                    // `open_bounded_regular_file`'s own classification here.
                    return Err(RejectReason::Symlink);
                    // cov:ignore: closing brace of an if-block whose only
                    // statement unconditionally returns — llvm-cov attributes
                    // a region to this brace that a `return` never reaches.
                }
            }
            #[cfg(windows)]
            {
                // safe_open's own reparse-point reject is synthesized here
                // (there is no OS errno to match, unlike unix's ELOOP —
                // CreateFile itself succeeded; safe_open constructed the Err
                // after inspecting the handle), so there is nothing to check
                // on `e` directly. Fall back to the same symlink_metadata
                // recheck unix uses for errno values (FreeBSD's EMLINK
                // included) that don't distinguish this case.
                let looks_like_symlink_swap = std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if looks_like_symlink_swap {
                    return Err(RejectReason::Symlink);
                }
            }
            return Err(RejectReason::Io(e));
        }
    };

    check_open_handle_regular(&file)?;

    Ok((file, metadata.len()))
}

/// Read the remainder of an already-open regular-file handle with the
/// `+1-probe` bounded-read pattern.
///
/// The initial `Vec` allocation is sized from an fd-based
/// `file.metadata()?.len()` call on `file` itself — the same descriptor the
/// read below uses, so this consults no path and re-resolves nothing (no
/// new TOCTOU exposure). Deriving the hint internally, rather than taking
/// it as a caller-supplied parameter, also closes a positional-argument
/// footgun a two-`u64`-parameter signature would otherwise invite: every
/// real caller passes the same `len` and `size_cap` pair `open_bounded_regular_file`
/// just produced, and a caller that (or a future caller that) swapped their
/// order would compile silently while quietly changing which value gets
/// enforced as the reject cap.
///
/// The upfront reservation is `min(metadata_len, size_cap, 1 MiB)`: no
/// combination of on-disk file size and caller-supplied `size_cap` can
/// force an over-large `Vec::with_capacity` call (which would abort the
/// process on allocation failure — an outcome this function's whole
/// contract of returning `Err` instead of aborting must not permit). A real
/// file larger than 1 MiB still reads through fine: `read_to_end` grows the
/// buffer via its normal reallocation as bytes come in, so the 1 MiB
/// ceiling only gives up the single-allocation optimized path above that size,
/// not correctness or the `size_cap` bound itself (still enforced by the
/// `+1-probe` below, independent of this reservation). A `metadata()`
/// failure here is folded into the same `Io` reject the read itself would
/// produce, rather than a separate error path.
///
/// The `+1-probe`: `take(size_cap + 1)` then post-read `bytes.len() >
/// size_cap`.  This is what makes silent truncation impossible: a file that
/// grew past `size_cap` after the caller's pre-open size check yields at
/// least `size_cap + 1` bytes and trips the post-read reject instead of
/// returning a truncated buffer.  `saturating_add(1)` avoids `size_cap + 1`
/// overflow when a caller passes `u64::MAX` as the cap — with overflow the
/// read limit would wrap to 0 and any non-empty file would be silently
/// accepted as empty bytes; the saturation keeps the bound at `u64::MAX`
/// (effectively unlimited) so a small file still reads through and the
/// post-check cannot spuriously fire.
#[doc(hidden)]
pub fn read_bounded_from_open_file(
    file: &mut std::fs::File,
    size_cap: u64,
) -> Result<Vec<u8>, RejectReason> {
    const PREALLOC_CEILING: u64 = 1 << 20; // 1 MiB
    let metadata_len = file.metadata().map_err(RejectReason::Io)?.len();
    let cap_hint = std::cmp::min(metadata_len, size_cap).min(PREALLOC_CEILING);
    let mut bytes = Vec::with_capacity(cap_hint as usize);
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

/// Read a regular file with a defense-in-depth bounded read: pre-open
/// symlink/kind/size gates, an `O_NOFOLLOW`-hardened open with a post-open
/// kind recheck (see [`open_bounded_regular_file`]), and a `+1-probe`
/// bounded read (see [`read_bounded_from_open_file`]).
///
/// Convenience wrapper over the two split primitives, for a caller that
/// doesn't need a post-open check between the open and the read. For path
/// containment under a root use [`read_bounded_contained_file`]; for any
/// other post-open check, call [`open_bounded_regular_file`] and
/// [`read_bounded_from_open_file`] directly and run it against the returned
/// handle in between.
#[doc(hidden)]
pub fn read_bounded_regular_file(path: &Path, size_cap: u64) -> Result<Vec<u8>, RejectReason> {
    let (mut file, _len) = open_bounded_regular_file(path, size_cap)?;
    read_bounded_from_open_file(&mut file, size_cap)
}

/// [`read_bounded_regular_file`] plus containment under `canonical_root`:
/// the read is rejected unless the file resolves inside that directory.
///
/// `canonical_root` must already be canonicalized (`std::fs::canonicalize`);
/// both containment checks compare against it with `Path::starts_with`.
///
/// # Checks, in order
///
/// 1. [`open_bounded_regular_file`] (leaf-symlink, kind and size gates;
///    `O_NOFOLLOW` open; post-open kind recheck).
/// 2. `canonicalize(path).starts_with(canonical_root)`, rejecting with
///    [`RejectReason::PathEscape`]. This covers an *intermediate*-directory
///    symlink that points outside the root: the leaf-symlink gate only
///    inspects the final component, so such a path still has a regular-file
///    leaf and passes step 1.
/// 3. A containment recheck derived from the opened descriptor (Linux:
///    `/proc/self/fd/<fd>` readlink; Apple platforms: `fcntl(fd, F_GETPATH,
///    ..)`), rejecting with [`RejectReason::PathEscapePostOpen`]. Step 2 is
///    a separate pathname resolution from the open, so an intermediate
///    directory can be swapped between the two; this step asks the kernel
///    what the *opened* file is, so there is no second lookup left to race.
///    It is authoritative wherever it is implemented, which makes the
///    relative order of steps 1 and 2 immaterial there.
/// 4. [`read_bounded_from_open_file`] on that same descriptor.
///
/// # Platform coverage
///
/// On platforms without an fd-to-path primitive wired up (non-Apple BSDs,
/// Windows, ...) step 3 is a no-op and step 2 is the only containment
/// check. Because step 2 runs after the open, a regular file outside the
/// root is briefly opened (never read) before being rejected on those
/// platforms.
///
/// # Errors
///
/// A failing fd-to-path lookup in step 3 is reported as
/// [`RejectReason::Io`], never skipped: a filesystem that cannot answer
/// the authoritative check fails closed rather than falling back to
/// step 2 alone.
#[doc(hidden)]
pub fn read_bounded_contained_file(
    path: &Path,
    canonical_root: &Path,
    size_cap: u64,
) -> Result<Vec<u8>, RejectReason> {
    let (mut file, _len) = open_bounded_regular_file(path, size_cap)?;

    let canonical = std::fs::canonicalize(path).map_err(RejectReason::Io)?;
    if !canonical.starts_with(canonical_root) {
        return Err(RejectReason::PathEscape {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    check_open_handle_containment(&file, canonical_root)?;

    read_bounded_from_open_file(&mut file, size_cap)
}

/// Linux: the path the kernel reports for `/proc/self/fd/<fd>` is the file
/// bound to *this* descriptor, not a fresh lookup of the original pathname.
/// (The reported string can still change if an ancestor is later renamed;
/// what cannot happen is a second, attacker-steerable pathname lookup.)
#[cfg(target_os = "linux")]
fn check_open_handle_containment(
    file: &std::fs::File,
    canonical_root: &Path,
) -> Result<(), RejectReason> {
    use std::os::unix::io::AsRawFd;
    let fd_link = std::path::PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    // cov:ignore: readlink on a descriptor this process just opened fails
    // only on an environment fault (procfs unmounted, fd-table race), which
    // is not deterministically unit-testable.
    let canonical = std::fs::read_link(&fd_link).map_err(RejectReason::Io)?;
    ensure_contained(canonical, canonical_root)
}

/// Apple platforms: `fcntl(fd, F_GETPATH, ..)` via [`rustix::fs::getpath`],
/// the Darwin/XNU analogue of the Linux `/proc/self/fd/<fd>` readlink. The
/// kernel rebuilds the path from the vnode's name cache, so unlike Linux it
/// has a genuine (if rare) failure path even for a live descriptor; that
/// failure maps to [`RejectReason::Io`] like any other.
///
/// No Apple CI target exists, so this arm and its tests are compiled but
/// not executed in CI. If they ever fail, check the path *form* first:
/// Darwin reports `/private/var/...` for paths reached through `/var`
/// (where `tempfile::tempdir()` lands), and the root must be canonicalized
/// the same way for `starts_with` to agree.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
fn check_open_handle_containment(
    file: &std::fs::File,
    canonical_root: &Path,
) -> Result<(), RejectReason> {
    use std::os::unix::ffi::OsStringExt;
    let path_cstr =
        rustix::fs::getpath(file).map_err(|errno| RejectReason::Io(std::io::Error::from(errno)))?;
    let canonical = std::path::PathBuf::from(std::ffi::OsString::from_vec(path_cstr.into_bytes()));
    ensure_contained(canonical, canonical_root)
}

/// Every other platform (non-Apple BSDs, Windows, ...): no fd-to-path
/// primitive is wired up, so the `canonicalize` check in
/// [`read_bounded_contained_file`] is the only containment gate.
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn check_open_handle_containment(
    _file: &std::fs::File,
    _canonical_root: &Path,
) -> Result<(), RejectReason> {
    Ok(())
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
fn ensure_contained(
    canonical: std::path::PathBuf,
    canonical_root: &Path,
) -> Result<(), RejectReason> {
    if canonical.starts_with(canonical_root) {
        Ok(())
    } else {
        Err(RejectReason::PathEscapePostOpen {
            canonical,
            root: canonical_root.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests;
