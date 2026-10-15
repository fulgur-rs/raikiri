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
use raikiri_traits::io::RejectReason;
use std::path::{Path, PathBuf};

/// Maximum byte count [`build_wpt_font_collection`] allows when reading an
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

/// Read one walked font file, warning and skipping the candidates the read
/// stage rejects.
///
/// Delegates to [`raikiri_traits::io::read_bounded_contained_file`] (leaf
/// symlink / kind / size gates, containment under `canonical_root` including
/// an fd-derived post-open recheck, and a bounded read). `canonical_root` is
/// the pre-canonicalized walker root; the containment checks reject an
/// intermediate directory swapped into a symlink pointing outside it
/// between the walk and the read.
///
/// Returns `Ok(None)` after emitting [`FontWarn::ReadRejected`] for a
/// symlink, non-regular, oversized, or escaping candidate. An I/O failure is
/// a hard [`FontError::Io`], so a missing Ahem never falls back silently. So
/// is any reason this module does not classify yet ([`RejectReason`] is
/// `#[non_exhaustive]`): an unknown reason fails closed rather than being
/// downgraded to warn+skip.
pub(crate) fn read_font_file(
    path: &Path,
    canonical_root: &Path,
    size_cap: u64,
    observer: &mut FontWarnObserver<'_>,
) -> Result<Option<Vec<u8>>, FontError> {
    match raikiri_traits::io::read_bounded_contained_file(path, canonical_root, size_cap) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(reason) if is_skipped_read_reject(&reason) => {
            emit_warn(
                observer,
                FontWarn::ReadRejected {
                    path,
                    reason: &reason,
                },
            );
            Ok(None)
        }
        Err(reason) => Err(FontError::Io {
            path: path.to_path_buf(),
            source: reject_into_io(reason),
        }),
    }
}

/// Whether [`read_font_file`] warns about and skips a candidate rejected for
/// `reason`. The walker already filtered these kinds, so each one means the
/// tree changed between walk and read; skipping one file keeps the rest of
/// the collection. `Io` and any reason added to [`RejectReason`] later are
/// hard errors.
fn is_skipped_read_reject(reason: &RejectReason) -> bool {
    matches!(
        reason,
        RejectReason::Symlink
            | RejectReason::NotRegularFile
            | RejectReason::NotRegularFilePostOpen
            | RejectReason::Oversized { .. }
            | RejectReason::PathEscape { .. }
            | RejectReason::PathEscapePostOpen { .. }
    )
}

/// The I/O error a hard-failing read reject propagates as: `Io`'s own
/// source, or an unclassified reason's message wrapped as
/// [`std::io::ErrorKind::Other`].
fn reject_into_io(reason: RejectReason) -> std::io::Error {
    match reason {
        RejectReason::Io(source) => source,
        other => std::io::Error::other(other.to_string()),
    }
}

/// Structured warn event emitted while building the WPT font collection
/// (`ifc::font::wpt_collection`) for every warn+skip site (walker +
/// read-time TOCTOU + font-library register-empty). Callers pass an
/// `Option<&mut dyn FnMut(&FontWarn<'_>)>` observer to opt into
/// programmatic consumption of these events; [`build_wpt_font_collection`]
/// omits the observer and keeps the CLI-facing `eprintln!` behavior.
///
/// # Design
///
/// Sibling convention: `#[non_exhaustive]` mirrors the taxonomy enums in
/// `crates/raikiri-traits/src/error.rs`.  Variants are
/// **split by callsite** (walker vs read) because the walker-vs-read
/// distinction is the exact TOCTOU signal that matters here: a walker
/// `Symlink` is a mundane cycle-safe skip, whereas a read-time
/// [`RejectReason::Symlink`] means the tree changed between walk and read
/// (leaf-swap TOCTOU).  Collapsing them would destroy that signal.  The read
/// stage carries the shared [`RejectReason`] instead of a parallel taxonomy.
///
/// # Not surfaced
///
/// [`RejectReason::Io`] never becomes a `FontWarn` event.  It is
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
    /// Read stage rejected a candidate the walker had already accepted.
    /// The walker pre-filters symlinks, non-regular entries, and oversized
    /// files, so any reason here means the tree changed between walk and
    /// read (a leaf swap, an intermediate-directory swap, or a grow past the
    /// cap). `reason` is never [`RejectReason::Io`], which is
    /// hard-propagated as [`FontError::Io`].
    ReadRejected {
        /// Path the walker accepted and the read stage rejected.
        path: &'a Path,
        /// Why the read stage rejected it.
        reason: &'a RejectReason,
    },
    /// the font library registered no family for the read blob
    /// (parse-invalid font, corrupt asset, etc.).  Handled by the aggregate
    /// [`FontError::PreferredFontUnavailable`] / [`FontError::NoFontsRegistered`]
    /// invariant checks downstream; the observer is the only per-file
    /// programmatic signal.
    RegisterEmpty {
        /// Path whose blob the font library rejected with an empty family list.
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
            FontWarn::ReadRejected { path, reason } => match reason {
                // The two escape reasons keep their fonts-root wording, which
                // differs from `RejectReason`'s own `Display`.
                RejectReason::PathEscape { canonical, root } => write!(
                    f,
                    "skipping {} (canonicalizes to {} which escapes fonts root {})",
                    path.display(),
                    canonical.display(),
                    root.display()
                ),
                RejectReason::PathEscapePostOpen { canonical, root } => write!(
                    f,
                    "skipping {} (opened fd resolves to {} which escapes fonts root {} (TOCTOU-swap between the open and this containment recheck))",
                    path.display(),
                    canonical.display(),
                    root.display()
                ),
                other => write!(f, "skipping {} ({other})", path.display()),
            },
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
    /// The dir had `.ttf`/`.otf` files, but none registered with the font library
    /// (every file was parse-invalid, or an asset got corrupted by check
    /// drift, etc.). A defensive backstop only reached in a future where
    /// `PREFERRED_FIRST` is empty.
    NoFontsRegistered(PathBuf),
    /// A font listed in `PREFERRED_FIRST` is not present in the dir, or
    /// the font library refused to register it.
    /// A dedicated Err so a silent fallback doesn't break cascade
    /// determinism.
    PreferredFontUnavailable {
        /// Basename of the font that was expected but didn't register
        /// successfully
        name: String,
        /// The fonts dir that was scanned
        dir: PathBuf,
    },
    /// An io failure during the dir walk, or a [`RejectReason::Io`] (or a
    /// reject reason this module does not classify) propagated from a font
    /// read.
    /// The reject reasons a read warns about and skips instead are listed in
    /// one place, the read stage's [`is_skipped_read_reject`].
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
                "no font families registered from {} (all .ttf/.otf files rejected by the font library — check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)",
                p.display()
            ),
            FontError::PreferredFontUnavailable { name, dir } => write!(
                f,
                "preferred font '{}' not registered under {} — missing from dir or rejected by the font library; silent fallback would break cascade determinism (check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)",
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
        // (== FONT_SIZE_CAP) is let through (build_wpt_font_collection's
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

/// Fetch one `src: url(...)` target for [`build_inline_document_fonts`].
///
/// Returns the raw font bytes, or `None` when the URL is unavailable
/// (network deny, missing file, policy rejection — the reason stays with the
/// loader; [`build_inline_document_fonts`] treats every `None` as "try the next
/// source", fail-closed). Size capping is enforced by [`build_inline_document_fonts`]
/// itself ([`FONT_SIZE_CAP`]), not by implementations, so every loader gets
/// the same bound.
pub trait FontFaceLoader {
    /// Load the bytes behind `url` (the raw `src: url(...)` string as
    /// authored — server-absolute, relative, or `data:`; interpretation is
    /// the implementation's job).
    fn load(&self, url: &str) -> Option<Vec<u8>>;
}

/// What [`build_inline_document_fonts`] did with one [`FontFaceRegistry`].
///
/// Family names are the `@font-face` `font-family` values as authored. A
/// family lands in exactly one of the three lists — application is
/// first-resolvable-source-wins per rule, so a family is never both applied
/// and skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFaceApplyReport {
    /// Families whose bytes were registered into the document layer under the
    /// `@font-face` name (a `url(...)` source resolved and the font library
    /// accepted it). Sorted for cross-process determinism (registry
    /// iteration itself is `HashMap` order).
    pub applied: Vec<String>,
    /// `(face family, local target)` pairs whose alias was expanded into
    /// computed `font-family` lists (a `local(...)` source named an
    /// already-registered family). Sorted by face family, same determinism
    /// rationale as [`Self::applied`].
    pub aliased: Vec<(String, String)>,
    /// Families with no resolvable source left (every source unavailable,
    /// oversized, unsupported-container, or rejected by the font library). Sorted, same
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

/// Decode a web-font container into the sfnt bytes understood by the font library.
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
/// given to `FontCollectionBuilder` in `raikiri-html` builds the same
/// collection. With `system_fonts` set, the installed
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
mod tests;
