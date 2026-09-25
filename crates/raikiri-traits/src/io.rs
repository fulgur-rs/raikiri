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
pub enum RejectReason {
    /// `symlink_metadata().file_type().is_symlink()` returned `true` at the
    /// pre-open check, **or** the open-time leaf-swap defense in
    /// [`open_bounded_regular_file`] (`O_NOFOLLOW` on unix, a
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
    /// The descriptor [`open_bounded_regular_file`] actually opened does not
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
    /// Produced only by [`read_bounded_contained_file`].
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
    /// a further swap. Produced only by [`read_bounded_contained_file`].
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
                panic!(
                    "expected Oversized{{DuringRead}} via +1-probe post-read reject, got {other:?}"
                )
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
        let file = safe_open(path).expect(
            "safe_open on /dev/null should succeed (regular open semantics on char device)",
        );
        match check_open_handle_regular(&file) {
            Err(RejectReason::NotRegularFilePostOpen) => {}
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            other => panic!(
                "expected NotRegularFilePostOpen for /dev/null post-open fstat, got {other:?}"
            ),
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
}
