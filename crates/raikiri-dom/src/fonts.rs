//! Font sources for the inline engine: the WPT bundled font directory
//! (system fonts disabled, so results are deterministic across machines),
//! bundled font bytes, the process-wide system font collection, and the
//! per-document `@font-face` layer.
//!
//! - Fetching is `scripts/wpt/fetch.sh` (a dev prerequisite); this module
//!   only takes the already-fetched `target/wpt/fonts/` as a `Path`
//! - Bundled `.ttf` / `.otf` are registered directly; `@font-face` URL
//!   sources' WOFF/WOFF2 are converted back to sfnt via `wuff` before
//!   registration
//!
//! Reference implementation for the directory walk:
//! fulgur `crates/fulgur-wpt/src/fonts.rs::load_fonts_dir` (walker + sort).
//!
//! Structured warn hook: a [`FontWarnObserver`] receives a [`FontWarn`] for
//! every warn+skip site (walker, read-time TOCTOU, faces the font library
//! rejects); without one, warnings go to stderr.

use raikiri_style::FontFaceRegistry;
use std::path::{Path, PathBuf};

/// Maximum byte count [`build_wpt_font_ctx`] allows when reading an
/// individual font file. 100 MiB leaves ample headroom for real bundled
/// fonts (Ahem: ~12 KiB, Noto CJK: ~20 MiB or so) while still rejecting the
/// memory exhaustion an attacker-supplied oversized regular file would
/// otherwise cause.
///
/// **Threat surface coverage**:
///
/// - **symlink → /dev/zero**: already closed by `collect_recursive`'s own
///   `file_type.is_symlink()` skip
/// - **FIFO / device / socket** (indefinite block): closed at walk time by
///   `collect_recursive`'s own `file_type.is_file()` gate. `Read::take(N)`
///   bounds memory but not time against a writer-less FIFO, so excluding it
///   at walk time is the 1st line of defense. The read-time TOCTOU-swap
///   (regular → FIFO / device substitution) is defended further down in the
///   leaf-swap item below
/// - **oversized regular file** (memory exhaustion): closed by the
///   `metadata.len() > FONT_SIZE_CAP` skip
/// - **mid-read grow (TOCTOU)**: the `+1-probe` inside
///   `raikiri_traits::io::read_bounded_from_open_file` (`take(FONT_SIZE_CAP +
///   1) + post-read bytes.len > cap` reject) prevents silent truncation and
///   surfaces TOCTOU-grow as `OversizedDuringRead`.
/// - **leaf-swap (TOCTOU)**: the vector where a regular file is swapped for
///   a symlink between the walker's and
///   `raikiri_traits::io::open_bounded_regular_file`'s pre-open
///   `symlink_metadata` calls is closed by `open_bounded_regular_file`'s
///   open-time `O_NOFOLLOW` (unix) / reparse-point-aware open (Windows).
///   Either ELOOP (POSIX-compliant; applies on Linux/macOS — FreeBSD is
///   non-compliant across every version and deliberately returns EMLINK
///   instead) or a fallback `symlink_metadata` recheck (FreeBSD/NetBSD/
///   OpenBSD's EMLINK / EFTYPE, or Windows' synthesized reject) is treated
///   as "equivalent to a leaf-swap symlink" and warn+skipped; the resulting
///   drop of Ahem is caught downstream by the aggregate
///   `PreferredFontUnavailable` check.
///
/// - **FIFO / device-swap (TOCTOU) after pre-open metadata**: the vector
///   where a regular file is swapped for a FIFO / character device / block
///   device between the walker's skip and
///   `open_bounded_regular_file`'s pre-open `symlink_metadata` is closed by
///   `open_bounded_regular_file`'s open (unix: `O_NOFOLLOW | O_NONBLOCK`)
///   plus a post-open fd-based fstat. `O_NONBLOCK` prevents `open()` from
///   blocking on a writer-less FIFO (the 1st defense against a time-DoS),
///   and the post-open `File::metadata().file_type().is_file()` fd-based
///   check race-freely rejects a non-regular kind
///   (`NotRegularFilePostOpen`). Unlike the pre-open path-based check, this
///   stats the already-opened fd, so a path-swap TOCTOU cannot bypass it.
///
/// TODO: wire this up to `RenderLimits` in the future.
pub(crate) const FONT_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// Reason `read_bounded_font_file` rejected a candidate font path.
///
/// Mirrors the shape of `raikiri_traits::io::RejectReason` (which this
/// module's leaf-swap / kind / size gates now delegate to via
/// `raikiri_traits::io::open_bounded_regular_file` and
/// `read_bounded_from_open_file`) so the callsite policy (`Io` → propagate,
/// other reasons → warn+skip) reads the same as before the delegation. A
/// local enum is kept alongside the traits crate's `RejectReason` so the
/// `Display` wording and the walker-vs-read warn taxonomy stay specific to
/// the fonts directory.
#[derive(Debug)]
pub(crate) enum FontReadReject {
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
    /// Post-open, fd-bound containment recheck (inside
    /// [`raikiri_traits::io::read_bounded_contained_file`]) found the
    /// already-opened descriptor resolves outside `canonical_root`.
    /// Unlike [`FontReadReject::PathEscape`] (a fresh, path-based
    /// `canonicalize` re-resolution, run after the open and itself
    /// TOCTOU-vulnerable to a second swap before this recheck runs),
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

/// Map a [`raikiri_traits::io::RejectReason`] onto this module's
/// [`FontReadReject`] taxonomy, one variant to one variant.
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
        RejectReason::PathEscape { canonical, root } => {
            FontReadReject::PathEscape { canonical, root }
        }
        RejectReason::PathEscapePostOpen { canonical, root } => {
            FontReadReject::PathEscapePostOpen { canonical, root }
        }
        RejectReason::Io(source) => FontReadReject::Io(source),
        // cov:ignore: unreachable while every current RejectReason variant is
        // matched explicitly above; required for its #[non_exhaustive]
        // contract.
        other => FontReadReject::Io(std::io::Error::other(other.to_string())),
    }
}

/// Read a regular font file that must stay inside the walked fonts
/// directory, via `raikiri_traits::io::read_bounded_contained_file` (leaf
/// symlink / kind / size gates, containment under `canonical_root` including
/// an fd-derived post-open recheck, and a bounded read). `canonical_root` is
/// the pre-canonicalized walker root; the containment checks reject an
/// intermediate directory swapped into a symlink pointing outside it
/// between the walk and the read.
pub(crate) fn read_bounded_font_file(
    path: &Path,
    canonical_root: &Path,
    size_cap: u64,
) -> Result<Vec<u8>, FontReadReject> {
    raikiri_traits::io::read_bounded_contained_file(path, canonical_root, size_cap)
        .map_err(map_reject_reason)
}

/// Structured warn event emitted by [`build_wpt_font_ctx_with_observer`] for
/// every warn+skip site (walker + read-time TOCTOU + fontique register-empty).
/// Consumers pass an `Option<&mut dyn FnMut(&FontWarn<'_>)>` observer to opt
/// into programmatic consumption of these events; the [`build_wpt_font_ctx`]
/// compatibility wrapper omits the observer and keeps the CLI-facing `eprintln!` behavior.
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
    /// (run after the open, see
    /// [`raikiri_traits::io::read_bounded_contained_file`]) re-resolves the pathname a second time
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
pub(crate) type FontWarnObserver<'o> = Option<&'o mut dyn FnMut(&FontWarn<'_>)>;

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
pub(crate) fn emit_warn(observer: &mut FontWarnObserver<'_>, event: FontWarn<'_>) {
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
pub(crate) fn read_reject_to_warn<'a>(path: &'a Path, reject: &'a FontReadReject) -> FontWarn<'a> {
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

/// Build the shared font layer from a verified WPT font directory, with
/// system fonts disabled.
///
/// # Errors
/// See [`FontError`].
pub fn build_wpt_font_collection(
    fonts_dir: &Path,
) -> Result<shodo::font::FontCollection, FontError> {
    crate::layout::ifc::font::wpt_collection(fonts_dir, &shodo::limits::Limits::default(), None)
}

/// Error type for [`build_wpt_font_collection`]. `std`-only, no `thiserror`
/// dependency (per raikiri workspace convention).
#[derive(Debug)]
pub enum FontError {
    /// `fonts_dir` does not exist (e.g. fetch was never run)
    DirNotFound(PathBuf),
    /// `fonts_dir` exists but has zero `.ttf`/`.otf` files
    EmptyDir(PathBuf),
    /// The dir had `.ttf`/`.otf` files, but none registered with fontique
    /// (every file was parse-invalid, or an asset got corrupted by check
    /// drift, etc.). A defensive backstop only reached in a future where
    /// `PREFERRED_FIRST` is empty.
    NoFontsRegistered(PathBuf),
    /// A font listed in `PREFERRED_FIRST` is not present in the dir, or
    /// fontique refused to register it.
    /// A dedicated Err so a silent fallback doesn't break cascade
    /// determinism.
    PreferredFontUnavailable {
        /// Basename of the font that was expected but didn't register
        /// successfully
        name: String,
        /// The fonts dir that was scanned
        dir: PathBuf,
    },
    /// An io failure during the dir walk, or a `FontReadReject::Io(_)`
    /// propagated from a font read (via the callsite-local
    /// `read_bounded_font_file`).
    /// Other reject reasons are warn+skipped (see the callsite comment on
    /// [`build_wpt_font_ctx`] for details — one source of
    /// truth for the full variant list, so this doc doesn't drift from it).
    Io {
        /// Path where the io error occurred (during the walk or read stage)
        path: PathBuf,
        /// The underlying io error
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

/// Families listed in PREFERRED_FIRST are registered at the head, **in
/// array index order**, regardless of the walker's own sort result. This is
/// for the determinism of the `"serif"` cascade's resolve order, and for the
/// hello-world VRT visual (Ahem square "Hi").
///
/// Currently only Ahem (the fulgur check found Lato-Regular absent). Append
/// to the array in the future if a real-text primary such as Lato-Medium
/// needs to be added.
pub(crate) const PREFERRED_FIRST: &[&str] = &["Ahem.ttf"];

/// Recursively walks `dir`, collecting + sorting `.ttf`/`.otf` files and
/// moving PREFERRED_FIRST to the head. `file_name` matching is
/// case-normalized to lowercase for `.ttf`/`.otf`.
///
/// Walker warn+skip sites (symlink / non-regular / oversized) route through
/// the shared `observer` so `_with_observer` consumers get walker-level
/// events too, not only read-time TOCTOU events.  When `observer` is `None`
/// the eprintln! fallback lives in [`emit_warn`].
pub(crate) fn walk_fonts(
    dir: &Path,
    observer: &mut FontWarnObserver<'_>,
) -> Result<Vec<PathBuf>, FontError> {
    let mut collected: Vec<PathBuf> = Vec::new();
    collect_recursive(dir, &mut collected, observer)?;
    // 1. path sort (for determinism)
    collected.sort();
    // 2. partition PREFERRED_FIRST to the front
    let (preferred, rest): (Vec<PathBuf>, Vec<PathBuf>) = collected.into_iter().partition(|p| {
        p.file_name()
            .and_then(|f| f.to_str())
            .map(|n| PREFERRED_FIRST.contains(&n))
            .unwrap_or(false)
    });
    // 3. Re-sort `preferred` into PREFERRED_FIRST's own array index order.
    // When the same basename exists under multiple subdirs (e.g. a future
    // WPT check where Ahem.ttf exists under both fonts/ and
    // fonts/CSSTest/), drain **every match** — calling `.find()` only once
    // would silently drop every match after the first from both
    // ordered_preferred and rest. `Vec::retain` pulls out every element
    // matching `target` so multiple matches are never lost.
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
    // 4. Concatenate preferred + rest. `remaining_preferred` should be
    // empty in theory (the partition condition matches
    // PREFERRED_FIRST.contains exactly), but appending any unmatched
    // leftovers to `rest` guards against a silent drop just in case.
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
    // Preserve each `DirEntry` and query its kind via `file_type()`.
    // The previous approach via `Path::is_dir()` had two holes:
    // (a) it follows symlinks, so a `fonts/loop -> .` cycle recursed
    //     infinitely → stack overflow abort,
    // (b) it silently treated a metadata error as `false` and dropped the
    //     entry.
    // `file_type()` doesn't follow symlinks, and returns io errors as a
    // `Result` so they can be propagated.
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
    // Sort by path to keep determinism independent of declaration order
    entries_with_type.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, file_type) in entries_with_type {
        // Skip symlinks (whether dir or file): cycle-safe. If a future WPT
        // check intentionally includes symlinks, extend this separately to
        // a canonicalize+visited-set approach. For now the WPT font tree is
        // assumed to be a plain hierarchy.
        if file_type.is_symlink() {
            emit_warn(observer, FontWarn::WalkerSkippedSymlink { path: &path });
            continue;
        }
        if file_type.is_dir() {
            collect_recursive(&path, out, observer)?;
            continue;
        }
        // Skip anything that isn't a regular file (FIFO / device / socket /
        // BlockDevice / CharDevice): a FIFO with no writer would block
        // `std::fs::read` indefinitely, and a device is the direct-placement
        // attack vector left once the /dev/zero symlink route is closed.
        // `Read::take(N)` only bounds memory, not time, so filtering by
        // `file_type` at walk time is what's load-bearing against a
        // time-axis DoS (a blocking read).
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
        // Ordinary file: extension check
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
        // Size cap: rejects the memory exhaustion an attacker-supplied
        // oversized regular font file would cause. The boundary value
        // (== FONT_SIZE_CAP) is let through (build_wpt_font_ctx's
        // `take(FONT_SIZE_CAP)` bounded read fully consumes it, so no
        // truncation occurs).
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

// ---------------------------------------------------------------------------
// @font-face application — font selection integration.
// ---------------------------------------------------------------------------

/// Fetch one `src: url(...)` target for [`apply_font_faces`].
///
/// Returns the raw font bytes, or `None` when the URL is unavailable
/// (network deny, missing file, policy rejection — the reason stays with the
/// loader; [`apply_font_faces`] treats every `None` as "try the next
/// source", fail-closed). Size capping is enforced by [`apply_font_faces`]
/// itself ([`FONT_SIZE_CAP`]), not by implementations, so every loader gets
/// the same bound.
pub trait FontFaceLoader {
    /// Load the bytes behind `url` (the raw `src: url(...)` string as
    /// authored — server-absolute, relative, or `data:`; interpretation is
    /// the implementation's job).
    fn load(&self, url: &str) -> Option<Vec<u8>>;
}

/// What [`apply_font_faces`] did with one [`FontFaceRegistry`].
///
/// Family names are the `@font-face` `font-family` values as authored. A
/// family lands in exactly one of the three lists — application is
/// first-resolvable-source-wins per rule, so a family is never both applied
/// and skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFaceApplyReport {
    /// Families whose bytes were registered into the `FontContext` under the
    /// `@font-face` name (a `url(...)` source resolved and fontique
    /// accepted it). Sorted for cross-process determinism (registry
    /// iteration itself is `HashMap` order).
    pub applied: Vec<String>,
    /// `(face family, local target)` pairs whose alias was expanded into
    /// computed `font-family` lists (a `local(...)` source named an
    /// already-registered family). Sorted by face family, same determinism
    /// rationale as [`Self::applied`].
    pub aliased: Vec<(String, String)>,
    /// Families with no resolvable source left (every source unavailable,
    /// oversized, unsupported-container, or fontique-rejected). Sorted, same
    /// rationale. These families keep their existing behavior — typically
    /// the collection fallback — and never error.
    pub skipped: Vec<String>,
}

/// Read a big-endian `u16` without indexing beyond an untrusted container.
fn woff_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// Read a big-endian `u32` without indexing beyond an untrusted container.
fn woff_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Validate the WOFF 1 table directory before calling the decoder.
///
/// The header's `totalSfntSize` alone is not sufficient as an allocation
/// guard: a malformed table entry can claim a larger `origLength`. Recompute
/// the output sfnt size from every table and require the declared file/table
/// bounds to agree before decompression.
fn woff1_within_cap(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"wOFF") || bytes.len() < 44 {
        return false;
    }
    let Some(length) = woff_u32(bytes, 8) else {
        return false;
    };
    let Some(num_tables) = woff_u16(bytes, 12).map(usize::from) else {
        return false;
    };
    let Some(directory_len) = num_tables.checked_mul(20) else {
        return false;
    };
    let Some(directory_end) = 44usize.checked_add(directory_len) else {
        return false;
    };
    let Some(declared_length) = usize::try_from(length).ok() else {
        return false;
    };
    if directory_end > bytes.len()
        || declared_length < directory_end
        || declared_length > bytes.len()
    {
        return false;
    }
    let Some(directory_sfnt_len) = 16usize.checked_mul(num_tables) else {
        return false;
    };
    let Some(mut sfnt_size) = 12usize.checked_add(directory_sfnt_len) else {
        return false;
    };
    for index in 0..num_tables {
        let Some(entry_offset) = index.checked_mul(20) else {
            return false;
        };
        let Some(entry) = 44usize.checked_add(entry_offset) else {
            return false;
        };
        let Some(offset) = woff_u32(bytes, entry + 4).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(comp_length) =
            woff_u32(bytes, entry + 8).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(orig_length) =
            woff_u32(bytes, entry + 12).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(end) = offset.checked_add(comp_length) else {
            return false;
        };
        if offset < directory_end || end > declared_length {
            return false;
        }
        let Some(padded_length) = orig_length.checked_add(3).map(|value| value & !3) else {
            return false;
        };
        let Some(next_size) = sfnt_size.checked_add(padded_length) else {
            return false;
        };
        sfnt_size = next_size;
    }
    woff_u32(bytes, 16).and_then(|declared| usize::try_from(declared).ok()) == Some(sfnt_size)
        && sfnt_size as u64 <= FONT_SIZE_CAP
}

/// Validate the WOFF 2 header's declared output and input bounds.
fn woff2_within_cap(bytes: &[u8]) -> bool {
    bytes.starts_with(b"wOF2")
        && bytes.len() >= 48
        && woff_u32(bytes, 8)
            .and_then(|length| usize::try_from(length).ok())
            .is_some_and(|length| (48..=bytes.len()).contains(&length))
        && woff_u32(bytes, 16).is_some_and(|size| size as u64 <= FONT_SIZE_CAP)
}

/// Decode a web-font container into the sfnt bytes understood by fontique.
///
/// The container signature selects the decoder. `format(...)` values are
/// capability hints rather than byte-format assertions, so a valid sfnt is
/// passed through even when a WOFF/WOFF2 hint is present (as required by CSS
/// Fonts format-specifier tests). A matching WOFF signature is validated before
/// decompression; malformed, oversized, or unsupported containers fail closed.
pub(crate) fn decode_web_font(bytes: Vec<u8>) -> Option<Vec<u8>> {
    let kind = if bytes.starts_with(b"wOFF") {
        Some("woff")
    } else if bytes.starts_with(b"wOF2") {
        Some("woff2")
    } else {
        return Some(bytes);
    };
    let safe = match kind {
        Some("woff") => woff1_within_cap(&bytes),
        Some("woff2") => woff2_within_cap(&bytes),
        _ => false,
    };
    if !safe {
        return None;
    }
    let decoded = match kind {
        Some("woff") => wuff::decompress_woff1(&bytes).ok()?,
        Some("woff2") => wuff::decompress_woff2(&bytes).ok()?,
        _ => return None,
    };
    (decoded.len() as u64 <= FONT_SIZE_CAP).then_some(decoded)
}

pub use crate::layout::ifc::font::BundledFace;

/// Build a shodo shared layer from bundled fonts.
///
/// The fonts are registered in order under their authored family names, and
/// every generic family maps onto the bundle in registration order, so the
/// result does not depend on the fonts installed on the host. The same list
/// given to `FontContextBuilder` in `raikiri-html` resolves a family to the
/// same face on the parley path. With `system_fonts` set, the installed
/// fonts are consulted after the bundle.
///
/// # Errors
/// An empty list, a face shodo rejects, or a resource limit.
pub fn build_bundled_font_collection(
    faces: Vec<BundledFace>,
    system_fonts: bool,
) -> Result<shodo::font::FontCollection, shodo::font::FontError> {
    crate::layout::ifc::font::bundled_collection(
        &shodo::limits::Limits::default(),
        faces,
        system_fonts,
    )
}

/// The process-wide shared layer built from the platform's installed fonts.
///
/// The platform is scanned once; later calls clone the shared layer. Faces
/// are loaded from the platform lazily, the first time a lookup selects them,
/// into this one layer, so every document and every thread of the process
/// shares it and its lock. The per-layer face and byte caps are lifted: with
/// a layer that lives as long as the process, a cap would become a lifetime
/// limit after which lookups silently fall back to the last resort. Callers
/// must not configure it (for example with `set_generic_families`): that
/// changes the answers for every other user of the layer.
pub fn system_font_collection() -> shodo::font::FontCollection {
    static SHARED: std::sync::OnceLock<shodo::font::FontCollection> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let limits = shodo::limits::Limits {
                max_faces_per_layer: None,
                max_layer_blob_bytes: None,
                ..shodo::limits::Limits::default()
            };
            shodo::font::FontCollection::with_options(&limits, shodo::font::FontOptions::default())
        })
        .clone()
}

/// A document layer over `shared` holding the faces of `faces`, for the
/// inline engine.
///
/// Faces are registered under their authored family name in the layer only:
/// the shared layer is left untouched, so one document's faces are never
/// visible to another, and computed `font-family` lists need no rewriting.
/// `loader` fetches `url()` sources; a `local()` source resolves against
/// `shared` by full name or PostScript name.
pub fn build_inline_document_fonts(
    shared: &shodo::font::FontCollection,
    faces: &FontFaceRegistry,
    loader: &dyn FontFaceLoader,
) -> (shodo::font::FontCollection, FontFaceApplyReport) {
    use crate::layout::ifc::font::face::{document_layer, register_font_faces};
    let layer = document_layer(shared, &shodo::limits::Limits::default());
    let report = register_font_faces(&layer, faces, loader);
    (layer, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Minimal valid TTF header (magic 0x00010000 + zero-fill).
    /// fontique's `register_fonts` assigns a `family_id` even to a
    /// zero-fill body once the header check passes (parse-invalid, but
    /// good enough for exercising the walker).
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
        // "AAA-non-preferred.ttf" sorts before Ahem.ttf under plain
        // alphabetical sort ('A' == 'A' but "AAA" < "Ahem" byte-wise:
        // 'A' < 'h'). Mixing this in verifies that PREFERRED_FIRST's
        // explicit reorder is actually taking effect (without it, there'd
        // be no way to distinguish Ahem merely happening to sort first
        // alphabetically from the reorder actually being applied).
        write_fake_ttf(tmp.path(), "AAA-non-preferred.ttf");
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        write_fake_ttf(tmp.path(), "CSSTest-Regular.ttf");
        write_fake_ttf(tmp.path(), "Lato-Bold.ttf");
        let paths = walk_fonts(tmp.path(), &mut None).unwrap();
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        // Ahem (PREFERRED_FIRST`[0]`) overrides AAA-non-preferred.ttf, which
        // is alphabetically first → the rest stay in path sort order
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
        // Regression check: the same basename (Ahem.ttf) exists both at the
        // top level and under a subdir (a future WPT check could plausibly
        // have a layout like fonts/Ahem.ttf + fonts/CSSTest/Ahem.ttf). The
        // old implementation called `.find()` only once, so every match
        // after the first was silently dropped from both ordered_preferred
        // and rest.
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

        // Both Ahem.ttf copies are included in the result (a duplicate
        // basename does not get dropped)
        let ahem_count = paths
            .iter()
            .filter(|p| p.file_name().and_then(|f| f.to_str()) == Some("Ahem.ttf"))
            .count();
        assert_eq!(
            ahem_count, 2,
            "duplicate Ahem.ttf basenames must both survive"
        );

        // The 2 PREFERRED_FIRST Ahem.ttf entries occupy the first 2 slots
        // (path sort order: top-level "Ahem.ttf" < "subdir/Ahem.ttf")
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
        match build_wpt_font_collection(bogus) {
            Err(FontError::DirNotFound(p)) => assert_eq!(p, bogus),
            Err(other) => panic!("expected DirNotFound, got {:?}", other),
            Ok(_) => panic!("expected DirNotFound err, got Ok"),
        }
    }

    #[test]
    fn empty_dir_returns_empty_dir_err() {
        let tmp = tempfile::tempdir().unwrap();
        match build_wpt_font_collection(tmp.path()) {
            Err(FontError::EmptyDir(p)) => assert_eq!(p, tmp.path()),
            Err(other) => panic!("expected EmptyDir, got {:?}", other),
            Ok(_) => panic!("expected EmptyDir err, got Ok"),
        }
    }

    /// `collect_recursive`'s own `std::fs::read_dir` error arm.
    /// `build_wpt_font_ctx` intercepts a missing path earlier via its own
    /// `DirNotFound` check (see `missing_dir_returns_err`), so reaching this
    /// specific arm requires calling `walk_fonts` directly against a path
    /// that exists (passing `.exists()`) but is not a directory.
    #[test]
    fn walk_fonts_on_regular_file_returns_io_err() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("not_a_dir.ttf");
        std::fs::write(&path, b"regular file, not a directory").unwrap();
        match walk_fonts(&path, &mut None) {
            Err(FontError::Io { path: p, .. }) => assert_eq!(p, path),
            other => panic!("expected FontError::Io, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn walker_skips_named_pipe_font_entry() {
        // Regression check: if an attacker places a FIFO named `evil.ttf` in
        // a fonts dir they control, `std::fs::read` would block indefinitely
        // on a writer-less FIFO. Checks that the walk-time
        // `file_type.is_file()` filter rejects a named pipe.
        // If this test fails, that's the signal the time-DoS surface has
        // reopened.
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "good.ttf");
        let fifo = tmp.path().join("evil.ttf");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok with FIFO present");
        // The FIFO is skipped; only good.ttf passes through
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
        // Regression check: verifies the walker skips a FONT_SIZE_CAP + 1
        // byte sparse regular file (actually zero-block; `set_len` only
        // inflates the logical size). A sparse file is used to avoid
        // consuming 100 MiB+ of real blocks while the test runs
        // (`metadata.len()` returns the logical size, so the filter still
        // fires correctly).
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
        // Regression check: verifies the filter doesn't silently
        // over-reject (the boundary value == FONT_SIZE_CAP is let through —
        // build_wpt_font_ctx's `take(FONT_SIZE_CAP)` bounded read fully
        // consumes exactly the boundary). boundary.ttf: creates a sparse
        // file via `File::set_len(FONT_SIZE_CAP)`, then checks directly that
        // exactly the boundary value (`metadata.len() == FONT_SIZE_CAP`)
        // lands on the accept side (`>` cap skips, `<= cap` accepts) —
        // a tiny file would never actually touch the boundary and so
        // couldn't catch a silent over-reject.
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
        // Regression check: `Path::is_dir()` follows symlinks and can enter
        // a recursion loop. Pins that the walker returns in finite time even
        // for a self-cycle like `fonts/loop → .`.
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "real.ttf");
        // symlink loop: tmp/loop → tmp (self-reference cycle)
        let loop_path = tmp.path().join("loop");
        std::os::unix::fs::symlink(tmp.path(), &loop_path).unwrap();

        // The previous implementation (`Path::is_dir()`) would stack overflow here
        let paths = walk_fonts(tmp.path(), &mut None).expect("walker Ok even with symlink cycle");
        // The symlink is skipped, so only real.ttf (no additional real.ttf
        // is discovered via the loop).
        assert_eq!(paths.len(), 1, "expected only real.ttf, got: {:?}", paths);
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("real.ttf")
        );
    }

    #[test]
    fn preferred_font_missing_from_disk_returns_err() {
        // Regression check: when the PREFERRED_FIRST font (Ahem.ttf) is
        // absent from the dir, another valid font (Other.ttf) must not
        // silently fall back into the "serif" cascade.
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Other.ttf"); // valid ttf (fontique accepts)
        // Ahem.ttf is deliberately not written -> violates the PREFERRED_FIRST invariant
        match build_wpt_font_collection(tmp.path()) {
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
        // Regression check: when the PREFERRED_FIRST font (Ahem.ttf) is
        // present on disk but rejected by fontique, another valid font must
        // not silently fall back into the "serif" cascade (register failure
        // is escalated to a dedicated Err in the same spirit as read
        // failure being escalated to a hard error).
        let tmp = tempfile::tempdir().unwrap();
        // Ahem.ttf: garbage bytes -> fontique refuses to register it
        std::fs::write(tmp.path().join("Ahem.ttf"), b"not a valid font").unwrap();
        // Other.ttf: a valid fake ttf -> fontique registers it successfully
        write_fake_ttf(tmp.path(), "Other.ttf");
        match build_wpt_font_collection(tmp.path()) {
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

    /// End-to-end check: `read_bounded_font_file` rejects a symlink at the
    /// pre-open `symlink_metadata` check.  This test does NOT exercise the
    /// `O_NOFOLLOW` path (the open never runs because the pre-open check
    /// short-circuits) — that unit is covered by
    /// `raikiri_traits::io`'s own `safe_open_rejects_symlink_at_open_time`
    /// test.  Kept to check the full-path behavior against future refactors
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

    /// Regression check: O_NOFOLLOW on the internal open path does not reject a
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
    /// through the containment check without incident. Regression check —
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
    /// `PathEscape`). Regression check — without this an implementation
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
    /// variants above), so this is the sole deterministic check that a future
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
        assert!(matches!(
            map_reject_reason(RejectReason::PathEscape {
                canonical: PathBuf::from("/outside/a.ttf"),
                root: PathBuf::from("/fonts"),
            }),
            FontReadReject::PathEscape { canonical, root }
                if canonical == Path::new("/outside/a.ttf") && root == Path::new("/fonts")
        ));
        assert!(matches!(
            map_reject_reason(RejectReason::PathEscapePostOpen {
                canonical: PathBuf::from("/outside/b.ttf"),
                root: PathBuf::from("/fonts"),
            }),
            FontReadReject::PathEscapePostOpen { canonical, root }
                if canonical == Path::new("/outside/b.ttf") && root == Path::new("/fonts")
        ));
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(matches!(
            map_reject_reason(RejectReason::Io(io_err)),
            FontReadReject::Io(source) if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
    }

    /// `FontReadReject::PathEscapePostOpen`'s `Display` output. Constructed
    /// directly (no filesystem I/O) since this is a pure formatting check.
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

    /// `FontWarn::ReadRejectedPathEscape`'s `Display` output (the
    /// non-post-open variant — `font_warn_display_reproduces_legacy_message_bodies`
    /// below only pins `ReadRejectedPathEscapePostOpen`).
    #[test]
    fn font_warn_display_read_rejected_path_escape_contains_paths() {
        let p = Path::new("/tmp/fake.ttf");
        let s = format!(
            "{}",
            FontWarn::ReadRejectedPathEscape {
                path: p,
                canonical: Path::new("/tmp/outside/font.ttf"),
                root: Path::new("/tmp/fonts"),
            }
        );
        assert_eq!(
            s,
            "skipping /tmp/fake.ttf (canonicalizes to /tmp/outside/font.ttf which escapes fonts root /tmp/fonts)"
        );
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

    /// `OwnedWarn::from_ref` maps every `FontWarn::ReadRejected*` variant.
    /// The real observer call sites for these all require a genuine
    /// walk-then-read TOCTOU race (see `build_wpt_font_ctx_with_observer`'s
    /// `Err(reason)` arm, `cov:ignore`d for the same reason), so — mirroring
    /// `read_reject_to_warn_maps_all_non_io_variants`'s direct-construction
    /// approach — this constructs each `FontWarn` variant directly rather
    /// than trying to trigger the race.
    #[test]
    fn owned_warn_from_ref_maps_all_read_rejected_variants() {
        let p = Path::new("/tmp/fake.ttf");
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedSymlink { path: p }),
            OwnedWarn::ReadRejectedSymlink(p.into())
        );
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedNotRegularFile { path: p }),
            OwnedWarn::ReadRejectedNotRegularFile(p.into())
        );
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedNotRegularFilePostOpen { path: p }),
            OwnedWarn::ReadRejectedNotRegularFilePostOpen(p.into())
        );
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedOversizedPreOpen {
                path: p,
                size: 8,
                cap: 4
            }),
            OwnedWarn::ReadRejectedOversizedPreOpen(p.into(), 8, 4)
        );
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedOversizedDuringRead {
                path: p,
                size: 101,
                cap: 100
            }),
            OwnedWarn::ReadRejectedOversizedDuringRead(p.into(), 101, 100)
        );
        let canonical = Path::new("/tmp/outside/font.ttf");
        let root = Path::new("/tmp/fonts");
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedPathEscape {
                path: p,
                canonical,
                root,
            }),
            OwnedWarn::ReadRejectedPathEscape(p.into(), canonical.into(), root.into())
        );
        assert_eq!(
            OwnedWarn::from_ref(&FontWarn::ReadRejectedPathEscapePostOpen {
                path: p,
                canonical,
                root,
            }),
            OwnedWarn::ReadRejectedPathEscapePostOpen(p.into(), canonical.into(), root.into())
        );
    }

    /// Observer fires `WalkerSkippedSymlink` when a symlink entry sits
    /// alongside real fonts.  Regression check: the walker's cycle-safe skip
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
        let _ = crate::layout::ifc::font::wpt_collection(
            tmp.path(),
            &shodo::limits::Limits::default(),
            Some(&mut cb),
        );

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
        let _ = crate::layout::ifc::font::wpt_collection(
            tmp.path(),
            &shodo::limits::Limits::default(),
            Some(&mut cb),
        );

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
        let _ = crate::layout::ifc::font::wpt_collection(
            tmp.path(),
            &shodo::limits::Limits::default(),
            Some(&mut cb),
        );

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
        let _ = crate::layout::ifc::font::wpt_collection(
            tmp.path(),
            &shodo::limits::Limits::default(),
            Some(&mut cb),
        );

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
    /// classification even when warn+skip sites fire.  Regression check: the
    /// observer integration logic must not divert the `FontError` return channel or
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
        // aggregate result depends on Ahem.ttf presence — the check is that
        // the call returns *some* Result (Ok or Err) without panicking.
        let _ = build_wpt_font_collection(tmp.path());
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
        // this is the sole deterministic check of the new mapping.
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
        // this is the sole deterministic check of the new mapping.
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
        // deterministic check of the mapping.
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

    /// `read_reject_to_warn`'s `FontReadReject::Io(_)` arm is a defensive
    /// contract check, not a normal mapping: `Io` is always hard-propagated
    /// as `FontError::Io` before the observer emit site (see this
    /// function's own doc), so reaching this arm at all means a future
    /// refactor routed `Io` through here by mistake. The `debug_assert!`
    /// exists to catch exactly that, so this test drives the arm directly
    /// and checks it actually fires rather than silently falling back.
    /// Debug-assert-only: release builds skip the assert and return the
    /// fallback value instead of panicking, so this is gated the same way.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Io must be peeled off before observer emit")]
    fn read_reject_to_warn_debug_asserts_on_io_variant() {
        let path = Path::new("/tmp/fake.ttf");
        let _ = read_reject_to_warn(path, &FontReadReject::Io(std::io::Error::other("boom")));
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

    // ------------------------------------------------------------------
    // apply_font_faces — @font-face selection integration.
    // ------------------------------------------------------------------

    /// Loader that serves fixed bytes for any URL (records what it saw).
    struct MapLoader {
        bytes: Vec<u8>,
        seen: std::cell::RefCell<Vec<String>>,
    }

    impl MapLoader {
        fn refusing() -> Self {
            Self {
                bytes: Vec::new(),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }
        fn serving(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl super::FontFaceLoader for MapLoader {
        fn load(&self, url: &str) -> Option<Vec<u8>> {
            self.seen.borrow_mut().push(url.to_string());
            if self.bytes.is_empty() {
                return None;
            }
            Some(self.bytes.clone())
        }
    }

    /// Real Ahem.ttf bytes for the positive registration paths. fontique
    /// rejects synthetic headers (a zero-fill body with only a valid sfnt
    /// magic does not register — see `write_fake_ttf`'s own doc for the
    /// walker-only case that *does* tolerate that), so only real font bytes
    /// prove `applied`/`aliased`. Embedded from the same crate's checked-in
    /// `tests/data/text-autospace/Ahem.ttf` fixture (byte-identical to the
    /// WPT-fetched copy) rather than the `target/wpt/fonts/` tree, so these
    /// tests run without a `scripts/wpt/fetch.sh` prerequisite.
    fn checked_in_ahem_bytes() -> Vec<u8> {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/text-autospace/Ahem.ttf"
        ))
        .to_vec()
    }

    /// A shared layer holding only Ahem, the base of the document layers
    /// these tests build.
    fn ahem_shared() -> shodo::font::FontCollection {
        super::build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".to_string(),
                bytes: checked_in_ahem_bytes(),
            }],
            false,
        )
        .expect("Ahem collection")
    }

    #[test]
    fn document_fonts_empty_registry_is_noop() {
        let shared = ahem_shared();
        let faces = FontFaceRegistry::new();
        let report = super::build_inline_document_fonts(&shared, &faces, &MapLoader::refusing()).1;
        assert_eq!(report, super::FontFaceApplyReport::default());
    }

    #[test]
    fn document_fonts_unavailable_url_is_skipped_fail_closed() {
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Custom; src: url(missing.ttf); }",
        );
        assert_eq!(faces.len(), 1);
        let report = super::build_inline_document_fonts(&shared, &faces, &MapLoader::refusing()).1;
        assert!(report.applied.is_empty());
        assert!(report.aliased.is_empty());
        assert_eq!(report.skipped, vec!["Custom".to_string()]);
    }

    #[test]
    fn document_fonts_garbage_bytes_are_rejected_fail_closed() {
        let shared = ahem_shared();
        let faces =
            FontFaceRegistry::from_source("@font-face { font-family: Custom; src: url(bad.ttf); }");
        // Not a font at all — fontique must accept zero families from it.
        let loader = MapLoader::serving(b"definitely not a font".to_vec());
        let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
        assert_eq!(report.skipped, vec!["Custom".to_string()]);
    }

    /// Loader that hands back a fresh `FONT_SIZE_CAP + 1`-byte buffer on
    /// every call, without holding one as a stored/cloned field (a stored
    /// `Vec` this size, plus `MapLoader`'s own clone-on-load, would double
    /// peak memory for no reason here).
    struct OversizedLoader;
    impl super::FontFaceLoader for OversizedLoader {
        fn load(&self, _url: &str) -> Option<Vec<u8>> {
            Some(vec![0u8; (FONT_SIZE_CAP + 1) as usize])
        }
    }

    /// `register_font_face_sources`'s own size-cap skip (checked before
    /// `decode_web_font`, ahead of and independent from the walker's
    /// on-disk size cap for `build_wpt_font_ctx`'s files).
    #[test]
    fn document_fonts_skips_oversized_url_bytes() {
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Custom; src: url(huge.ttf); }",
        );
        let applied = super::build_inline_document_fonts(&shared, &faces, &OversizedLoader)
            .1
            .applied;
        assert!(applied.is_empty());
    }

    /// `register_font_face_sources`'s own `decode_web_font` rejection
    /// fallthrough for a URL source — distinct from
    /// `apply_font_faces_garbage_bytes_are_rejected_fail_closed` above,
    /// whose bytes don't match a WOFF/WOFF2 signature at all (so
    /// `decode_web_font` passes them through unchanged and they're instead
    /// rejected later by fontique). Here the signature matches but the
    /// container is truncated, so `woff1_within_cap` itself fails and
    /// `decode_web_font` returns `None`.
    #[test]
    fn document_fonts_skips_malformed_woff_container() {
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Custom; src: url(bad.woff); }",
        );
        let loader = MapLoader::serving(b"wOFF".to_vec());
        let applied = super::build_inline_document_fonts(&shared, &faces, &loader)
            .1
            .applied;
        assert!(applied.is_empty());
    }

    #[test]
    fn document_fonts_registers_url_bytes_under_face_name() {
        let ahem = checked_in_ahem_bytes();
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Custom; src: url(custom.ttf) format(\"truetype\"); }",
        );
        let loader = MapLoader::serving(ahem);
        let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
        assert_eq!(report.applied, vec!["Custom".to_string()]);
        assert!(report.skipped.is_empty());
        assert!(report.aliased.is_empty());
    }

    #[test]
    fn document_fonts_format_hint_does_not_force_container_decode() {
        let ahem = checked_in_ahem_bytes();
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Hinted; src: url(ahem.ttf) format(\"woff2\"); }",
        );
        let loader = MapLoader::serving(ahem);
        let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
        assert_eq!(report.applied, vec!["Hinted".to_string()]);
    }

    #[test]
    fn document_fonts_malformed_woff2_is_rejected_after_loading() {
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Custom; src: url(custom.woff2) format(\"woff2\"); }",
        );
        let loader = MapLoader::serving(b"unread".to_vec());
        let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
        assert_eq!(report.skipped, vec!["Custom".to_string()]);
        assert_eq!(
            loader.seen.borrow().as_slice(),
            &["custom.woff2".to_string()]
        );
    }

    #[test]
    fn document_fonts_registers_woff_and_woff2_assets() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let root = std::path::PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("target")
            .join("wpt")
            .join("fonts");
        let cases = [
            ("WoffOne", "Revalia.woff", Some("woff")),
            (
                "WoffTwo",
                "noto/NotoNaskhArabic-regular.woff2",
                Some("woff2"),
            ),
            (
                "WoffTwoVariations",
                "noto/NotoNaskhArabic-regular.woff2",
                Some("woff2-variations"),
            ),
            ("WoffTwoNoHint", "noto/NotoNaskhArabic-regular.woff2", None),
        ];
        for (family, relative, hint) in cases {
            let path = root.join(relative);
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!(
                    "skipping WOFF registration test: {} not found",
                    path.display()
                );
                return;
            };
            let shared = ahem_shared();
            let source = match hint {
                Some(format) => format!(
                    "@font-face {{ font-family: {family}; src: url({relative}) format(\"{format}\"); }}"
                ),
                None => format!("@font-face {{ font-family: {family}; src: url({relative}); }}"),
            };
            let faces = FontFaceRegistry::from_source(&source);
            let loader = MapLoader::serving(bytes);
            let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
            assert_eq!(report.applied, vec![family.to_string()]);
            assert!(report.skipped.is_empty());
            assert_eq!(loader.seen.borrow().as_slice(), &[relative.to_string()]);
        }
    }

    #[test]
    fn document_fonts_falls_through_from_bad_woff2_to_ttf() {
        struct FallbackLoader {
            good: Vec<u8>,
            seen: std::cell::RefCell<Vec<String>>,
        }

        impl super::FontFaceLoader for FallbackLoader {
            fn load(&self, url: &str) -> Option<Vec<u8>> {
                self.seen.borrow_mut().push(url.to_string());
                if url == "bad.woff2" {
                    Some(b"not a web font".to_vec())
                } else {
                    Some(self.good.clone())
                }
            }
        }

        let ahem = checked_in_ahem_bytes();
        let shared = ahem_shared();
        let faces = FontFaceRegistry::from_source(
            "@font-face { font-family: Fallback; src: url(bad.woff2) format(\"woff2\"), url(good.ttf) format(\"truetype\"); }",
        );
        let loader = FallbackLoader {
            good: ahem,
            seen: std::cell::RefCell::new(Vec::new()),
        };
        let report = super::build_inline_document_fonts(&shared, &faces, &loader).1;
        assert_eq!(report.applied, vec!["Fallback".to_string()]);
        assert_eq!(
            loader.seen.borrow().as_slice(),
            &["bad.woff2".to_string(), "good.ttf".to_string()]
        );
    }

    // ------------------------------------------------------------------
    // FontError / FontReadReject — Display and Error::source coverage.
    // ------------------------------------------------------------------

    #[test]
    fn font_error_display_reproduces_expected_message_bodies() {
        let dir = PathBuf::from("/fonts/wpt");
        assert_eq!(
            format!("{}", FontError::DirNotFound(dir.clone())),
            "fonts dir not found: /fonts/wpt"
        );
        assert_eq!(
            format!("{}", FontError::EmptyDir(dir.clone())),
            "fonts dir has no .ttf/.otf files: /fonts/wpt (did you run scripts/wpt/fetch.sh?)"
        );
        assert_eq!(
            format!("{}", FontError::NoFontsRegistered(dir.clone())),
            "no font families registered from /fonts/wpt (all .ttf/.otf files rejected by parley/fontique — check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)"
        );
        assert_eq!(
            format!(
                "{}",
                FontError::PreferredFontUnavailable {
                    name: "Ahem.ttf".to_string(),
                    dir: dir.clone(),
                }
            ),
            "preferred font 'Ahem.ttf' not registered under /fonts/wpt — missing from dir or rejected by parley/fontique; silent fallback would break cascade determinism (check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)"
        );
        // The wrapped io::Error's own Display text is not this module's
        // contract to pin exactly, so only the fixed prefix/suffix wording
        // and the source message's presence are asserted.
        let io_display = format!(
            "{}",
            FontError::Io {
                path: dir,
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            }
        );
        assert!(io_display.starts_with("io error reading /fonts/wpt: "));
        assert!(io_display.contains("denied"));
    }

    #[test]
    fn font_error_source_returns_io_error_only_for_io_variant() {
        use std::error::Error as _;
        let io_err = FontError::Io {
            path: PathBuf::from("/fonts/wpt/Ahem.ttf"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        };
        let source = io_err.source().expect("Io variant must expose its source");
        assert!(source.to_string().contains("missing"));

        let non_io_err = FontError::EmptyDir(PathBuf::from("/fonts/wpt"));
        assert!(non_io_err.source().is_none());
    }

    /// `FontReadReject::PathEscapePostOpen`'s Display is already pinned by
    /// `font_read_reject_path_escape_post_open_display_contains_paths` above;
    /// this covers the remaining seven variants.
    #[test]
    fn font_read_reject_display_reproduces_expected_message_bodies() {
        assert_eq!(FontReadReject::Symlink.to_string(), "path is a symlink");
        assert_eq!(
            FontReadReject::NotRegularFile.to_string(),
            "path is not a regular file"
        );
        assert_eq!(
            FontReadReject::NotRegularFilePostOpen.to_string(),
            "opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open)"
        );
        assert_eq!(
            FontReadReject::OversizedPreOpen { size: 8, cap: 4 }.to_string(),
            "file size 8 bytes exceeds cap 4 bytes"
        );
        assert_eq!(
            FontReadReject::OversizedDuringRead {
                size: 101,
                cap: 100
            }
            .to_string(),
            "file grew past cap during read: 101 bytes read, cap 100 bytes (TOCTOU-grow)"
        );
        assert_eq!(
            FontReadReject::PathEscape {
                canonical: PathBuf::from("/outside/leaf.ttf"),
                root: PathBuf::from("/fonts/root"),
            }
            .to_string(),
            "canonicalizes to /outside/leaf.ttf which escapes fonts root /fonts/root"
        );
        let display = FontReadReject::Io(std::io::Error::other("boom")).to_string();
        assert!(display.starts_with("I/O error: "));
        assert!(display.contains("boom"));
    }

    // ------------------------------------------------------------------
    // woff1_within_cap / woff2_within_cap / decode_web_font — WOFF
    // container validation, exercised with fully synthetic bytes (no
    // filesystem fixtures) so every arithmetic-bounds branch can be
    // perturbed independently.
    // ------------------------------------------------------------------

    /// Build a well-formed, self-consistent WOFF1 buffer: 44-byte header +
    /// one 20-byte table-directory entry + `table_data`, stored
    /// uncompressed (`compLength == origLength`, wuff's convention for "no
    /// compression applied to this table" — see `decompress_woff1`'s
    /// `is_compressed` check). Every length field is derived from
    /// `table_data`, so a test can start from this and perturb exactly the
    /// field it wants to test. `table_data.len()` should stay a multiple of
    /// 4 so the caller doesn't have to reason about sfnt padding.
    fn build_woff1(table_data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"wOFF"); // signature
        buf.extend_from_slice(&0u32.to_be_bytes()); // flavor
        buf.extend_from_slice(&0u32.to_be_bytes()); // length (patched below)
        buf.extend_from_slice(&1u16.to_be_bytes()); // numTables
        buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
        buf.extend_from_slice(&0u32.to_be_bytes()); // totalSfntSize (patched below)
        buf.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
        buf.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaOffset
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaLength
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaOrigLength
        buf.extend_from_slice(&0u32.to_be_bytes()); // privOffset
        buf.extend_from_slice(&0u32.to_be_bytes()); // privLength
        assert_eq!(buf.len(), 44, "WOFF1 header must be exactly 44 bytes");

        const DIRECTORY_END: u32 = 64; // 44-byte header + one 20-byte entry
        buf.extend_from_slice(b"TEST"); // tag
        buf.extend_from_slice(&DIRECTORY_END.to_be_bytes()); // offset
        buf.extend_from_slice(&(table_data.len() as u32).to_be_bytes()); // compLength
        buf.extend_from_slice(&(table_data.len() as u32).to_be_bytes()); // origLength (== compLength: stored uncompressed)
        buf.extend_from_slice(&0u32.to_be_bytes()); // origChecksum (unchecked by wuff's decoder)
        assert_eq!(buf.len(), DIRECTORY_END as usize);

        buf.extend_from_slice(table_data);
        let total_len = buf.len() as u32;
        buf[8..12].copy_from_slice(&total_len.to_be_bytes());

        let padded = (table_data.len() as u32).div_ceil(4) * 4;
        let sfnt_size = 12 + 16 + padded; // sfnt header + one 16-byte sfnt dir entry + table bytes
        buf[16..20].copy_from_slice(&sfnt_size.to_be_bytes());

        buf
    }

    /// Build a 48-byte WOFF2 header only — `woff2_within_cap` never reads
    /// past the header, so no table directory or compressed data is needed.
    /// `num_tables` is written but deliberately NOT inspected by
    /// `woff2_within_cap` (only by wuff's real parser), which is what makes
    /// `decode_web_font_rejects_woff2_that_fails_to_decompress` below useful.
    fn build_woff2_header(length: u32, num_tables: u16, total_sfnt_size: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"wOF2"); // signature
        buf.extend_from_slice(&0u32.to_be_bytes()); // flavor
        buf.extend_from_slice(&length.to_be_bytes()); // length
        buf.extend_from_slice(&num_tables.to_be_bytes()); // numTables
        buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
        buf.extend_from_slice(&total_sfnt_size.to_be_bytes()); // totalSfntSize
        buf.extend_from_slice(&0u32.to_be_bytes()); // totalCompressedSize
        buf.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
        buf.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaOffset
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaLength
        buf.extend_from_slice(&0u32.to_be_bytes()); // metaOrigLength
        buf.extend_from_slice(&0u32.to_be_bytes()); // privOffset
        buf.extend_from_slice(&0u32.to_be_bytes()); // privLength
        assert_eq!(buf.len(), 48, "WOFF2 header must be exactly 48 bytes");
        buf
    }

    #[test]
    fn woff1_within_cap_accepts_well_formed_header() {
        assert!(woff1_within_cap(&build_woff1(b"ABCDEFGH")));
    }

    #[test]
    fn woff1_within_cap_rejects_short_buffer() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes.truncate(43); // one byte short of the 44-byte header
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_missing_signature() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[0..4].copy_from_slice(b"OTTO");
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_directory_exceeding_buffer() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        // Truncate below the declared table-directory end (64 bytes) while
        // leaving the >=44-byte header-length check satisfied.
        bytes.truncate(50);
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_declared_length_below_directory_end() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[8..12].copy_from_slice(&60u32.to_be_bytes()); // < the 64-byte directory end
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_declared_length_exceeding_buffer() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[8..12].copy_from_slice(&1000u32.to_be_bytes());
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_table_offset_inside_directory() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        // Claim the table's data starts at byte 40 -- inside the
        // header/directory region rather than after it (an overlap a
        // well-formed WOFF1 can never have).
        bytes[48..52].copy_from_slice(&40u32.to_be_bytes());
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_table_end_exceeding_declared_length() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[52..56].copy_from_slice(&1000u32.to_be_bytes()); // compLength
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_sfnt_size_mismatch() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[16..20].copy_from_slice(&999u32.to_be_bytes()); // wrong totalSfntSize
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff1_within_cap_rejects_oversized_declared_sfnt_size() {
        // No compressed-data section is needed (compLength stays 0) -- only
        // the directory's declared origLength needs to push the recomputed
        // sfnt size past FONT_SIZE_CAP.
        let mut bytes = build_woff1(&[]);
        let huge_orig_length: u32 = 200 * 1024 * 1024;
        bytes[56..60].copy_from_slice(&huge_orig_length.to_be_bytes()); // origLength
        let sfnt_size = 12u64 + 16 + huge_orig_length as u64; // already 4-byte aligned
        assert!(sfnt_size > FONT_SIZE_CAP);
        // totalSfntSize matches the recomputed size, so only the cap check
        // (not the equality check) can be what rejects this buffer.
        bytes[16..20].copy_from_slice(&(sfnt_size as u32).to_be_bytes());
        assert!(!woff1_within_cap(&bytes));
    }

    #[test]
    fn woff2_within_cap_accepts_well_formed_header() {
        assert!(woff2_within_cap(&build_woff2_header(48, 1, 1000)));
    }

    #[test]
    fn woff2_within_cap_rejects_missing_signature() {
        let mut bytes = build_woff2_header(48, 1, 1000);
        bytes[0..4].copy_from_slice(b"wOFF");
        assert!(!woff2_within_cap(&bytes));
    }

    #[test]
    fn woff2_within_cap_rejects_short_buffer() {
        let mut bytes = build_woff2_header(48, 1, 1000);
        bytes.truncate(47);
        assert!(!woff2_within_cap(&bytes));
    }

    #[test]
    fn woff2_within_cap_rejects_declared_length_below_header_size() {
        assert!(!woff2_within_cap(&build_woff2_header(40, 1, 1000)));
    }

    #[test]
    fn woff2_within_cap_rejects_declared_length_exceeding_buffer() {
        assert!(!woff2_within_cap(&build_woff2_header(1000, 1, 1000)));
    }

    #[test]
    fn woff2_within_cap_rejects_oversized_total_sfnt_size() {
        let over_cap = FONT_SIZE_CAP as u32 + 1;
        assert!(!woff2_within_cap(&build_woff2_header(48, 1, over_cap)));
    }

    #[test]
    fn decode_web_font_passes_through_unrecognized_signature() {
        // Neither a WOFF nor a WOFF2 signature -- e.g. a bare sfnt (TTF/OTF)
        // already resolved by `local()`/direct bytes. `format(...)` hints
        // are capability hints, not byte-format assertions (CSS Fonts 4
        // §4.2), so this must pass through unchanged rather than reject.
        let bytes = b"not a web font container at all".to_vec();
        let decoded = decode_web_font(bytes.clone())
            .expect("unrecognized signature must pass through unchanged");
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn decode_web_font_rejects_malformed_woff1_container() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        bytes[16..20].copy_from_slice(&999u32.to_be_bytes()); // wrong totalSfntSize
        assert!(!woff1_within_cap(&bytes), "fixture must fail the cap gate");
        assert!(decode_web_font(bytes).is_none());
    }

    #[test]
    fn decode_web_font_rejects_malformed_woff2_container() {
        let bytes = build_woff2_header(1000, 1, 1000); // declared length exceeds actual buffer
        assert!(!woff2_within_cap(&bytes), "fixture must fail the cap gate");
        assert!(decode_web_font(bytes).is_none());
    }

    #[test]
    fn decode_web_font_rejects_woff1_that_fails_to_decompress() {
        let mut bytes = build_woff1(b"ABCDEFGH");
        // woff1_within_cap only checks structural bounds, not the reserved
        // field -- flip it nonzero so the container passes this module's
        // cap/bounds gate but wuff's own parser (which the WOFF1 spec
        // requires to reject a nonzero reserved field) still refuses it.
        bytes[14..16].copy_from_slice(&1u16.to_be_bytes());
        assert!(
            woff1_within_cap(&bytes),
            "reserved field is not part of the cap/bounds gate"
        );
        assert!(decode_web_font(bytes).is_none());
    }

    #[test]
    fn decode_web_font_rejects_woff2_that_fails_to_decompress() {
        // woff2_within_cap never inspects numTables; a zero count passes the
        // cap/bounds gate but wuff's header parser requires at least one
        // table and refuses to decode.
        let bytes = build_woff2_header(48, 0, 1000);
        assert!(
            woff2_within_cap(&bytes),
            "numTables is not part of the cap/bounds gate"
        );
        assert!(decode_web_font(bytes).is_none());
    }

    #[test]
    fn decode_web_font_decodes_well_formed_woff1() {
        let bytes = build_woff1(b"ABCDEFGH");
        assert!(
            woff1_within_cap(&bytes),
            "fixture must be a valid WOFF1 container"
        );
        let decoded = decode_web_font(bytes).expect("well-formed uncompressed WOFF1 must decode");
        // sfnt header (12 bytes) + one 16-byte sfnt directory entry +
        // the 8-byte table body, exactly the totalSfntSize declared above.
        assert_eq!(decoded.len(), 36);
        assert_eq!(&decoded[28..36], b"ABCDEFGH");
    }

    // ------------------------------------------------------------------
    // font_face_weight_override / font_face_style_override — descriptor to
    // fontique-override mapping.
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // expand_font_face_alias / remove_unavailable_ch_family — direct calls
    // exercising branches not reached by the apply_font_faces integration
    // tests above (all synthetic; no FontContext/WPT fixture involved,
    // since neither function ever consults `fonts.collection`).
    // ------------------------------------------------------------------

    #[test]
    fn wpt_font_collection_resolves_ahem() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/text-autospace");
        let collection = build_wpt_font_collection(&dir).expect("collection");
        let query = shodo::font::FontQuery {
            families: vec![shodo::style::FontFamily::Named("Ahem".to_owned())],
            ..Default::default()
        };
        assert!(collection.match_cluster(&query, "a").is_some());
    }

    #[test]
    fn wpt_font_collection_reports_a_missing_directory() {
        let error = build_wpt_font_collection(Path::new("/nonexistent-fonts")).unwrap_err();
        assert!(matches!(error, FontError::DirNotFound(_)));
    }

    const AHEM_BYTES: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace/Ahem.ttf"
    ));

    #[test]
    fn a_bundled_collection_resolves_its_family_and_every_generic_to_the_bundle() {
        let collection = build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".to_owned(),
                bytes: AHEM_BYTES.to_vec(),
            }],
            false,
        )
        .expect("collection");
        for family in [
            shodo::style::FontFamily::Named("Ahem".to_owned()),
            shodo::style::FontFamily::Generic(shodo::style::GenericFamily::Serif),
            shodo::style::FontFamily::Generic(shodo::style::GenericFamily::Monospace),
        ] {
            let query = shodo::font::FontQuery {
                families: vec![family.clone()],
                ..Default::default()
            };
            let matched = collection
                .match_cluster(&query, "a")
                .unwrap_or_else(|| panic!("{family:?} resolves"));
            let data = collection.font_data(matched.id).expect("font data");
            assert_eq!(data.data.as_ref(), AHEM_BYTES, "{family:?}");
        }
    }

    #[test]
    fn an_empty_bundle_is_refused() {
        assert!(build_bundled_font_collection(Vec::new(), false).is_err());
    }

    #[test]
    fn the_system_collection_is_built_once_and_shared() {
        let first = system_font_collection();
        let second = system_font_collection();
        // Every layer gets an identity of its own, so equal identities mean
        // the second call returned the layer the first call built.
        assert_eq!(first.layer_handle().id(), second.layer_handle().id());
    }
}
