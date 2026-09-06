//! WPT bundled font dir を register し、system_fonts: false と generic
//! family alias で cross-machine 決定性 FontContext を構築する。
//!
//! - Fetch は `scripts/wpt/fetch.sh` (dev prerequisite)、本 module は
//!   fetch 済 `target/wpt/fonts/` を Path で受けるだけ
//! - production runtime は `parley::FontContext::new()` を今のまま使う
//! - 現状 scope: `.ttf` / `.otf` のみ、WOFF/WOFF2 は将来対応
//!
//! 参考実装:
//! - fulgur `crates/fulgur-wpt/src/fonts.rs::load_fonts_dir` (walker + sort)
//! - blitz `packages/blitz-dom/src/lib.rs::build_single_font_ctx`
//!   (system_fonts: false + generic alias append)
//!
//! Structured warn hook: [`build_wpt_font_ctx_with_observer`] accepts an
//! optional callback that receives a [`FontWarn`] for every warn+skip site
//! (walker + read-time TOCTOU + fontique register-empty). The signature-
//! preserving [`build_wpt_font_ctx`] delegates to it with `None`, keeping
//! the CLI-facing `eprintln!` shape for external consumers pinned by
//! `crates/raikiri/tests/external_consumer.rs`.

use parley::FontContext;
use std::path::{Path, PathBuf};

/// [`build_wpt_font_ctx`] が個別 font file を読み込む際に許容する最大 byte 数。
/// 100 MiB は現実の bundled font (Ahem: ~12 KiB, Noto CJK: ~20 MiB 前後) に対して
/// 十分な余裕を残しつつ、attacker が用意した巨大 regular file による memory
/// exhaustion を弾く閾値。
///
/// **Threat surface coverage**:
///
/// - **symlink → /dev/zero**: `collect_recursive` 側の
///   `file_type.is_symlink()` skip で既に closed
/// - **FIFO / device / socket** (indefinite block): `collect_recursive` 側の
///   `file_type.is_file()` gate で walk 段階で closed。`Read::take(N)` は memory を
///   bound するが writer 未定の FIFO に対して time は bound しないので、walk 段階で
///   排除するのが 1st line of defense。read 段階の TOCTOU-swap (regular → FIFO /
///   device 差し替え) は下記 leaf-swap 項の後段で defended
/// - **oversized regular file** (memory exhaustion): `metadata.len() >
///   FONT_SIZE_CAP` skip で closed
/// - **mid-read grow (TOCTOU)**: the `+1-probe` inside
///   `raikiri_traits::io::read_bounded_from_open_file` (`take(FONT_SIZE_CAP +
///   1) + post-read bytes.len > cap` reject) prevents silent truncation and
///   surfaces TOCTOU-grow as `OversizedDuringRead`.
/// - **leaf-swap (TOCTOU)**: walker と `raikiri_traits::io::open_bounded_regular_file`
///   の pre-open `symlink_metadata` の間で regular file が symlink に差し替わる
///   vector は、`open_bounded_regular_file` の open-time `O_NOFOLLOW` (unix) /
///   reparse-point-aware open (Windows) で closed。ELOOP (POSIX 準拠、Linux /
///   macOS が該当。FreeBSD は全バージョンで意図的に EMLINK を返す非準拠) or 事後
///   `symlink_metadata` recheck (FreeBSD/NetBSD/OpenBSD の EMLINK / EFTYPE、
///   あるいは Windows の synthesized reject) を「leaf-swap
///   symlink 相当」として warn+skip し、Ahem の drop は下流の aggregate
///   `PreferredFontUnavailable` check が catch する。
///
/// - **FIFO / device-swap (TOCTOU) after pre-open metadata**: walker skip と
///   `open_bounded_regular_file` の pre-open `symlink_metadata` の間に regular
///   file が FIFO / character device / block device に差し替わる vector は、
///   `open_bounded_regular_file` の open (unix: `O_NOFOLLOW | O_NONBLOCK`) +
///   post-open fd-based fstat で closed。`O_NONBLOCK` が writer 未定 FIFO の
///   `open()` block を防ぎ (time-DoS の 1st defense)、post-open
///   `File::metadata().file_type().is_file()` の fd-based check が非 regular
///   kind を race-free に reject する (`NotRegularFilePostOpen`)。pre-open
///   path-based check と違い fd 発行後の stat なので path-swap TOCTOU では
///   bypass 不可能。
///
/// TODO: 将来 RenderLimits と連動させる。
const FONT_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// Reason `read_bounded_font_file` rejected a candidate font path.
///
/// Mirrors the shape of `raikiri_traits::io::RejectReason` (which this
/// module's leaf-swap / kind / size gates now delegate to via
/// `raikiri_traits::io::open_bounded_regular_file` and
/// `read_bounded_from_open_file`) so the callsite policy (`Io` → propagate,
/// other reasons → warn+skip) reads the same as before the delegation. A
/// local enum is kept alongside the traits crate's `RejectReason` because
/// this module's variants also need to carry the path-containment rejects
/// (`PathEscape` / `PathEscapePostOpen`) that stay module-local — see
/// `check_open_handle_containment` below for why.
#[derive(Debug)]
enum FontReadReject {
    /// `symlink_metadata().file_type().is_symlink()` returned true, **or**
    /// the open returned an errno consistent with `O_NOFOLLOW` refusing to
    /// follow a symlink (POSIX `ELOOP`; a fallback `symlink_metadata` recheck
    /// covers errno values that don't distinguish this case, including
    /// FreeBSD's `EMLINK` at every version, not just older BSDs).  Either
    /// way the leaf was a symlink at reject time and no read happened
    /// through it.
    Symlink,
    /// Path is neither a regular file nor a symlink (device, fifo, socket,
    /// directory, block/char device).  Rejected at pre-open metadata time —
    /// `File::open` on a fifo with no writer would block indefinitely.
    NotRegularFile,
    /// The fd returned by `safe_open` resolves to something other than a
    /// regular file (FIFO / character device / block device / socket).
    /// Rejected via a fd-based `File::metadata()` fstat after open, so the
    /// check is race-free against a path-swap between pre-open
    /// `symlink_metadata` and `safe_open`.  Paired with `O_NONBLOCK` in
    /// `safe_open` (unix) so a writer-less FIFO swapped in mid-window cannot
    /// block `open()` before the fstat is reached.  Callsite policy:
    /// warn+skip (analogous to `NotRegularFile`).  Variant is split by
    /// callsite (pre-open vs post-open) rather than reusing `NotRegularFile`
    /// so the TOCTOU signal — regular at walk, non-regular at read — remains
    /// distinguishable to observers (same philosophy as [`FontWarn`]'s
    /// per-stage split).  Closes the FIFO/device leaf-swap gap.
    NotRegularFilePostOpen,
    /// Pre-open `metadata.len() > cap` reject.  The file was never opened.
    /// `cap` carries the configured cap (rather than deferring to
    /// `FONT_SIZE_CAP`) so tests that pass a small cap get honest messages.
    OversizedPreOpen { size: u64, cap: u64 },
    /// The file grew past `cap` between the metadata check and the bounded
    /// read (TOCTOU-grow race), caught by the `+1-probe` pattern:
    /// `take(cap + 1)` then post-read `bytes.len() > cap`.
    OversizedDuringRead { size: u64, cap: u64 },
    /// After canonicalization, the candidate path resolves outside the
    /// walker's canonical root — the intermediate-symlink escape vector
    /// (`fonts_dir/subdir/` swapped into a symlink to `/tmp/evil/` between
    /// walk and read, with `subdir/Ahem.ttf` still a regular file leaf so
    /// `is_symlink()` on the leaf does not fire).  Belt-and-suspenders: given
    /// the walker's `is_symlink()` skip on directory entries this branch is
    /// only reachable if the tree changed between walk and read, but the
    /// containment check is what makes that safe.  Mirrors
    /// `raikiri_vrt::reference::FixtureError::PathEscape`.
    /// Closes the intermediate-directory-swap PathEscape gap.
    PathEscape {
        /// Canonicalized target path that fell outside the root.
        canonical: PathBuf,
        /// Canonicalized fonts root that the target should have stayed under.
        root: PathBuf,
    },
    /// Post-open, fd-bound containment recheck (`check_open_handle_containment`)
    /// found the already-opened descriptor resolves outside `canonical_root`.
    /// Unlike [`FontReadReject::PathEscape`] (a fresh, path-based
    /// `canonicalize` re-resolution — itself TOCTOU-vulnerable to a second
    /// swap before this recheck runs, and, per [`read_bounded_font_file`]'s
    /// own doc, now performed *after* the open rather than before it),
    /// this variant is derived from the fd the open actually returned —
    /// on Linux via `/proc/self/fd/<fd>`, on Apple platforms via
    /// `fcntl(fd, F_GETPATH, ..)` — so firing here
    /// means the intermediate-directory swap happened inside the window
    /// between the open and this fd-based recheck (the exact gap left
    /// surviving the earlier fixes). Split by path-based-vs-fd-based
    /// callsite for the same reason `NotRegularFile`/`NotRegularFilePostOpen`
    /// are split: the distinction is the TOCTOU signal, not which one runs
    /// first (both now run after the open).
    PathEscapePostOpen {
        /// fd-derived canonical path (post-open) that fell outside the root.
        canonical: PathBuf,
        /// Canonicalized fonts root that the target should have stayed under.
        root: PathBuf,
    },
    /// I/O error from `symlink_metadata`, `safe_open`, `canonicalize`, or
    /// `read_to_end` that is not consistent with a symlink-swap.  Callsite
    /// propagates as [`FontError::Io`].
    Io(std::io::Error),
}

impl std::fmt::Display for FontReadReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontReadReject::Symlink => write!(f, "path is a symlink"),
            FontReadReject::NotRegularFile => write!(f, "path is not a regular file"),
            FontReadReject::NotRegularFilePostOpen => write!(
                f,
                "opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open)"
            ),
            FontReadReject::OversizedPreOpen { size, cap } => {
                write!(f, "file size {size} bytes exceeds cap {cap} bytes")
            }
            FontReadReject::OversizedDuringRead { size, cap } => write!(
                f,
                "file grew past cap during read: {size} bytes read, cap {cap} bytes (TOCTOU-grow)"
            ),
            FontReadReject::PathEscape { canonical, root } => write!(
                f,
                "canonicalizes to {} which escapes fonts root {}",
                canonical.display(),
                root.display()
            ),
            FontReadReject::PathEscapePostOpen { canonical, root } => write!(
                f,
                "opened fd resolves to {} which escapes fonts root {} (TOCTOU-swap between the open and this containment recheck)",
                canonical.display(),
                root.display()
            ),
            FontReadReject::Io(source) => write!(f, "I/O error: {source}"),
        }
    }
}

/// Post-open fd-bound containment recheck: verify the descriptor
/// `raikiri_traits::io::open_bounded_regular_file` returned still resolves
/// under `canonical_root`, using a path derived **from the fd itself**
/// rather than a fresh path-based `canonicalize` call.
///
/// # Why this closes the intermediate-dir-swap TOCTOU
///
/// `read_bounded_font_file`'s `canonicalize` gate (which, per that
/// function's own doc, runs after the open rather than before it) checks
/// `starts_with(canonical_root)`. That check and the open inside
/// `open_bounded_regular_file` independently re-resolve the same pathname,
/// so they are two separate lookups of a mutable filesystem tree, not one
/// atomic operation on one object: an attacker who swaps an intermediate
/// directory component between the two calls can make the canonicalize
/// observe a path inside `canonical_root` while the open's own pathname
/// resolution — running microseconds earlier or later — binds the fd to a
/// different, outside-root object. The leaf is still a regular file either
/// way, so neither the post-open kind check inside
/// `open_bounded_regular_file` (kind only) nor the path-based
/// canonicalize check (independently re-resolved, stale by the time this
/// runs) catches the swap.
///
/// The fix is to derive the containment check from the **same fd** that
/// will be read, so there is no second pathname resolution after the
/// check to race against. On Linux, `/proc/self/fd/<fd>` is a magic
/// symlink the kernel keeps pointing at the dentry the fd is actually
/// bound to; `readlink`-ing it costs one syscall and does not walk
/// `path` again, so the check is bound to the fd the open actually returned
/// rather than to an independent re-resolution of `path` — which is
/// exactly the attack this closes. (Precisely: the kernel reconstructs
/// the *reported path string* from the dentry chain at readlink time, so
/// a rename of an ancestor directory between the open and this call
/// can still change what `readlink` reports; what cannot happen is a
/// second, attacker-steerable *pathname lookup* of `path` — the class of
/// swap this task closes.) Because the same `file` is used for both this
/// check and the subsequent read, "checked" and "used" are provably the
/// same fd — matching the pattern `raikiri_traits::io::open_bounded_regular_file`
/// already establishes internally for its own post-open kind check.
///
/// If `/proc` is unavailable (e.g. a container/chroot without procfs
/// mounted), `read_link` returns `Err` and this function propagates it
/// via `FontReadReject::Io`. At the `read_bounded_font_file` callsite
/// that reject is **not** warn+skip — `FontReadReject::Io` hard-aborts
/// the whole font-context build as `FontError::Io` (see the "Not
/// surfaced" section on [`FontWarn`]). This is deliberate fail-closed:
/// a filesystem that cannot support the authoritative containment check
/// is treated as a hard error rather than silently falling back to the
/// weaker path-based-only guarantee (the `canonicalize` check alone,
/// without this fd-based recheck).
///
/// # Portability
///
/// Two platform families have an active fd-to-path primitive; a third
/// (everything else, including non-Apple BSDs and Windows) has none wired
/// up and falls back to a no-op.
///
/// - **Linux** (this impl): `std::fs::read_link` on `/proc/self/fd/<fd>`
///   (pure Rust, no `unsafe`, no new dependency, no raw FFI call).
/// - **Apple platforms** (macOS/iOS/tvOS/watchOS/visionOS, see the
///   `#[cfg(any(target_os = "macos", ...))]` impl below): `fcntl(fd,
///   F_GETPATH, ..)` via `rustix::fs::getpath`, a safe wrapper — added
///   as a direct dependency: Pure Rust, already transitively vetted in this
///   dependency tree (`Cargo.lock` carried rustix v1.1.4 via `tempfile`
///   before this change), keeps raikiri-dom's own `unsafe` surface at
///   zero for this fix. `F_GETPATH` is a Darwin/XNU-specific `fcntl`
///   command — it is not a generic BSD primitive; FreeBSD/OpenBSD/NetBSD/
///   DragonFly have no equivalent, and `rustix::fs::getpath` itself is
///   `#[cfg(apple)]`-gated internally (see rustix's own `build.rs`) to
///   exactly this platform set, so this fix covers Apple platforms only.
/// - **Everything else** (non-Apple BSDs, Windows, ...): no-op fallback
///   (below) that leaves the `canonicalize` + `starts_with` check (which
///   runs *after* the open, not before — see [`read_bounded_font_file`]'s
///   own doc) as the only *intermediate-directory-swap containment* gate
///   there — no worse than
///   before this fix landed. Non-Apple BSDs remain an untracked residual
///   as of this change. This is a distinct gap from Windows' leaf-swap-at-
///   open defense, which `raikiri_traits::io::open_bounded_regular_file`
///   already closes via `FILE_FLAG_OPEN_REPARSE_POINT` plus a post-open
///   attribute recheck — see that function's doc for details.
///
/// Closes the intermediate-directory-swap PathEscape residual.
#[cfg(target_os = "linux")]
fn check_open_handle_containment(
    file: &std::fs::File,
    canonical_root: &Path,
) -> Result<(), FontReadReject> {
    use std::os::unix::io::AsRawFd;
    let fd_link = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    let canonical = std::fs::read_link(&fd_link).map_err(FontReadReject::Io)?;
    if !canonical.starts_with(canonical_root) {
        return Err(FontReadReject::PathEscapePostOpen {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    Ok(())
}

/// Apple-platform implementation: derives the post-open containment path
/// from the same fd the open returned, via `fcntl(fd, F_GETPATH, ..)`
/// through [`rustix::fs::getpath`] — the Darwin/XNU analogue of the Linux
/// arm's `/proc/self/fd/<fd>` readlink above. Same race-freedom argument
/// applies: `F_GETPATH` asks the kernel for the path bound to *this*
/// vnode via *this* descriptor, not a fresh pathname lookup of `path`, so
/// there is no second attacker-steerable resolution to race against.
/// (Caveat mirroring the Linux arm's: the kernel reconstructs the
/// *reported path string* from the vnode's cached name cache entry, which
/// can still change if an ancestor directory is renamed, or go stale
/// after the last hard link to the file is unlinked; what cannot happen
/// is a second, attacker-steerable *pathname lookup* of `path` — the
/// class of swap this task closes.)
///
/// A `getpath` failure is mapped through `From<rustix::io::Errno> for
/// std::io::Error` into `FontReadReject::Io`, taking the same fail-closed
/// hard-abort path as the Linux arm's `read_link` failure (see the
/// "deliberate fail-closed" paragraph above): a
/// filesystem that cannot support the authoritative containment check is
/// a hard error, not a silent fallback to the weaker path-based-only
/// guarantee (the `canonicalize` check alone, without this fd-based
/// recheck). Note the failure *shape* differs from Linux: a live fd's
/// `/proc/self/fd/<fd>` entry effectively cannot fail to `readlink`, while
/// `F_GETPATH` reconstructs its answer from the vnode's name cache and has
/// a genuine (if rare) failure path even for a live, still-open
/// descriptor — the fail-closed mapping above treats that the same as any
/// other containment-check failure.
///
/// # Untested in CI
///
/// No CI target for any Apple platform exists in this repo yet. The unit
/// tests mirroring
/// `check_open_handle_containment_accepts_file_within_root` /
/// `_rejects_file_outside_root` are compiled and pinned under this same
/// `cfg` below but have never executed against a real toolchain. This is a
/// deliberate tradeoff: document untested-status rather than block on
/// standing up Apple CI. If a future
/// Apple CI run fails these tests, the first suspect should be
/// `F_GETPATH`'s path *form* rather than the containment logic — Darwin
/// resolves several common temp-dir prefixes to a different canonical
/// form than the path used to reach them (`/tmp` → `/private/tmp`,
/// `/var` → `/private/var`), and `tempfile::tempdir()` on macOS lands
/// under `/var/folders/...`, i.e. exactly such a prefix. This function
/// itself does not care (`canonical_root` is derived the same way, from
/// `std::fs::canonicalize` on the same temp dir, so both sides of the
/// `starts_with` comparison would resolve consistently), but a mismatch
/// here is the first thing to check before suspecting the containment
/// check proper.
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
) -> Result<(), FontReadReject> {
    use std::os::unix::ffi::OsStringExt;
    let path_cstr = rustix::fs::getpath(file)
        .map_err(|errno| FontReadReject::Io(std::io::Error::from(errno)))?;
    let canonical = PathBuf::from(std::ffi::OsString::from_vec(path_cstr.into_bytes()));
    if !canonical.starts_with(canonical_root) {
        return Err(FontReadReject::PathEscapePostOpen {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    Ok(())
}

/// Fallback for every other platform (non-Apple BSDs, Windows, ...): no
/// portable primitive is wired up (see the Portability section on the
/// `target_os = "linux"` impl above). No-op so behavior on these
/// platforms is unchanged by this task — the `canonicalize` + `starts_with`
/// gate in `read_bounded_font_file` (which now runs *after* the open, per
/// that function's own comments) remains the only containment check,
/// exactly as it was before the post-open containment recheck existed.
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
) -> Result<(), FontReadReject> {
    Ok(())
}

/// Map a [`raikiri_traits::io::RejectReason`] onto this module's
/// [`FontReadReject`] taxonomy. `raikiri_traits::io::open_bounded_regular_file`
/// and `read_bounded_from_open_file` carry no `path` field (the caller
/// already has it), so `FontReadReject`'s path-carrying variants are not in
/// play here — this mapping only ever produces the path-free variants.
///
/// `RejectReason` and `OversizePhase` are both `#[non_exhaustive]`; a future
/// variant this match has not been updated for falls through to the
/// wildcard arm and is treated as an I/O-shaped hard error (this module's
/// existing fail-closed policy for anything it cannot classify), rather
/// than being silently downgraded to warn+skip.
fn map_reject_reason(reason: raikiri_traits::io::RejectReason) -> FontReadReject {
    use raikiri_traits::io::{OversizePhase, RejectReason};
    match reason {
        RejectReason::Symlink => FontReadReject::Symlink,
        RejectReason::NotRegularFile => FontReadReject::NotRegularFile,
        RejectReason::NotRegularFilePostOpen => FontReadReject::NotRegularFilePostOpen,
        RejectReason::Oversized {
            size,
            cap,
            phase: OversizePhase::PreOpen,
        } => FontReadReject::OversizedPreOpen { size, cap },
        RejectReason::Oversized {
            size,
            cap,
            phase: OversizePhase::DuringRead,
        } => FontReadReject::OversizedDuringRead { size, cap },
        RejectReason::Io(source) => FontReadReject::Io(source),
        // cov:ignore: unreachable while RejectReason is Symlink|NotRegularFile|
        // NotRegularFilePostOpen|Oversized|Io only (all explicitly matched
        // above); required for its #[non_exhaustive] contract.
        other => FontReadReject::Io(std::io::Error::other(other.to_string())),
    }
}

/// Read a regular font file with the leaf-swap TOCTOU defense stack:
/// `raikiri_traits::io::open_bounded_regular_file` provides the pre-open
/// `symlink_metadata` + `is_file()` + size-cap gates, the `O_NOFOLLOW |
/// O_NONBLOCK` open (unix) / reparse-point-aware open (Windows), and the
/// post-open fd-based kind recheck (rejects non-regular kinds bound to the
/// descriptor). This function layers the path-containment defense on top —
/// a `canonicalize` + `starts_with(canonical_root)` gate, and
/// [`check_open_handle_containment`] (rejects an fd resolving outside
/// `canonical_root`, on Linux via `/proc/self/fd/<fd>`, on Apple platforms
/// via `fcntl(fd, F_GETPATH, ..)`) — race-free against a path-swap because
/// it consults the fd the traits helper's open actually returned rather
/// than re-resolving `path` — and the `+1-probe` bounded read via
/// `raikiri_traits::io::read_bounded_from_open_file`.
///
/// # `canonical_root` containment
///
/// `canonical_root` is the pre-canonicalized walker root, threaded through
/// so `read_bounded_font_file` can verify that after canonicalization the
/// candidate path stays inside that root.  Belt-and-suspenders against the
/// intermediate-symlink attack (walker recorded `subdir/font.ttf`, attacker
/// swapped `subdir/` into a symlink to `/tmp/evil/` between walk and read).
/// The leaf-symlink check does not cover this — `symlink_metadata` follows
/// intermediate components and only refuses to follow the final component —
/// so an intermediate-symlink escape produces `is_symlink() == false` on the
/// leaf and would slip past every existing gate.  Mirrors
/// `raikiri_vrt::reference::read_bounded_fixture_file`. Closes the
/// intermediate-directory-swap PathEscape gap.
///
/// This `canonicalize`-based gate is **not** race-free by itself: it and
/// the open inside `open_bounded_regular_file` are two independent
/// pathname resolutions of a mutable tree, so an intermediate directory can
/// be swapped between them regardless of which one runs first — it is run
/// *after* the open below purely because the open must happen first to
/// obtain the handle [`check_open_handle_containment`] needs. On platforms
/// where that fd-based recheck is actually wired up (Linux, Apple; see the
/// Portability section above), the ordering is immaterial: the post-open
/// recheck is authoritative regardless of when this `canonicalize` runs.
/// On the no-op fallback platforms, though, this ordering means a regular
/// file outside `canonical_root` is now briefly opened before being
/// rejected — never read, so no content is disclosed, but the fd exists
/// for a moment where it previously would not have; a documented residual
/// on those platforms, not a claim that the ordering has zero effect
/// anywhere.
/// [`check_open_handle_containment`] closes the residual window post-open,
/// deriving the containment check from the fd the open actually returned
/// instead of a second path-based lookup. Closes the
/// intermediate-directory-swap PathEscape residual.
fn read_bounded_font_file(
    path: &Path,
    canonical_root: &Path,
    size_cap: u64,
) -> Result<Vec<u8>, FontReadReject> {
    let (mut file, _len) =
        raikiri_traits::io::open_bounded_regular_file(path, size_cap).map_err(map_reject_reason)?;

    // Containment check via canonicalization.  Given the leaf `is_symlink()`
    // gate inside `open_bounded_regular_file`, an unresolvable escape via a
    // *leaf* symlink is already rejected; this branch covers
    // *intermediate*-directory-symlink escapes (e.g. `fonts_dir/subdir/`
    // swapped into a symlink to `/tmp/evil/` between walk and read, with
    // `subdir/Ahem.ttf` still a regular file leaf so `is_symlink()` on the
    // leaf does not fire).  Mirrors
    // `raikiri_vrt::reference::read_bounded_fixture_file`. Closes the
    // intermediate-directory-swap PathEscape gap.  Only "belt": the
    // fd-bound `check_open_handle_containment` call below (post-open,
    // derived from the fd the open above actually returned) is what closes
    // the window.
    let canonical = std::fs::canonicalize(path).map_err(FontReadReject::Io)?;
    if !canonical.starts_with(canonical_root) {
        return Err(FontReadReject::PathEscape {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    // Post-open fd-bound containment recheck: verify the descriptor the
    // open above bound still resolves under `canonical_root`, derived from
    // the fd itself (Linux: `/proc/self/fd/<fd>` readlink; Apple platforms:
    // `fcntl(fd, F_GETPATH, ..)`) rather than a fresh path-based
    // `canonicalize`. Closes the intermediate-dir-swap TOCTOU window
    // between the open above and this recheck itself — the `canonicalize`
    // above is its own separate, independently re-resolved pathname lookup
    // and cannot close that window by construction (see this function's
    // own doc comment). See `check_open_handle_containment` doc for the full
    // rationale and the remaining no-op fallback for platforms without a
    // wired-up primitive.
    check_open_handle_containment(&file, canonical_root)?;

    raikiri_traits::io::read_bounded_from_open_file(&mut file, size_cap).map_err(map_reject_reason)
}

/// Structured warn event emitted by [`build_wpt_font_ctx_with_observer`] for
/// every warn+skip site (walker + read-time TOCTOU + fontique register-empty).
/// Consumers pass an `Option<&mut dyn FnMut(&FontWarn<'_>)>` observer to opt
/// into programmatic consumption of these events; the [`build_wpt_font_ctx`]
/// shim omits the observer and keeps the CLI-facing `eprintln!` behavior.
///
/// # Design
///
/// Sibling convention: `#[non_exhaustive]` mirrors the taxonomy enums in
/// `crates/raikiri-traits/src/error.rs`.  Variants are
/// **split by callsite** (walker vs read) rather than sharing a single
/// [`FontReadReject`]-shaped taxonomy because the walker-vs-read distinction
/// is the exact TOCTOU signal that matters here: a walker `Symlink` is a
/// mundane cycle-safe skip, whereas a read-time `Symlink` means the tree
/// changed between walk and read (leaf-swap TOCTOU).  Collapsing them would
/// destroy that signal.
///
/// # Not surfaced
///
/// `FontReadReject::Io(_)` never becomes a `FontWarn` variant.  It is
/// hard-propagated as [`FontError::Io`] (preserves the
/// "Ahem.ttf silent-fallback prevention" guarantee), never warn+skip, so an
/// observer never sees it.
#[non_exhaustive]
#[derive(Debug)]
pub enum FontWarn<'a> {
    /// Walker skipped a symlink entry under `fonts_dir` (cycle-safe policy).
    /// Mundane: pre-read filter to keep `Path::is_dir()`-cycle recursion off.
    WalkerSkippedSymlink {
        /// Absolute (or `fonts_dir`-relative) path of the skipped symlink.
        path: &'a Path,
    },
    /// Walker skipped a non-regular entry (FIFO / device / socket /
    /// block-or-char device).  `open(O_RDONLY)` on a writer-less FIFO would
    /// block indefinitely; the pre-read `file_type.is_file()` gate is the
    /// time-DoS defense.
    WalkerSkippedNonRegular {
        /// Absolute (or `fonts_dir`-relative) path of the skipped entry.
        path: &'a Path,
        /// The `FileType` returned by `DirEntry::file_type`, preserved so an
        /// observer can distinguish FIFO vs device vs socket vs block-or-char.
        file_type: std::fs::FileType,
    },
    /// Walker skipped a regular file whose `metadata.len()` exceeded
    /// [`FONT_SIZE_CAP`] at walk time.
    WalkerSkippedOversized {
        /// Absolute (or `fonts_dir`-relative) path of the oversized file.
        path: &'a Path,
        /// Logical file size (`metadata.len()`) at walk time.
        size: u64,
        /// The size cap being enforced (currently [`FONT_SIZE_CAP`]).
        cap: u64,
    },
    /// Read stage rejected a candidate as a symlink.  The walker had already
    /// accepted the path as a regular file, so surfacing here signals a
    /// **leaf-swap TOCTOU race** between walk and read.
    ReadRejectedSymlink {
        /// Path that was a regular file at walk time and a symlink at read.
        path: &'a Path,
    },
    /// Read stage rejected a candidate whose kind was neither regular file
    /// nor symlink.  As with [`FontWarn::ReadRejectedSymlink`], the walker
    /// had already accepted the path, so this is a TOCTOU-swap signal.
    ReadRejectedNotRegularFile {
        /// Path that was a regular file at walk time and something else at read.
        path: &'a Path,
    },
    /// Read stage's **post-open fd-based fstat** rejected a candidate whose
    /// opened descriptor did not resolve to a regular file.  This is the
    /// race-free complement to [`FontWarn::ReadRejectedNotRegularFile`]: the
    /// pre-open path-based check is TOCTOU-vulnerable (a regular file can be
    /// swapped for a FIFO / device between pre-open metadata and
    /// `safe_open`), whereas the post-open fstat consults the inode already
    /// bound to the fd.  Firing here means the swap happened inside the
    /// pre-open→open window; paired with `O_NONBLOCK` in `safe_open` (unix)
    /// so a writer-less FIFO cannot block `open()` before this fstat runs.
    /// Closes the FIFO/device leaf-swap gap.
    ReadRejectedNotRegularFilePostOpen {
        /// Path whose opened fd resolved to a non-regular kind
        /// (swap window between pre-open metadata and open).
        path: &'a Path,
    },
    /// Read stage rejected a candidate whose `metadata.len()` exceeded the
    /// cap between walk and read (TOCTOU-swap to a larger file, or racy
    /// grow-in-place before the pre-open metadata check).
    ReadRejectedOversizedPreOpen {
        /// Path whose size crossed the cap between walk and read.
        path: &'a Path,
        /// Post-swap logical file size (`metadata.len()`) at read time.
        size: u64,
        /// The size cap being enforced (currently [`FONT_SIZE_CAP`]).
        cap: u64,
    },
    /// Read stage caught a **TOCTOU-grow** race via the `+1-probe` pattern:
    /// the file grew past the cap between the pre-open metadata check and
    /// the bounded read.  The load-bearing observability signal this
    /// enum was designed for.
    ReadRejectedOversizedDuringRead {
        /// Path that grew past the cap during the bounded read.
        path: &'a Path,
        /// Number of bytes actually read before the `+1-probe` fired
        /// (`bytes.len() as u64`, `> cap`).
        size: u64,
        /// The size cap being enforced (currently [`FONT_SIZE_CAP`]).
        cap: u64,
    },
    /// Read stage caught an **intermediate-symlink escape**: the leaf
    /// resolved (via `canonicalize` then `starts_with(canonical_root)`)
    /// outside the walker's canonical root after an intermediate directory
    /// was swapped into a symlink between walk and read.  Mirror of
    /// `raikiri_vrt::reference::FixtureError::PathEscape`.
    /// Closes the intermediate-directory-swap PathEscape gap.
    ReadRejectedPathEscape {
        /// Walker-observed candidate path (pre-canonicalization).
        path: &'a Path,
        /// Canonicalized target path that fell outside the root.
        canonical: &'a Path,
        /// Canonicalized fonts root that the target should have stayed under.
        root: &'a Path,
    },
    /// Read stage's **post-open fd-bound containment recheck** rejected a
    /// candidate whose opened descriptor resolved outside the walker's
    /// canonical root. This is the race-free complement to
    /// [`FontWarn::ReadRejectedPathEscape`]: that check's `canonicalize`
    /// (which, per [`read_bounded_font_file`]'s own doc, now runs after the
    /// open rather than before it) re-resolves the pathname a second time
    /// (independently of the open's own resolution), leaving a window for
    /// an intermediate-directory swap between the two lookups; this fd-bound
    /// recheck (Linux: `/proc/self/fd/<fd>` readlink; Apple platforms:
    /// `fcntl(fd, F_GETPATH, ..)`) is derived from the fd the open actually
    /// returned, so firing here means the swap happened inside the window
    /// between the open and this recheck itself. Closes the
    /// intermediate-directory-swap PathEscape residual.
    ReadRejectedPathEscapePostOpen {
        /// Walker-observed candidate path (pre-canonicalization).
        path: &'a Path,
        /// fd-derived canonical path (post-open) that fell outside the root.
        canonical: &'a Path,
        /// Canonicalized fonts root that the target should have stayed under.
        root: &'a Path,
    },
    /// fontique's `register_fonts` returned no families for the read blob
    /// (parse-invalid font, corrupt asset, etc.).  Handled by the aggregate
    /// [`FontError::PreferredFontUnavailable`] / [`FontError::NoFontsRegistered`]
    /// invariant checks downstream; the observer is the only per-file
    /// programmatic signal.
    RegisterEmpty {
        /// Path whose blob fontique rejected with an empty family list.
        path: &'a Path,
    },
}

impl<'a> std::fmt::Display for FontWarn<'a> {
    /// Reproduces the pre-observer `eprintln!` message bodies verbatim so
    /// swapping between observer-Some and observer-None does not change what
    /// operators see on stderr (message wording is a "behavior-change"
    /// surface, so it's held stable deliberately).  The
    /// callsite prefixes `[raikiri-dom::fonts] warn: `.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontWarn::WalkerSkippedSymlink { path } => write!(
                f,
                "skipping symlink entry {} (cycle-safe policy)",
                path.display()
            ),
            FontWarn::WalkerSkippedNonRegular { path, file_type } => write!(
                f,
                "skipping non-regular entry {} (file_type={:?})",
                path.display(),
                file_type
            ),
            FontWarn::WalkerSkippedOversized { path, size, cap } => write!(
                f,
                "skipping oversized font {} ({size} bytes > cap {cap})",
                path.display()
            ),
            FontWarn::ReadRejectedSymlink { path } => {
                write!(f, "skipping {} (path is a symlink)", path.display())
            }
            FontWarn::ReadRejectedNotRegularFile { path } => write!(
                f,
                "skipping {} (path is not a regular file)",
                path.display()
            ),
            FontWarn::ReadRejectedNotRegularFilePostOpen { path } => write!(
                f,
                "skipping {} (opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open))",
                path.display()
            ),
            FontWarn::ReadRejectedOversizedPreOpen { path, size, cap } => write!(
                f,
                "skipping {} (file size {size} bytes exceeds cap {cap} bytes)",
                path.display()
            ),
            FontWarn::ReadRejectedOversizedDuringRead { path, size, cap } => write!(
                f,
                "skipping {} (file grew past cap during read: {size} bytes read, cap {cap} bytes (TOCTOU-grow))",
                path.display()
            ),
            FontWarn::ReadRejectedPathEscape {
                path,
                canonical,
                root,
            } => write!(
                f,
                "skipping {} (canonicalizes to {} which escapes fonts root {})",
                path.display(),
                canonical.display(),
                root.display()
            ),
            FontWarn::ReadRejectedPathEscapePostOpen {
                path,
                canonical,
                root,
            } => write!(
                f,
                "skipping {} (opened fd resolves to {} which escapes fonts root {} (TOCTOU-swap between the open and this containment recheck))",
                path.display(),
                canonical.display(),
                root.display()
            ),
            FontWarn::RegisterEmpty { path } => {
                write!(f, "skipping {}: no family registered", path.display())
            }
        }
    }
}

/// Observer alias — an optional mutable closure receiving a [`FontWarn`] by
/// reference.  Threaded by mutable reference (`&mut FontWarnObserver<'_>`)
/// through the walker so recursion can auto-reborrow rather than moving the
/// `Option` at each site.
type FontWarnObserver<'o> = Option<&'o mut dyn FnMut(&FontWarn<'_>)>;

/// Emit a warn event: call the observer if `Some`, otherwise write the
/// default `eprintln!` line so CLI use continues to see the same output.
///
/// Delegates to the crate-common [`crate::diag::emit_warn_via`] macro so this
/// site and `layout.rs`'s [`crate::layout::LayoutWarn`] silent-clamp
/// diagnostics share one mechanism instead of two independent answers to the
/// same need. See that module's doc for why this is
/// a macro rather than a generic `Observer<W>` type — `FontWarn`'s observer
/// is higher-ranked over `FontWarn`'s own borrowed lifetime, which a
/// monomorphic generic parameter cannot express.
fn emit_warn(observer: &mut FontWarnObserver<'_>, event: FontWarn<'_>) {
    crate::diag::emit_warn_via!(observer, "[raikiri-dom::fonts]", event);
}

/// Map a non-`Io` [`FontReadReject`] to its `FontWarn::ReadRejected*`
/// counterpart.  Extracted as a pure function so the observer-event shape
/// can be unit-tested without needing to trigger a real TOCTOU race: the
/// read-time `ReadRejected*` arms are cov:ignore in the callsite loop
/// (walker pre-filters symlink/non-regular/oversized), so this helper is
/// the sole deterministic coverage of the mapping.
///
/// # Panics
///
/// Debug-asserts on `FontReadReject::Io(_)`: `Io` is hard-propagated as
/// [`FontError::Io`] before the observer emit site, so it must never reach
/// this mapping.  Release builds fall back to a defensive no-op path via
/// `ReadRejectedNotRegularFile` (least-surprising residual); the debug
/// assert exists to catch a future refactor that routes `Io` through here
/// by mistake.
fn read_reject_to_warn<'a>(path: &'a Path, reject: &'a FontReadReject) -> FontWarn<'a> {
    match reject {
        FontReadReject::Symlink => FontWarn::ReadRejectedSymlink { path },
        FontReadReject::NotRegularFile => FontWarn::ReadRejectedNotRegularFile { path },
        FontReadReject::NotRegularFilePostOpen => {
            FontWarn::ReadRejectedNotRegularFilePostOpen { path }
        }
        FontReadReject::OversizedPreOpen { size, cap } => FontWarn::ReadRejectedOversizedPreOpen {
            path,
            size: *size,
            cap: *cap,
        },
        FontReadReject::OversizedDuringRead { size, cap } => {
            FontWarn::ReadRejectedOversizedDuringRead {
                path,
                size: *size,
                cap: *cap,
            }
        }
        FontReadReject::PathEscape { canonical, root } => FontWarn::ReadRejectedPathEscape {
            path,
            canonical: canonical.as_path(),
            root: root.as_path(),
        },
        FontReadReject::PathEscapePostOpen { canonical, root } => {
            FontWarn::ReadRejectedPathEscapePostOpen {
                path,
                canonical: canonical.as_path(),
                root: root.as_path(),
            }
        }
        FontReadReject::Io(_) => {
            debug_assert!(
                false,
                "read_reject_to_warn called with Io(_); Io must be peeled off before observer emit"
            );
            FontWarn::ReadRejectedNotRegularFile { path }
        }
    }
}

/// WPT bundled fonts dir から FontContext を構築する。system font
/// resolver は完全 disable、generic family (`serif`/`sans-serif`/...)
/// は register 済 family の先頭 (Ahem) に解決される。
///
/// Delegates to [`build_wpt_font_ctx_with_observer`] with a `None` observer,
/// preserving the CLI-facing `eprintln!` warn output.  Consumers wanting a
/// structured observer callback for TOCTOU-swap/grow anomalies
/// should call `_with_observer` directly.  Signature preserved
/// for the `crates/raikiri/tests/external_consumer.rs` pin.
///
/// # Errors
///
/// See [`build_wpt_font_ctx_with_observer`].
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError> {
    build_wpt_font_ctx_with_observer(fonts_dir, None)
}

/// WPT bundled fonts dir から FontContext を構築する。system font
/// resolver は完全 disable、generic family (`serif`/`sans-serif`/...)
/// は register 済 family の先頭 (Ahem) に解決される。
///
/// The `observer` receives a [`FontWarn`] for every warn+skip site — walker
/// (symlink / non-regular / oversized), read-time TOCTOU (`ReadRejected*`),
/// and fontique register-empty.  When `None`, warn output falls back to the
/// legacy `eprintln!` shape so CLI use is unaffected.
///
/// # Errors
/// - [`FontError::DirNotFound`] — `fonts_dir` が存在しない
/// - [`FontError::EmptyDir`] — dir は存在するが `.ttf`/`.otf` が 1 個も無い
/// - [`FontError::PreferredFontUnavailable`] — `PREFERRED_FIRST` に list した
///   font (現在は Ahem.ttf) が dir に無い、または fontique が register を
///   拒否した (silent fallback は cascade 決定性を破壊するため Err にする)
/// - [`FontError::NoFontsRegistered`] — dir には `.ttf`/`.otf` があるが 1 個も
///   register できなかった (PREFERRED_FIRST 経路で先に catch されるので
///   PREFERRED_FIRST が空の future 想定でのみ到達)
/// - [`FontError::Io`] — dir walk 中、または font read
///   (callsite-local `read_bounded_font_file` 経由) の Io error propagate
///   (`FontReadReject::Io(_)` -> `FontError::Io`、他 reject reason は warn+skip
///   経由で `FontWarn::ReadRejected*` として observer にも surface)
pub fn build_wpt_font_ctx_with_observer(
    fonts_dir: &Path,
    mut observer: FontWarnObserver<'_>,
) -> Result<FontContext, FontError> {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
    use std::sync::Arc;

    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    // Canonicalize the walker root once, up front, so
    // `read_bounded_font_file`'s containment check compares against a stable
    // fully-resolved prefix.  `fonts_dir.exists()` cleared the DirNotFound
    // path above; a canonicalize failure here would be a TOCTOU race (dir
    // unlinked between check and canonicalize) — propagate as Io rather than
    // panic.
    let canonical_root = std::fs::canonicalize(fonts_dir).map_err(|source|
        // cov:ignore: the Err arm here needs a TOCTOU race (fonts_dir
        // unlinked between the `.exists()` check above and canonicalize)
        // to fire, which is not deterministically unit-testable.  The
        // other TOCTOU-race arms in this module (the open Err arm inside
        // `raikiri_traits::io::open_bounded_regular_file`, the
        // TOCTOU-grow reject, the read-time reject arm below) use the
        // same cov:ignore shape.
        FontError::Io {
            path: fonts_dir.to_path_buf(),
            source,
        })?;
    let paths = walk_fonts(fonts_dir, &mut observer)?;
    if paths.is_empty() {
        return Err(FontError::EmptyDir(fonts_dir.to_path_buf()));
    }

    // blitz pattern (packages/blitz-dom/src/lib.rs::build_single_font_ctx):
    // system_fonts: false で fontique の platform resolver を完全 disable
    let mut ctx = FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };

    // Register 順 = fallback 順。walker が PREFERRED_FIRST を先頭に置く。
    //
    // File read failure は **hard error として propagate**する
    // (Ahem.ttf の silent drop を防ぐため): 従来の eprintln! warn skip では、
    // Ahem.ttf (PREFERRED_FIRST`[0]`) が read failed 時に silently 次候補
    // (CSSTest 等) が register され、cascade "serif" が想定外の font に解決
    // されてしまう。walker が返した path は既に存在確認済 (read_dir で
    // 列挙された) なので、read 段階で失敗するのは permission 変更や symlink
    // 損傷など明確な異常。ここで停止する方が「default で silent regression」
    // より安全。個別 file の fontique reject (register.is_empty) は
    // aggregate check (`family_ids.is_empty` → NoFontsRegistered) が catch する
    // ので warn+skip のまま維持。
    let mut family_ids = Vec::new();
    // PREFERRED_FIRST invariant tracking:
    // path が PREFERRED_FIRST member かつ register 成功したものを basename
    // 単位で記録。loop 後にこの set と PREFERRED_FIRST を照合し、欠落 or
    // register-empty があれば PreferredFontUnavailable。silent fallback を防ぐ。
    let mut registered_preferred_basenames: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for path in paths {
        // Bounded read via `read_bounded_font_file`, which delegates the
        // leaf-swap / kind / size gates to
        // `raikiri_traits::io::open_bounded_regular_file` and layers this
        // module's own path-containment recheck on top (see that
        // function's doc comment).
        //
        // Callsite policy:
        // - `Io(_)` -> hard-error propagate.  Preserves the "Ahem.ttf
        //   silent-fallback prevention" guarantee: an
        //   unexpected Io error at read time aborts the build directly, so
        //   the registry cannot silently drop the preferred font without a
        //   caller-visible error.  `PreferredFontUnavailable` is not the
        //   catch for this branch — it fires only for the warn+skip arm
        //   below (see next bullet).
        // - `Symlink | NotRegularFile | NotRegularFilePostOpen |
        //   OversizedPreOpen | OversizedDuringRead | PathEscape |
        //   PathEscapePostOpen` -> warn+skip.
        //   `collect_recursive` already pre-filtered symlink/non-regular/
        //   oversized, so surfacing here means the tree changed between walk
        //   and read (TOCTOU-swap / TOCTOU-grow / intermediate-symlink swap).
        //   Non-preferred fonts silently drop from the registry; if Ahem.ttf
        //   is affected, `PreferredFontUnavailable` fires downstream on the
        //   aggregate `registered_preferred_basenames` check.
        //   `OversizedDuringRead` closes the silent-truncation window the
        //   prior `take(FONT_SIZE_CAP)` had (the `+1-probe` surfaces
        //   TOCTOU-grow instead of returning a truncated buffer).  `Symlink`
        //   covers the leaf-swap TOCTOU class: safe_open's `O_NOFOLLOW`
        //   ELOOP or the post-error `symlink_metadata` recheck surface a
        //   mid-walk swap-in.  `PathEscape` covers the
        //   intermediate-symlink-swap class (walker recorded
        //   `subdir/font.ttf`, attacker swapped `subdir/` into a symlink to
        //   `/tmp/evil/` between walk and read; the leaf still stats as a
        //   regular file so `is_symlink()` does not fire but canonicalize
        //   resolves outside `canonical_root`) — this closes the
        //   intermediate-directory-swap PathEscape gap.  `NotRegularFilePostOpen`
        //   covers the FIFO/character device / block device swap TOCTOU
        //   class (walker + pre-open metadata saw a regular file, attacker
        //   swapped it for a FIFO / device between pre-open metadata and
        //   `safe_open`).  `O_NONBLOCK` in `safe_open` prevents the
        //   writer-less FIFO from blocking `open()`, and the fd-based
        //   `File::metadata()` fstat inspects the inode already bound to the
        //   descriptor (race-free by construction) — this closes the
        //   FIFO/device leaf-swap gap.  `PathEscapePostOpen` covers
        //   the intermediate-dir-swap subclass that survives even
        //   `PathEscape`'s `canonicalize` check (which itself now runs
        //   after the open, not before): an attacker swaps the
        //   intermediate directory again between the open and this
        //   `canonicalize` check.  The fd-bound
        //   `check_open_handle_containment` recheck (Linux: `/proc/self/fd/
        //   <fd>` readlink; Apple platforms: `fcntl(fd, F_GETPATH, ..)`) is
        //   derived from the fd the open actually returned, so it is
        //   race-free against that second swap by construction — this
        //   closes the intermediate-directory-swap PathEscape residual.
        //
        // Trade-offs recorded:
        // - The walker's symlink_metadata / is_symlink / is_file /
        //   metadata.len checks are re-run inside `read_bounded_font_file`.
        //   Justified by the standalone-safe invariant; the cost is `~O(N)`
        //   extra stat syscalls at init.  Raising `FONT_SIZE_CAP` requires
        //   updating the walker's copy too.
        // - Directory-swap tampering (regular-file -> directory between walk
        //   and read) previously hard-errored via `File::open` EISDIR ->
        //   `FontError::Io`; now surfaces as `NotRegularFile` -> warn+skip.
        //   Signal downgrade for non-preferred fonts; preferred invariant
        //   still fires.
        let bytes = match read_bounded_font_file(&path, &canonical_root, FONT_SIZE_CAP) {
            Ok(bytes) => bytes,
            Err(FontReadReject::Io(source)) => {
                return Err(FontError::Io {
                    path: path.clone(),
                    source,
                });
            }
            // cov:ignore: the walker pre-filters symlink/non-regular/oversized,
            // so surfacing a non-Io `FontReadReject` here requires a real
            // TOCTOU race between walk and read.  The
            // `read_reject_to_warn` mapping is instead unit-tested
            // deterministically via `read_reject_to_warn_maps_all_non_io_variants`.
            Err(reason) => {
                emit_warn(&mut observer, read_reject_to_warn(&path, &reason));
                continue;
            }
        };
        let blob = Blob::new(Arc::new(bytes) as _);
        let registered = ctx.collection.register_fonts(blob, None);
        if registered.is_empty() {
            emit_warn(&mut observer, FontWarn::RegisterEmpty { path: &path });
            continue;
        }
        // register 成功した path が PREFERRED_FIRST 対象なら record
        if let Some(basename) = path.file_name().and_then(|f| f.to_str())
            && PREFERRED_FIRST.contains(&basename)
        {
            registered_preferred_basenames.insert(basename.to_string());
        }
        family_ids.extend(registered.iter().map(|(id, _)| *id));
    }

    // PREFERRED_FIRST invariant enforce:
    // PREFERRED_FIRST 全 member が register 成功したことを確認。missing (walker
    // で拾えなかった) or register-empty (fontique reject) の場合、他の valid
    // font が silent fallback として cascade "serif" に解決されないよう
    // dedicated Err を返す。
    for expected in PREFERRED_FIRST {
        if !registered_preferred_basenames.contains(*expected) {
            return Err(FontError::PreferredFontUnavailable {
                name: (*expected).to_string(),
                dir: fonts_dir.to_path_buf(),
            });
        }
    }

    // Backstop: PREFERRED_FIRST が空である将来 (現在は unreachable path、
    // PREFERRED_FIRST=`["Ahem.ttf"]` の invariant check が先に fire する)
    // に備えた defensive check。cascade "serif" が空の family_ids に対して
    // 何にも解決されない状態を Err で surface する。
    if family_ids.is_empty() {
        return Err(FontError::NoFontsRegistered(fonts_dir.to_path_buf()));
    }

    // Generic family alias remap (blitz pattern):
    // UA CSS default "serif" cascade を bundled family (先頭 = Ahem)
    // に解決させる
    for generic in [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
    ] {
        ctx.collection
            .append_generic_families(generic, family_ids.iter().copied());
    }

    Ok(ctx)
}

/// [`build_wpt_font_ctx`] の error 型。std のみ、`thiserror` 依存なし
/// (raikiri workspace 慣習準拠)。
#[derive(Debug)]
pub enum FontError {
    /// `fonts_dir` が存在しない (fetch 未実行の場合など)
    DirNotFound(PathBuf),
    /// `fonts_dir` は存在するが `.ttf`/`.otf` が 1 個も見つからない
    EmptyDir(PathBuf),
    /// dir に `.ttf`/`.otf` はあったが 1 個も fontique に register されなかった
    /// (全 file が parse-invalid、または pin drift で asset が壊れた等)。
    /// `PREFERRED_FIRST` が空の future 想定でのみ到達する defensive backstop。
    NoFontsRegistered(PathBuf),
    /// `PREFERRED_FIRST` に list された font が dir に存在しない、または
    /// fontique が register を拒否した。
    /// silent fallback で cascade 決定性を破壊しないよう dedicated Err。
    PreferredFontUnavailable {
        /// 期待されたが register 成功しなかった font の basename
        name: String,
        /// scan 対象の fonts dir
        dir: PathBuf,
    },
    /// dir walk 中の io failure、または font read (callsite-local
    /// `read_bounded_font_file` 経由) の `FontReadReject::Io(_)` propagate。
    /// 他の reject reason は warn+skip される (詳細は
    /// [`build_wpt_font_ctx`] の callsite comment 参照 — one source of
    /// truth for the full variant list, so this doc doesn't drift from it)。
    Io {
        /// io error が発生した path (walk 段階または read 段階)
        path: PathBuf,
        /// 元の io error
        source: std::io::Error,
    },
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::DirNotFound(p) => {
                write!(f, "fonts dir not found: {}", p.display())
            }
            FontError::EmptyDir(p) => write!(
                f,
                "fonts dir has no .ttf/.otf files: {} (did you run scripts/wpt/fetch.sh?)",
                p.display()
            ),
            FontError::NoFontsRegistered(p) => write!(
                f,
                "no font families registered from {} (all .ttf/.otf files rejected by parley/fontique — check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)",
                p.display()
            ),
            FontError::PreferredFontUnavailable { name, dir } => write!(
                f,
                "preferred font '{}' not registered under {} — missing from dir or rejected by parley/fontique; silent fallback would break cascade determinism (check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)",
                name,
                dir.display()
            ),
            FontError::Io { path, source } => {
                write!(f, "io error reading {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for FontError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FontError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// PREFERRED_FIRST に list した family は walker sort の結果に関わらず
/// **配列 index 順に**先頭に register される。cascade `"serif"` の
/// resolve 順の決定性と、hello-world VRT visual (Ahem square "Hi") のため。
///
/// 現在は Ahem のみ (fulgur pin では Lato-Regular が不在)。将来 Lato-Medium 等の
/// real-text primary を追加したい場合は array に append する。
const PREFERRED_FIRST: &[&str] = &["Ahem.ttf"];

/// dir を recursive walk して `.ttf`/`.otf` を collect + sort + PREFERRED_FIRST
/// を先頭に move する。file_name の case は `.ttf`/`.otf` (小文字 normalize)。
///
/// Walker warn+skip sites (symlink / non-regular / oversized) route through
/// the shared `observer` so `_with_observer` consumers get walker-level
/// events too, not only read-time TOCTOU events.  When `observer` is `None`
/// the eprintln! fallback lives in [`emit_warn`].
fn walk_fonts(dir: &Path, observer: &mut FontWarnObserver<'_>) -> Result<Vec<PathBuf>, FontError> {
    let mut collected: Vec<PathBuf> = Vec::new();
    collect_recursive(dir, &mut collected, observer)?;
    // 1. path sort (決定性)
    collected.sort();
    // 2. PREFERRED_FIRST を先頭に partition
    let (preferred, rest): (Vec<PathBuf>, Vec<PathBuf>) = collected.into_iter().partition(|p| {
        p.file_name()
            .and_then(|f| f.to_str())
            .map(|n| PREFERRED_FIRST.contains(&n))
            .unwrap_or(false)
    });
    // 3. preferred は PREFERRED_FIRST の配列 index 順に再ソート。
    // 同一 basename が複数 subdir に存在するケース (例: 将来の WPT pin で
    // Ahem.ttf が fonts/ と fonts/CSSTest/ 両方に存在) では **全 match** を
    // drain する — `.find()` を 1 回だけ呼ぶと最初の match 以外が
    // ordered_preferred からも rest からも silently drop されてしまう。
    // `Vec::retain` で target にマッチする要素を全部
    // 抜き取ることで、複数 match を取りこぼさない。
    let mut ordered_preferred: Vec<PathBuf> = Vec::new();
    let mut remaining_preferred = preferred;
    for target in PREFERRED_FIRST {
        let mut matched: Vec<PathBuf> = Vec::new();
        remaining_preferred.retain(|p| {
            if p.file_name().and_then(|f| f.to_str()) == Some(*target) {
                matched.push(p.clone());
                false
            } else {
                true
            }
        });
        ordered_preferred.extend(matched);
    }
    // 4. preferred + rest を結合。remaining_preferred は理論上 empty
    // (partition の条件が PREFERRED_FIRST.contains と一致する為) だが、
    // 万一 unmatched な要素が残っても rest 側に足すことで silent drop を
    // 防ぐための対策。
    let mut result = ordered_preferred;
    result.extend(rest);
    result.extend(remaining_preferred);
    Ok(result)
}

fn collect_recursive(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    observer: &mut FontWarnObserver<'_>,
) -> Result<(), FontError> {
    let entries = std::fs::read_dir(dir).map_err(|source| FontError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    // 各 DirEntry を preserve して `file_type()` で kind を照会する。
    // 過去の `Path::is_dir()` 経由は:
    // (a) symlink を follow するため `fonts/loop -> .` の cycle で無限再帰
    //     → stack overflow abort、
    // (b) metadata error を silently `false` として扱い entry を落とす、
    // という 2 つの穴があった。`file_type()` は symlink を follow せず、
    // io error も Result で返すので propagate 可能。
    let mut entries_with_type: Vec<(PathBuf, std::fs::FileType)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| FontError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| FontError::Io {
            path: path.clone(),
            source,
        })?;
        entries_with_type.push((path, file_type));
    }
    // path sort で decl-order 非依存の決定性を保つ
    entries_with_type.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, file_type) in entries_with_type {
        // symlink (dir でも file でも) は skip: cycle-safe。将来 WPT pin に
        // 意図的な symlink が含まれるようになったら別途 canonicalize+visited
        // set 方式に拡張する。今は WPT font tree は plain hierarchy 前提。
        if file_type.is_symlink() {
            emit_warn(observer, FontWarn::WalkerSkippedSymlink { path: &path });
            continue;
        }
        if file_type.is_dir() {
            collect_recursive(&path, out, observer)?;
            continue;
        }
        // regular file 以外 (FIFO / device / socket / BlockDevice / CharDevice) は
        // skip: FIFO は `std::fs::read` 経由で writer 未定なら無限 block、device は
        // /dev/zero symlink 経路が閉じられた後の直接配置 attack vector。
        // `Read::take(N)` は memory bound しか担保しないので、時間軸の DoS
        // (blocking read) は walk 段階で file_type filter するのが load-bearing。
        if !file_type.is_file() {
            emit_warn(
                observer,
                FontWarn::WalkerSkippedNonRegular {
                    path: &path,
                    file_type,
                },
            );
            continue;
        }
        // 通常 file: extension check
        let is_font = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| {
                let s = s.to_ascii_lowercase();
                s == "ttf" || s == "otf"
            })
            .unwrap_or(false);
        if !is_font {
            continue;
        }
        // Size cap: attacker が用意した巨大 regular font file による memory
        // exhaustion を弾く。境界値 (== FONT_SIZE_CAP) は
        // 通す (build_wpt_font_ctx 側の `take(FONT_SIZE_CAP)` bounded read が
        // 完全 consume するので truncation は起きない)。
        let metadata = std::fs::symlink_metadata(&path).map_err(|source| FontError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.len() > FONT_SIZE_CAP {
            emit_warn(
                observer,
                FontWarn::WalkerSkippedOversized {
                    path: &path,
                    size: metadata.len(),
                    cap: FONT_SIZE_CAP,
                },
            );
            continue;
        }
        out.push(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Minimal valid TTF header (magic 0x00010000 + zero-fill).
    /// fontique の register_fonts は header 検査後 zero-fill body でも
    /// family_id を割り当てる (parse-invalid だが walker exercise には十分)。
    fn write_fake_ttf(dir: &Path, name: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(&[0x00, 0x01, 0x00, 0x00]).unwrap();
        f.write_all(&[0u8; 64]).unwrap();
    }

    #[test]
    fn walker_loads_ttf_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "a.ttf");
        write_fake_ttf(tmp.path(), "b.ttf");
        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(
            paths
                .iter()
                .all(|p| p.extension().and_then(|e| e.to_str()) == Some("ttf"))
        );
    }

    #[test]
    fn walker_ignores_non_font_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "font.ttf");
        std::fs::write(tmp.path().join("README.md"), b"ignore").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"ignore").unwrap();
        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("font.ttf")
        );
    }

    #[test]
    fn walker_recurses_into_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("CSSTest");
        std::fs::create_dir(&sub).unwrap();
        write_fake_ttf(tmp.path(), "top.ttf");
        write_fake_ttf(&sub, "nested.ttf");
        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn walker_sort_order_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        // Non-alphabetical create order to exercise sort
        for name in ["z.ttf", "a.ttf", "m.ttf"] {
            write_fake_ttf(tmp.path(), name);
        }
        let first = walk_fonts(tmp.path(), &mut None).unwrap();
        let second = walk_fonts(tmp.path(), &mut None).unwrap();
        assert_eq!(first, second, "walker must be deterministic across calls");
        let names: Vec<_> = first
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.ttf", "m.ttf", "z.ttf"]);
    }

    #[test]
    fn walker_preferred_first_orders_ahem_before_csstest() {
        let tmp = tempfile::tempdir().unwrap();
        // "AAA-non-preferred.ttf" は plain alphabetical sort だと Ahem.ttf
        // より前に来る ('A' == 'A' だが "AAA" < "Ahem" byte-wise: 'A' < 'h').
        // これを混ぜることで、PREFERRED_FIRST の explicit reorder が本当に
        // 効いていることを検証する (これが無いと Ahem が
        // alphabetically 先頭なだけの偶然と reorder 適用が区別できない)。
        write_fake_ttf(tmp.path(), "AAA-non-preferred.ttf");
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        write_fake_ttf(tmp.path(), "CSSTest-Regular.ttf");
        write_fake_ttf(tmp.path(), "Lato-Bold.ttf");
        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        // Ahem (PREFERRED_FIRST`[0]`) が alphabetically 先頭の
        // AAA-non-preferred.ttf を override → 残りは path sort
        // (AAA-non-preferred, CSSTest, Lato-Bold)
        assert_eq!(
            names,
            vec![
                "Ahem.ttf",
                "AAA-non-preferred.ttf",
                "CSSTest-Regular.ttf",
                "Lato-Bold.ttf",
            ]
        );
    }

    #[test]
    fn walker_handles_duplicate_preferred_basename_in_subdirs() {
        // Regression pin: 同一 basename (Ahem.ttf) が top-level と
        // subdir 両方に存在するケース (将来の WPT pin で fonts/Ahem.ttf +
        // fonts/CSSTest/Ahem.ttf のような構成があり得る)。旧実装は
        // `.find()` を 1 回しか呼ばない為、2 個目以降の match が
        // ordered_preferred からも rest からも silently drop されていた。
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        write_fake_ttf(&sub, "Ahem.ttf");
        write_fake_ttf(tmp.path(), "Other.ttf");

        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        assert_eq!(
            paths.len(),
            3,
            "both Ahem.ttf copies + Other.ttf must survive the walk, got: {:?}",
            paths
        );

        // 両方の Ahem.ttf copy が結果に含まれる (basename 重複でも drop されない)
        let ahem_count = paths
            .iter()
            .filter(|p| p.file_name().and_then(|f| f.to_str()) == Some("Ahem.ttf"))
            .count();
        assert_eq!(
            ahem_count, 2,
            "duplicate Ahem.ttf basenames must both survive"
        );

        // PREFERRED_FIRST の Ahem.ttf 2 個は先頭 2 slot を占める (path sort順:
        // top-level "Ahem.ttf" < "subdir/Ahem.ttf")
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("Ahem.ttf")
        );
        assert_eq!(
            paths[1].file_name().and_then(|f| f.to_str()),
            Some("Ahem.ttf")
        );
        assert_eq!(
            paths[2].file_name().and_then(|f| f.to_str()),
            Some("Other.ttf")
        );
    }

    #[test]
    fn missing_dir_returns_err() {
        // NB: `parley::FontContext` doesn't impl `Debug` (parley 0.10), so
        // `Result::unwrap_err` (which requires `T: Debug`) can't be used
        // here. `match` sidesteps that bound.
        let bogus = Path::new("/definitely/does/not/exist/raikiri-dom-fonts-test");
        match build_wpt_font_ctx(bogus) {
            Err(FontError::DirNotFound(p)) => assert_eq!(p, bogus),
            Err(other) => panic!("expected DirNotFound, got {:?}", other),
            Ok(_) => panic!("expected DirNotFound err, got Ok"),
        }
    }

    #[test]
    fn empty_dir_returns_empty_dir_err() {
        let tmp = tempfile::tempdir().unwrap();
        match build_wpt_font_ctx(tmp.path()) {
            Err(FontError::EmptyDir(p)) => assert_eq!(p, tmp.path()),
            Err(other) => panic!("expected EmptyDir, got {:?}", other),
            Ok(_) => panic!("expected EmptyDir err, got Ok"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn walker_skips_named_pipe_font_entry() {
        // Regression pin: 攻撃者が制御下 fonts dir に
        // `evil.ttf` という名前の FIFO を配置した場合、`std::fs::read` が
        // writer 未定の FIFO で無限 block してしまう。walk 段階で
        // `file_type.is_file()` filter が named pipe を弾くことを pin する。
        // このテストが落ちる = time-DoS surface が再度開いた合図。
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "good.ttf");
        let fifo = tmp.path().join("evil.ttf");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok with FIFO present");
        // FIFO は skip、good.ttf のみ通過
        assert_eq!(
            paths.len(),
            1,
            "expected only good.ttf (FIFO skipped), got: {:?}",
            paths
        );
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("good.ttf")
        );
    }

    #[test]
    fn walker_skips_oversized_font_file() {
        // Regression pin: FONT_SIZE_CAP + 1 byte の
        // sparse regular file (実際には zero-block、`set_len` で logical size
        // のみ膨らむ) を walker が skip することを pin する。
        // sparse file を使うのは、テスト実行時に 100 MiB+ の実 block 消費を
        // 避けるため (metadata.len() は logical size を返すので filter は
        // 正しく発火する)。
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "ok.ttf");
        let big = tmp.path().join("big.ttf");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(FONT_SIZE_CAP + 1).unwrap();

        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok with oversized file");
        assert_eq!(
            paths.len(),
            1,
            "expected only ok.ttf (oversized big.ttf skipped), got: {:?}",
            paths
        );
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("ok.ttf")
        );
    }

    #[test]
    fn walker_accepts_regular_file_at_size_cap_boundary() {
        // Regression pin: filter が silently over-reject していないことを
        // pin する (境界値 == FONT_SIZE_CAP は通す — build_wpt_font_ctx 側の
        // `take(FONT_SIZE_CAP)` bounded read は境界を全 consume する)。
        // boundary.ttf: `File::set_len(FONT_SIZE_CAP)` で sparse file を作り、
        // 境界値ちょうど (`metadata.len() == FONT_SIZE_CAP`) が accept 側に
        // 落ちる (`>` cap で skip、`<= cap` で accept) ことを直接 pin する
        // (tiny file では境界を実際に触れず silent over-reject を
        // 捕捉できないため)。
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("boundary.ttf");
        let f = std::fs::File::create(&path).expect("create boundary.ttf");
        f.set_len(FONT_SIZE_CAP)
            .expect("sparse set_len FONT_SIZE_CAP");
        drop(f);
        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok on boundary-size font");
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("boundary.ttf")
        );
    }

    #[test]
    #[cfg(unix)]
    fn walker_skips_symlink_dirs_no_cycle_overflow() {
        // Regression pin: `Path::is_dir()`
        // が symlink を follow して recursion loop に入る問題。`fonts/loop → .`
        // のような self-cycle でも walker が有限時間で return することを pin。
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "real.ttf");
        // symlink loop: tmp/loop → tmp (self-reference cycle)
        let loop_path = tmp.path().join("loop");
        std::os::unix::fs::symlink(tmp.path(), &loop_path).unwrap();

        // 過去実装 (`Path::is_dir()`) では stack overflow していた
        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok even with symlink cycle");
        // symlink を skip したので real.ttf のみ (loop 経由で発見される
        // 追加 real.ttf は無い)。
        assert_eq!(paths.len(), 1, "expected only real.ttf, got: {:?}", paths);
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("real.ttf")
        );
    }

    #[test]
    fn preferred_font_missing_from_disk_returns_err() {
        // Regression pin: PREFERRED_FIRST
        // font (Ahem.ttf) が dir に存在しない場合、他の valid font (Other.ttf)
        // が silent fallback として cascade "serif" に解決されてはならない。
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Other.ttf"); // valid ttf (fontique accepts)
        // Ahem.ttf は書かない → PREFERRED_FIRST invariant 違反
        match build_wpt_font_ctx(tmp.path()) {
            Err(FontError::PreferredFontUnavailable { name, dir }) => {
                assert_eq!(name, "Ahem.ttf");
                assert_eq!(dir, tmp.path());
            }
            Err(other) => panic!(
                "expected PreferredFontUnavailable(Ahem.ttf), got {:?}",
                other
            ),
            Ok(_) => panic!(
                "expected PreferredFontUnavailable err (Ahem missing from disk), got Ok — \
                 silent non-Ahem fallback would break cascade determinism"
            ),
        }
    }

    #[test]
    fn preferred_font_register_failure_returns_err() {
        // Regression pin: PREFERRED_FIRST
        // font (Ahem.ttf) が disk に存在するが fontique に reject された場合、
        // 他の valid font が silent fallback として cascade "serif" に解決
        // されてはならない (read failure を hard-error に昇格させたのと
        // 同じ精神で、register failure も dedicated Err に昇格)。
        let tmp = tempfile::tempdir().unwrap();
        // Ahem.ttf: garbage bytes → fontique が register 拒否
        std::fs::write(tmp.path().join("Ahem.ttf"), b"not a valid font").unwrap();
        // Other.ttf: valid fake ttf → fontique が register 成功
        write_fake_ttf(tmp.path(), "Other.ttf");
        match build_wpt_font_ctx(tmp.path()) {
            Err(FontError::PreferredFontUnavailable { name, dir }) => {
                assert_eq!(name, "Ahem.ttf");
                assert_eq!(dir, tmp.path());
            }
            Err(other) => panic!(
                "expected PreferredFontUnavailable(Ahem.ttf), got {:?}",
                other
            ),
            Ok(_) => panic!(
                "expected PreferredFontUnavailable err (Ahem present but rejected), got Ok — \
                 silent fallback to Other.ttf would break cascade 'serif' → Ahem promise"
            ),
        }
    }

    /// 実 WPT font (target/wpt/fonts/) を使った integration-style test。
    /// scripts/wpt/fetch.sh 未実行時は skip (should_panic 相当ではなく early return
    /// で clean skip)。
    #[test]
    fn build_wpt_font_ctx_registers_generic_serif() {
        use std::path::PathBuf;

        // Locate target/wpt/fonts (workspace root からの相対)。cargo test 実行時の
        // CWD は crate dir なので `../..` で root。
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let fonts_dir = PathBuf::from(&manifest_dir)
            .join("..")
            .join("..")
            .join("target")
            .join("wpt")
            .join("fonts");

        if !fonts_dir.join("Ahem.ttf").exists() {
            eprintln!(
                "skipping build_wpt_font_ctx_registers_generic_serif: \
                 Ahem.ttf not found under {} \
                 (run scripts/wpt/fetch.sh first)",
                fonts_dir.display()
            );
            return;
        }

        let ctx = build_wpt_font_ctx(&fonts_dir).expect("build Ok with Ahem present");
        // parley 0.10 の resolution API 経由で "serif" generic が非空 family
        // に解決されることを assert する完全な検証は将来の end-to-end VRT
        // が担保する。ここでは build_wpt_font_ctx が real WPT font dir
        // (Ahem.ttf 含む) に対して panic せず Ok を返すことのみを smoke
        // check する (spike scope。controller ambiguity は解決済み)。
        let _ = ctx;
    }

    /// End-to-end pin: `read_bounded_font_file` rejects a symlink at the
    /// pre-open `symlink_metadata` check.  This test does NOT exercise the
    /// `O_NOFOLLOW` path (the open never runs because the pre-open check
    /// short-circuits) — that unit is covered by
    /// `raikiri_traits::io`'s own `safe_open_rejects_symlink_at_open_time`
    /// test.  Kept to pin the full-path behavior against future refactors
    /// that might reorder the checks.
    #[cfg(unix)]
    #[test]
    fn read_bounded_font_file_rejects_symlink_via_pre_open_check() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let target = tmp.path().join("target.ttf");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target")
            .unwrap();
        let link = tmp.path().join("link.ttf");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match read_bounded_font_file(&link, &canonical_root, FONT_SIZE_CAP) {
            Err(FontReadReject::Symlink) => {}
            other => panic!("expected Symlink, got {other:?}"),
        }
    }

    /// Regression pin: O_NOFOLLOW on the internal open path does not reject a
    /// legitimate regular file.  Without this test, an implementation that
    /// broke the safe_open fallback (e.g. accidentally always returning
    /// ELOOP) would be missed by the symlink-only tests.
    #[test]
    fn read_bounded_font_file_accepts_regular_file_with_nofollow() {
        let tmp = tempfile::tempdir().unwrap();
        // `tmp.path()` may contain symlinked components (e.g. macOS
        // /tmp → /private/tmp, or `TMPDIR` on some Linux distros).  The
        // containment check compares a canonicalized leaf against
        // `canonical_root`, so the root must also be canonicalized or the
        // happy path spuriously fires `PathEscape`.
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = tmp.path().join("regular.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"regular content")
            .unwrap();

        let bytes = read_bounded_font_file(&path, &canonical_root, FONT_SIZE_CAP)
            .expect("regular file should be accepted");
        assert_eq!(bytes, b"regular content");
    }

    /// `read_bounded_font_file` rejects a directory as `NotRegularFile`
    /// (structural coverage for the `!file_type.is_file()` branch — the same
    /// arm also fires on FIFO / device / socket paths, whose behavior is
    /// pinned separately by `walker_skips_named_pipe_font_entry`).
    #[test]
    fn read_bounded_font_file_rejects_directory_as_not_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        match read_bounded_font_file(tmp.path(), &canonical_root, FONT_SIZE_CAP) {
            Err(FontReadReject::NotRegularFile) => {}
            other => panic!("expected NotRegularFile, got {other:?}"),
        }
    }

    /// `read_bounded_font_file` rejects an oversized regular file at the
    /// pre-open metadata check (`metadata.len() > cap`).  Uses a small cap
    /// against a tiny file so the test doesn't need `set_len(FONT_SIZE_CAP + 1)`
    /// (that alternative is exercised by `walker_skips_oversized_font_file`).
    #[test]
    fn read_bounded_font_file_rejects_oversized_pre_open() {
        let cap = 4u64;
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = tmp.path().join("too-big.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[0u8; 8])
            .unwrap();

        match read_bounded_font_file(&path, &canonical_root, cap) {
            Err(FontReadReject::OversizedPreOpen { size, cap: c }) => {
                assert_eq!(size, 8);
                assert_eq!(c, cap);
            }
            other => panic!("expected OversizedPreOpen, got {other:?}"),
        }
    }

    /// Happy path: a regular font file inside the canonical root reads
    /// through the containment check without incident. Regression pin —
    /// without this, an implementation that made the `starts_with`
    /// comparison too strict (e.g. required byte-identical paths after
    /// canonicalization but not before) would fail silently on tmpdir
    /// layouts with symlinked prefixes.
    #[test]
    fn read_bounded_font_file_accepts_regular_file_inside_canonical_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = tmp.path().join("inside.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside canonical root")
            .unwrap();

        let bytes = read_bounded_font_file(&path, &canonical_root, FONT_SIZE_CAP)
            .expect("regular file inside canonical root should be accepted");
        assert_eq!(bytes, b"inside canonical root");
    }

    /// Intermediate-symlink escape: an attacker swapped `sub/` (a
    /// directory child of `fonts_dir`) into a symlink pointing at
    /// `outside/` between walk and read. `outside/font.ttf` is a real
    /// regular file, so `symlink_metadata(sub/font.ttf).is_symlink()`
    /// returns `false` (symlink_metadata follows intermediate components,
    /// only refusing to follow the final component) — the leaf-symlink,
    /// non-regular-file, and size gates all pass. The containment check
    /// is the load-bearing defense: canonicalize resolves the leaf to
    /// `outside/font.ttf`, which does not `starts_with(canonical_root)`,
    /// producing `FontReadReject::PathEscape`. Closes the
    /// intermediate-directory-swap PathEscape gap.
    #[cfg(unix)]
    #[test]
    fn read_bounded_font_file_rejects_intermediate_symlink_escape() {
        // Two separate tempdirs so `outside` genuinely lives outside the
        // canonical root of `fonts`.
        let fonts_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(fonts_tmp.path()).unwrap();
        let canonical_outside = std::fs::canonicalize(outside_tmp.path()).unwrap();

        // Real regular-file leaf under the outside dir.
        let outside_leaf = canonical_outside.join("font.ttf");
        std::fs::File::create(&outside_leaf)
            .unwrap()
            .write_all(b"outside content")
            .unwrap();

        // Intermediate directory swap: `fonts/sub` → `outside/`. The leaf
        // path the walker would have recorded is `fonts/sub/font.ttf`.
        let sub_link = fonts_tmp.path().join("sub");
        std::os::unix::fs::symlink(&canonical_outside, &sub_link).unwrap();
        let leaf_via_intermediate = sub_link.join("font.ttf");

        // Sanity check the setup: the leaf itself is *not* a symlink
        // (symlink_metadata follows intermediate components).  If this
        // assertion ever fails the test has stopped exercising the
        // intermediate-symlink shape and is instead exercising the
        // already-covered leaf-symlink shape.
        let leaf_meta = std::fs::symlink_metadata(&leaf_via_intermediate).unwrap();
        assert!(
            !leaf_meta.file_type().is_symlink(),
            "leaf must not be a symlink: symlink_metadata on {} reported symlink=true, \
             which means the test is exercising the leaf-symlink vector rather than the \
             intermediate-symlink vector this test is meant to cover",
            leaf_via_intermediate.display()
        );
        assert!(
            leaf_meta.file_type().is_file(),
            "leaf must be a regular file"
        );

        match read_bounded_font_file(&leaf_via_intermediate, &canonical_root, FONT_SIZE_CAP) {
            Err(FontReadReject::PathEscape { canonical, root }) => {
                assert_eq!(canonical, outside_leaf);
                assert_eq!(root, canonical_root);
            }
            other => panic!("expected PathEscape for intermediate-symlink escape, got {other:?}"),
        }
    }

    /// Nonexistent path is caught by the pre-open `symlink_metadata` gate
    /// and surfaced as the existing `FontReadReject::Io` variant (not
    /// `PathEscape`). Regression pin — without this an implementation
    /// that reordered the canonicalize call before the pre-open gate
    /// would silently reclassify NotFound as an Io-under-canonicalize
    /// (still Io, but with confusing provenance) or worse, the pre-open
    /// error message would move.
    #[test]
    fn read_bounded_font_file_returns_io_for_nonexistent_path() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let missing = tmp.path().join("does-not-exist.ttf");

        match read_bounded_font_file(&missing, &canonical_root, FONT_SIZE_CAP) {
            Err(FontReadReject::Io(source)) => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Io(NotFound) for missing path, got {other:?}"),
        }
    }

    /// `map_reject_reason` translates every `raikiri_traits::io::RejectReason`
    /// variant to its `FontReadReject` counterpart. `NotRegularFilePostOpen`
    /// and `Oversized { phase: DuringRead }` are only reachable through
    /// `read_bounded_font_file` via a genuine TOCTOU race (cov:ignore at
    /// that callsite, same shape as `read_reject_to_warn`'s race-only
    /// variants above), so this is the sole deterministic pin that a future
    /// match reorder can't silently reroute a race-detected reject into the
    /// wildcard `Io` arm (which this module treats as a hard-propagate,
    /// not warn+skip).
    #[test]
    fn map_reject_reason_maps_all_variants() {
        use raikiri_traits::io::{OversizePhase, RejectReason};

        assert!(matches!(
            map_reject_reason(RejectReason::Symlink),
            FontReadReject::Symlink
        ));
        assert!(matches!(
            map_reject_reason(RejectReason::NotRegularFile),
            FontReadReject::NotRegularFile
        ));
        assert!(matches!(
            map_reject_reason(RejectReason::NotRegularFilePostOpen),
            FontReadReject::NotRegularFilePostOpen
        ));
        assert!(matches!(
            map_reject_reason(RejectReason::Oversized {
                size: 42,
                cap: 10,
                phase: OversizePhase::PreOpen,
            }),
            FontReadReject::OversizedPreOpen { size: 42, cap: 10 }
        ));
        assert!(matches!(
            map_reject_reason(RejectReason::Oversized {
                size: 101,
                cap: 100,
                phase: OversizePhase::DuringRead,
            }),
            FontReadReject::OversizedDuringRead {
                size: 101,
                cap: 100
            }
        ));
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(matches!(
            map_reject_reason(RejectReason::Io(io_err)),
            FontReadReject::Io(source) if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
    }

    // ------------------------------------------------------------------
    // Post-open fd-bound containment tests (intermediate-dir-swap TOCTOU
    // between the open and this fd-based recheck itself)
    // ------------------------------------------------------------------
    //
    // Like the FIFO/device swap window the traits-crate tests cover, the
    // actual intermediate-dir swap race — between the open and
    // `check_open_handle_containment`'s fd-based recheck — is not
    // deterministically reachable through the composed function
    // `read_bounded_font_file` — it requires a second attacker action
    // landing in a microseconds-wide window between two syscalls on the
    // same thread. The defense is instead exercised at the primitive
    // level: `raikiri_traits::io::open_bounded_regular_file` +
    // `check_open_handle_containment` called directly with a matching and
    // a mismatching `canonical_root`, proving the fd-derived path used for
    // the check is bound to the actual opened file regardless of what a
    // path string observed at a different point in time would say.

    /// `check_open_handle_containment` accepts a file whose fd resolves
    /// under the given `canonical_root` — the happy-path regression pin
    /// that the `/proc/self/fd/<fd>` readlink does not spuriously reject
    /// files that are, in fact, inside the root.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_open_handle_containment_accepts_file_within_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = canonical_root.join("inside.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside content")
            .unwrap();

        let (file, _len) = raikiri_traits::io::open_bounded_regular_file(&path, FONT_SIZE_CAP)
            .expect("open should succeed on regular file");
        check_open_handle_containment(&file, &canonical_root)
            .expect("fd resolving inside canonical_root must pass containment recheck");
    }

    /// `check_open_handle_containment` rejects a file whose fd resolves
    /// outside the given `canonical_root`. This is the primitive-level pin
    /// of the actual defense: even though the open succeeded (it is a
    /// perfectly regular file, just not under the expected root), the
    /// fd-derived `/proc/self/fd/<fd>` path fails
    /// `starts_with(canonical_root)` and the recheck rejects — proving the
    /// check does not trust a path string, only the fd's own resolved
    /// target. Simulates what an intermediate-dir-swap attacker would
    /// achieve: an fd bound to a location outside `canonical_root`, opened
    /// via a pathname that (at a *different* point in time, e.g. the
    /// `canonicalize` above) resolved inside it. Closes the
    /// intermediate-directory-swap PathEscape residual.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_open_handle_containment_rejects_file_outside_root() {
        let fonts_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(fonts_tmp.path()).unwrap();
        let canonical_outside = std::fs::canonicalize(outside_tmp.path()).unwrap();

        let outside_leaf = canonical_outside.join("font.ttf");
        std::fs::File::create(&outside_leaf)
            .unwrap()
            .write_all(b"outside content")
            .unwrap();

        let (file, _len) =
            raikiri_traits::io::open_bounded_regular_file(&outside_leaf, FONT_SIZE_CAP)
                .expect("open should succeed on regular file");
        match check_open_handle_containment(&file, &canonical_root) {
            Err(FontReadReject::PathEscapePostOpen { canonical, root }) => {
                assert_eq!(canonical, outside_leaf);
                assert_eq!(root, canonical_root);
            }
            other => panic!(
                "expected PathEscapePostOpen for fd resolving outside canonical_root, got {other:?}"
            ),
        }
    }

    /// `FontReadReject::PathEscapePostOpen`'s `Display` output. Constructed
    /// directly (no filesystem I/O) since this is a pure formatting check —
    /// the variant's actual construction sites are pinned by
    /// `check_open_handle_containment_rejects_file_outside_root` above.
    #[test]
    fn font_read_reject_path_escape_post_open_display_contains_paths() {
        let err = FontReadReject::PathEscapePostOpen {
            canonical: PathBuf::from("/outside/leaf.ttf"),
            root: PathBuf::from("/fonts/root"),
        };
        let s = err.to_string();
        assert!(s.contains("/outside/leaf.ttf"), "missing canonical: {s}");
        assert!(s.contains("/fonts/root"), "missing root: {s}");
    }

    // ------------------------------------------------------------------
    // Apple-platform post-open fd-bound containment tests (the
    // Apple-platform arm of the same defense the
    // Linux tests above pin, via `fcntl(fd, F_GETPATH, ..)` instead of
    // `/proc/self/fd/<fd>` readlink).
    // ------------------------------------------------------------------
    //
    // These mirror `check_open_handle_containment_accepts_file_within_root`
    // / `_rejects_file_outside_root` above structurally (same setup, same
    // assertions) — only the underlying platform primitive differs. They
    // do not run in this repo's Linux CI and have never executed against
    // a real Apple toolchain (no Apple CI target exists here yet; see the
    // "Untested in CI" section on the
    // `check_open_handle_containment` Apple-platform impl above for the
    // accepted-risk rationale and the `F_GETPATH` path-form caveat to
    // check first if either of these ever fails on real Apple CI).

    /// `check_open_handle_containment` (Apple-platform arm) accepts a file
    /// whose fd resolves under the given `canonical_root` — the happy-path
    /// regression pin that `fcntl(fd, F_GETPATH, ..)` does not spuriously
    /// reject files that are, in fact, inside the root.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos"
    ))]
    #[test]
    fn check_open_handle_containment_accepts_file_within_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = canonical_root.join("inside.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside content")
            .unwrap();

        let (file, _len) = raikiri_traits::io::open_bounded_regular_file(&path, FONT_SIZE_CAP)
            .expect("open should succeed on regular file");
        check_open_handle_containment(&file, &canonical_root)
            .expect("fd resolving inside canonical_root must pass containment recheck");
    }

    /// `check_open_handle_containment` (Apple-platform arm) rejects a file
    /// whose fd resolves outside the given `canonical_root`. Mirrors the
    /// Linux primitive-level pin: even though the open succeeded (it is a
    /// perfectly regular file, just not under the expected root), the
    /// fd-derived `F_GETPATH` path fails
    /// `starts_with(canonical_root)` and the recheck rejects — proving the
    /// check does not trust a path string, only the fd's own resolved
    /// target.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos"
    ))]
    #[test]
    fn check_open_handle_containment_rejects_file_outside_root() {
        let fonts_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(fonts_tmp.path()).unwrap();
        let canonical_outside = std::fs::canonicalize(outside_tmp.path()).unwrap();

        let outside_leaf = canonical_outside.join("font.ttf");
        std::fs::File::create(&outside_leaf)
            .unwrap()
            .write_all(b"outside content")
            .unwrap();

        let (file, _len) =
            raikiri_traits::io::open_bounded_regular_file(&outside_leaf, FONT_SIZE_CAP)
                .expect("open should succeed on regular file");
        match check_open_handle_containment(&file, &canonical_root) {
            Err(FontReadReject::PathEscapePostOpen { canonical, root }) => {
                assert_eq!(canonical, outside_leaf);
                assert_eq!(root, canonical_root);
            }
            other => panic!(
                "expected PathEscapePostOpen for fd resolving outside canonical_root, got {other:?}"
            ),
        }
    }

    // ------------------------------------------------------------------
    // Observer tests
    // ------------------------------------------------------------------
    //
    // Structural coverage for `build_wpt_font_ctx_with_observer`:
    // - Deterministic walker sites (symlink / non-regular / oversized) fire.
    // - Deterministic fontique register-empty site fires.
    // - Default None-observer path still writes to eprintln! and does not
    //   panic.
    // - The `read_reject_to_warn` mapping covers all non-Io variants
    //   (including `NotRegularFilePostOpen`)
    //   without needing a TOCTOU race (that path is cov:ignore in the loop).

    /// Owned copy of a [`FontWarn`] event for observer test assertions.
    /// `FontWarn<'a>` borrows the path from the walker's iteration variable,
    /// so tests that want to assert *after* the walker returns must snapshot
    /// each event into an owned form.
    #[derive(Debug, PartialEq, Eq)]
    enum OwnedWarn {
        WalkerSkippedSymlink(PathBuf),
        WalkerSkippedNonRegular(PathBuf),
        WalkerSkippedOversized(PathBuf, u64, u64),
        ReadRejectedSymlink(PathBuf),
        ReadRejectedNotRegularFile(PathBuf),
        ReadRejectedNotRegularFilePostOpen(PathBuf),
        ReadRejectedOversizedPreOpen(PathBuf, u64, u64),
        ReadRejectedOversizedDuringRead(PathBuf, u64, u64),
        ReadRejectedPathEscape(PathBuf, PathBuf, PathBuf),
        ReadRejectedPathEscapePostOpen(PathBuf, PathBuf, PathBuf),
        RegisterEmpty(PathBuf),
    }

    impl OwnedWarn {
        fn from_ref(w: &FontWarn<'_>) -> Self {
            match *w {
                FontWarn::WalkerSkippedSymlink { path } => Self::WalkerSkippedSymlink(path.into()),
                FontWarn::WalkerSkippedNonRegular { path, .. } => {
                    Self::WalkerSkippedNonRegular(path.into())
                }
                FontWarn::WalkerSkippedOversized { path, size, cap } => {
                    Self::WalkerSkippedOversized(path.into(), size, cap)
                }
                FontWarn::ReadRejectedSymlink { path } => Self::ReadRejectedSymlink(path.into()),
                FontWarn::ReadRejectedNotRegularFile { path } => {
                    Self::ReadRejectedNotRegularFile(path.into())
                }
                FontWarn::ReadRejectedNotRegularFilePostOpen { path } => {
                    Self::ReadRejectedNotRegularFilePostOpen(path.into())
                }
                FontWarn::ReadRejectedOversizedPreOpen { path, size, cap } => {
                    Self::ReadRejectedOversizedPreOpen(path.into(), size, cap)
                }
                FontWarn::ReadRejectedOversizedDuringRead { path, size, cap } => {
                    Self::ReadRejectedOversizedDuringRead(path.into(), size, cap)
                }
                FontWarn::ReadRejectedPathEscape {
                    path,
                    canonical,
                    root,
                } => Self::ReadRejectedPathEscape(path.into(), canonical.into(), root.into()),
                FontWarn::ReadRejectedPathEscapePostOpen {
                    path,
                    canonical,
                    root,
                } => {
                    Self::ReadRejectedPathEscapePostOpen(path.into(), canonical.into(), root.into())
                }
                FontWarn::RegisterEmpty { path } => Self::RegisterEmpty(path.into()),
            }
        }
    }

    /// Observer fires `WalkerSkippedSymlink` when a symlink entry sits
    /// alongside real fonts.  Regression pin: the walker's cycle-safe skip
    /// must route through the shared observer, not just the legacy
    /// eprintln!.
    #[cfg(unix)]
    #[test]
    fn observer_fires_walker_skipped_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        // symlink loop (self-cycle) — walker will see it via file_type()
        // as a symlink and skip.
        let loop_path = tmp.path().join("loop");
        std::os::unix::fs::symlink(tmp.path(), &loop_path).unwrap();

        let mut events: Vec<OwnedWarn> = Vec::new();
        let mut cb = |w: &FontWarn<'_>| events.push(OwnedWarn::from_ref(w));
        let _ = build_wpt_font_ctx_with_observer(tmp.path(), Some(&mut cb));

        assert!(
            events
                .iter()
                .any(|e| matches!(e, OwnedWarn::WalkerSkippedSymlink(p) if p == &loop_path)),
            "expected WalkerSkippedSymlink({}) in observer events, got: {:?}",
            loop_path.display(),
            events
        );
    }

    /// Observer fires `WalkerSkippedNonRegular` when a FIFO poses as a
    /// `.ttf` file.  Complements `walker_skips_named_pipe_font_entry`
    /// (pre-observer) by pinning that the same skip is now structured.
    #[cfg(unix)]
    #[test]
    fn observer_fires_walker_skipped_non_regular() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        let fifo = tmp.path().join("evil.ttf");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        let mut events: Vec<OwnedWarn> = Vec::new();
        let mut cb = |w: &FontWarn<'_>| events.push(OwnedWarn::from_ref(w));
        let _ = build_wpt_font_ctx_with_observer(tmp.path(), Some(&mut cb));

        assert!(
            events
                .iter()
                .any(|e| matches!(e, OwnedWarn::WalkerSkippedNonRegular(p) if p == &fifo)),
            "expected WalkerSkippedNonRegular({}) in observer events, got: {:?}",
            fifo.display(),
            events
        );
    }

    /// Observer fires `WalkerSkippedOversized` when a `.ttf` grows past
    /// `FONT_SIZE_CAP`.  Sparse `set_len(FONT_SIZE_CAP + 1)` avoids
    /// consuming 100 MiB of test disk; `metadata.len()` returns the logical
    /// size regardless.
    #[test]
    fn observer_fires_walker_skipped_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        let big = tmp.path().join("big.ttf");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(FONT_SIZE_CAP + 1).unwrap();
        drop(f);

        let mut events: Vec<OwnedWarn> = Vec::new();
        let mut cb = |w: &FontWarn<'_>| events.push(OwnedWarn::from_ref(w));
        let _ = build_wpt_font_ctx_with_observer(tmp.path(), Some(&mut cb));

        assert!(
            events.iter().any(|e| matches!(
                e,
                OwnedWarn::WalkerSkippedOversized(p, size, cap)
                    if p == &big && *size == FONT_SIZE_CAP + 1 && *cap == FONT_SIZE_CAP
            )),
            "expected WalkerSkippedOversized({}, {}, {}) in observer events, got: {:?}",
            big.display(),
            FONT_SIZE_CAP + 1,
            FONT_SIZE_CAP,
            events
        );
    }

    /// Observer fires `RegisterEmpty` when a non-preferred `.ttf` contains
    /// garbage bytes that fontique rejects.  Uses `Other.ttf` (not
    /// `Ahem.ttf`) so `PreferredFontUnavailable` does not preempt the
    /// event.
    #[test]
    fn observer_fires_register_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        // Other.ttf: garbage bytes → fontique returns no families.
        let other = tmp.path().join("Other.ttf");
        std::fs::write(&other, b"not a valid font").unwrap();

        let mut events: Vec<OwnedWarn> = Vec::new();
        let mut cb = |w: &FontWarn<'_>| events.push(OwnedWarn::from_ref(w));
        let _ = build_wpt_font_ctx_with_observer(tmp.path(), Some(&mut cb));

        assert!(
            events
                .iter()
                .any(|e| matches!(e, OwnedWarn::RegisterEmpty(p) if p == &other)),
            "expected RegisterEmpty({}) in observer events, got: {:?}",
            other.display(),
            events
        );
    }

    /// Default-None observer path: `build_wpt_font_ctx` (which delegates
    /// with `None`) must not panic and must preserve the original error
    /// classification even when warn+skip sites fire.  Regression pin: the
    /// observer plumbing must not divert the `FontError` return channel or
    /// change the eprintln! fallback in a way that breaks CLI use.
    #[cfg(unix)]
    #[test]
    fn observer_none_path_still_falls_back_to_eprintln_and_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        // Two independent warn+skip triggers (symlink + oversized) so the
        // eprintln! fallback exercises multiple FontWarn arms in one run.
        let loop_path = tmp.path().join("loop");
        std::os::unix::fs::symlink(tmp.path(), &loop_path).unwrap();
        let big = tmp.path().join("big.ttf");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(FONT_SIZE_CAP + 1).unwrap();
        drop(f);

        // `build_wpt_font_ctx` delegates to `_with_observer(_, None)`, so
        // this exercises the eprintln! fallback path end-to-end.  The
        // aggregate result depends on Ahem.ttf presence — the pin is that
        // the call returns *some* Result (Ok or Err) without panicking.
        let _ = build_wpt_font_ctx(tmp.path());
    }

    /// `read_reject_to_warn` maps every non-Io `FontReadReject` variant to
    /// its `FontWarn::ReadRejected*` counterpart.  The read-time observer
    /// arm in `build_wpt_font_ctx_with_observer` is `cov:ignore` (the
    /// walker pre-filters symlink/non-regular/oversized, so the arm only
    /// fires on a real TOCTOU race), which would otherwise leave the
    /// TOCTOU-observability deliverable untested.  This unit test closes
    /// that coverage gap deterministically.
    #[test]
    fn read_reject_to_warn_maps_all_non_io_variants() {
        let path = Path::new("/tmp/fake.ttf");

        assert!(matches!(
            read_reject_to_warn(path, &FontReadReject::Symlink),
            FontWarn::ReadRejectedSymlink { path: p } if p == path
        ));
        assert!(matches!(
            read_reject_to_warn(path, &FontReadReject::NotRegularFile),
            FontWarn::ReadRejectedNotRegularFile { path: p } if p == path
        ));
        // NotRegularFilePostOpen: the read-time arm is cov:ignore
        // (pre-open+O_NONBLOCK+fstat window race required to fire), so
        // this is the sole deterministic pin of the new mapping.
        assert!(matches!(
            read_reject_to_warn(path, &FontReadReject::NotRegularFilePostOpen),
            FontWarn::ReadRejectedNotRegularFilePostOpen { path: p } if p == path
        ));
        assert!(matches!(
            read_reject_to_warn(
                path,
                &FontReadReject::OversizedPreOpen { size: 42, cap: 10 }
            ),
            FontWarn::ReadRejectedOversizedPreOpen { path: p, size: 42, cap: 10 }
                if p == path
        ));
        assert!(matches!(
            read_reject_to_warn(
                path,
                &FontReadReject::OversizedDuringRead {
                    size: 101,
                    cap: 100
                }
            ),
            FontWarn::ReadRejectedOversizedDuringRead { path: p, size: 101, cap: 100 }
                if p == path
        ));
        // PathEscape: since the loop's read-time arm is cov:ignore
        // (walker pre-filter + intermediate-symlink race required to fire),
        // this is the sole deterministic pin of the new mapping.
        let canonical = PathBuf::from("/tmp/outside/font.ttf");
        let root = PathBuf::from("/tmp/fonts");
        assert!(matches!(
            read_reject_to_warn(
                path,
                &FontReadReject::PathEscape {
                    canonical: canonical.clone(),
                    root: root.clone(),
                }
            ),
            FontWarn::ReadRejectedPathEscape {
                path: p,
                canonical: c,
                root: r,
            } if p == path && c == canonical.as_path() && r == root.as_path()
        ));
        // PathEscapePostOpen: the read-time arm requires a genuine race
        // between the open and the fd-based containment recheck (cov:ignore
        // in the loop, same shape as PathEscape above), so this is the sole
        // deterministic pin of the mapping.
        assert!(matches!(
            read_reject_to_warn(
                path,
                &FontReadReject::PathEscapePostOpen {
                    canonical: canonical.clone(),
                    root: root.clone(),
                }
            ),
            FontWarn::ReadRejectedPathEscapePostOpen {
                path: p,
                canonical: c,
                root: r,
            } if p == path && c == canonical.as_path() && r == root.as_path()
        ));
    }

    /// `FontWarn`'s `Display` output must reproduce the pre-observer
    /// `eprintln!` message bodies verbatim.  Warn-message wording is a
    /// behavior-change surface; this test pins
    /// that the None-observer fallback is byte-identical (modulo the
    /// callsite prefix `[raikiri-dom::fonts] warn: `).
    #[test]
    fn font_warn_display_reproduces_legacy_message_bodies() {
        let p = Path::new("/tmp/fake.ttf");

        // Walker sites — bodies from the original eprintln! calls in
        // collect_recursive (symlink / non-regular / oversized).  The
        // {file_type:?} formatting on non-regular uses a locally-inferred
        // FileType via `symlink_metadata` on the tempdir root so the test
        // does not depend on FileType Debug's exact platform shape.
        assert_eq!(
            format!("{}", FontWarn::WalkerSkippedSymlink { path: p }),
            "skipping symlink entry /tmp/fake.ttf (cycle-safe policy)"
        );
        assert_eq!(
            format!(
                "{}",
                FontWarn::WalkerSkippedOversized {
                    path: p,
                    size: 200,
                    cap: 100
                }
            ),
            "skipping oversized font /tmp/fake.ttf (200 bytes > cap 100)"
        );

        // Read-time sites — bodies mirror the original eprintln! wording
        // "skipping <path> (<FontReadReject Display>)".
        assert_eq!(
            format!("{}", FontWarn::ReadRejectedSymlink { path: p }),
            "skipping /tmp/fake.ttf (path is a symlink)"
        );
        assert_eq!(
            format!("{}", FontWarn::ReadRejectedNotRegularFile { path: p }),
            "skipping /tmp/fake.ttf (path is not a regular file)"
        );
        // NotRegularFilePostOpen: message body pins the TOCTOU-swap
        // wording so operators can grep for "TOCTOU-swap" and distinguish
        // this from a mundane pre-open non-regular reject.
        assert_eq!(
            format!(
                "{}",
                FontWarn::ReadRejectedNotRegularFilePostOpen { path: p }
            ),
            "skipping /tmp/fake.ttf (opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open))"
        );
        assert_eq!(
            format!(
                "{}",
                FontWarn::ReadRejectedOversizedPreOpen {
                    path: p,
                    size: 8,
                    cap: 4
                }
            ),
            "skipping /tmp/fake.ttf (file size 8 bytes exceeds cap 4 bytes)"
        );
        assert_eq!(
            format!(
                "{}",
                FontWarn::ReadRejectedOversizedDuringRead {
                    path: p,
                    size: 101,
                    cap: 100
                }
            ),
            "skipping /tmp/fake.ttf (file grew past cap during read: 101 bytes read, cap 100 bytes (TOCTOU-grow))"
        );
        // PathEscapePostOpen: message
        // body pins the TOCTOU-swap wording (mirroring
        // NotRegularFilePostOpen above) so operators can grep for
        // "TOCTOU-swap" and distinguish this fd-bound, race-free reject
        // from `PathEscape`'s path-based one.
        assert_eq!(
            format!(
                "{}",
                FontWarn::ReadRejectedPathEscapePostOpen {
                    path: p,
                    canonical: Path::new("/tmp/outside/font.ttf"),
                    root: Path::new("/tmp/fonts"),
                }
            ),
            "skipping /tmp/fake.ttf (opened fd resolves to /tmp/outside/font.ttf which escapes fonts root /tmp/fonts (TOCTOU-swap between the open and this containment recheck))"
        );

        // Register-empty site — body from the original eprintln! after the
        // fontique register_fonts empty branch.
        assert_eq!(
            format!("{}", FontWarn::RegisterEmpty { path: p }),
            "skipping /tmp/fake.ttf: no family registered"
        );
    }
}
