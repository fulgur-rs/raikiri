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
/// Delegates to `raikiri_traits::io::read_bounded_contained_file` (leaf
/// symlink / kind / size gates, containment under `canonical_root` including
/// an fd-derived post-open recheck, and a bounded read). `canonical_root` is
/// the pre-canonicalized walker root; the containment checks reject an
/// intermediate directory swapped into a symlink pointing outside it
/// between the walk and the read.
///
/// Returns `Ok(None)` after emitting [`FontWarn::ReadRejected`] for a
/// symlink, non-regular, oversized, or escaping candidate. An I/O failure is
/// a hard [`FontError::Io`], so a missing Ahem never falls back silently. So
/// is any reason this module does not classify yet (`RejectReason` is
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
/// the collection. `Io` and any reason added to `RejectReason` later are
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
    /// one place, the read stage's `is_skipped_read_reject`.
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
mod tests {
    use super::*;
    use raikiri_traits::io::OversizePhase;
    use std::io::Write;
    use std::path::Path;

    /// Minimal valid TTF header (magic 0x00010000 + zero-fill).
    /// the font library registers a family even for a
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
    /// `build_wpt_font_collection` intercepts a missing path earlier via its own
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
        // build_wpt_font_collection's `take(FONT_SIZE_CAP)` bounded read fully
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
        write_fake_ttf(tmp.path(), "Other.ttf"); // valid ttf (the font library accepts)
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
        // present on disk but rejected by the font library, another valid font must
        // not silently fall back into the "serif" cascade (register failure
        // is escalated to a dedicated Err in the same spirit as read
        // failure being escalated to a hard error).
        let tmp = tempfile::tempdir().unwrap();
        // Ahem.ttf: garbage bytes -> the font library refuses to register it
        std::fs::write(tmp.path().join("Ahem.ttf"), b"not a valid font").unwrap();
        // Other.ttf: a valid fake ttf -> the font library registers it successfully
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

    /// Run [`read_font_file`] with a capturing observer and return its result
    /// alongside the warn events it emitted.
    fn read_font_file_with_events(
        path: &Path,
        canonical_root: &Path,
        size_cap: u64,
    ) -> (Result<Option<Vec<u8>>, FontError>, Vec<OwnedWarn>) {
        let mut events: Vec<OwnedWarn> = Vec::new();
        let result = {
            let mut cb = |w: &FontWarn<'_>| events.push(OwnedWarn::from_ref(w));
            read_font_file(path, canonical_root, size_cap, &mut Some(&mut cb))
        };
        (result, events)
    }

    /// End-to-end check: `read_font_file` rejects a symlink at the
    /// pre-open `symlink_metadata` check.  This test does NOT exercise the
    /// `O_NOFOLLOW` path (the open never runs because the pre-open check
    /// short-circuits) — that unit is covered by
    /// `raikiri_traits::io`'s own `safe_open_rejects_symlink_at_open_time`
    /// test.  Kept to check the full-path behavior against future refactors
    /// that might reorder the checks.
    #[cfg(unix)]
    #[test]
    fn read_font_file_rejects_symlink_via_pre_open_check() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let target = tmp.path().join("target.ttf");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target")
            .unwrap();
        let link = tmp.path().join("link.ttf");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let (result, events) = read_font_file_with_events(&link, &canonical_root, FONT_SIZE_CAP);
        assert!(
            matches!(result, Ok(None)),
            "expected a skip, got {result:?}"
        );
        assert_eq!(
            events,
            [OwnedWarn::ReadRejected(
                link.clone(),
                format!("skipping {} (path is a symlink)", link.display())
            )]
        );
    }

    /// Regression check: O_NOFOLLOW on the internal open path does not reject a
    /// legitimate regular file.  Without this test, an implementation that
    /// broke the safe_open fallback (e.g. accidentally always returning
    /// ELOOP) would be missed by the symlink-only tests.
    #[test]
    fn read_font_file_accepts_regular_file_with_nofollow() {
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

        let (result, events) = read_font_file_with_events(&path, &canonical_root, FONT_SIZE_CAP);
        let bytes = result
            .expect("regular file should be accepted")
            .expect("regular file should not be skipped");
        assert_eq!(bytes, b"regular content");
        assert!(events.is_empty(), "unexpected warns: {events:?}");
    }

    /// `read_font_file` rejects a directory as `NotRegularFile`
    /// (structural coverage for the `!file_type.is_file()` branch — the same
    /// arm also fires on FIFO / device / socket paths, whose behavior is
    /// pinned separately by `walker_skips_named_pipe_font_entry`).
    #[test]
    fn read_font_file_rejects_directory_as_not_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let (result, events) =
            read_font_file_with_events(tmp.path(), &canonical_root, FONT_SIZE_CAP);
        assert!(
            matches!(result, Ok(None)),
            "expected a skip, got {result:?}"
        );
        assert_eq!(
            events,
            [OwnedWarn::ReadRejected(
                tmp.path().into(),
                format!(
                    "skipping {} (path is not a regular file)",
                    tmp.path().display()
                )
            )]
        );
    }

    /// `read_font_file` rejects an oversized regular file at the
    /// pre-open metadata check (`metadata.len() > cap`).  Uses a small cap
    /// against a tiny file so the test doesn't need `set_len(FONT_SIZE_CAP + 1)`
    /// (that alternative is exercised by `walker_skips_oversized_font_file`).
    #[test]
    fn read_font_file_rejects_oversized_pre_open() {
        let cap = 4u64;
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = tmp.path().join("too-big.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[0u8; 8])
            .unwrap();

        let (result, events) = read_font_file_with_events(&path, &canonical_root, cap);
        assert!(
            matches!(result, Ok(None)),
            "expected a skip, got {result:?}"
        );
        assert_eq!(
            events,
            [OwnedWarn::ReadRejected(
                path.clone(),
                format!(
                    "skipping {} (file size 8 bytes exceeds cap 4 bytes)",
                    path.display()
                )
            )]
        );
    }

    /// Happy path: a regular font file inside the canonical root reads
    /// through the containment check without incident. Regression check —
    /// without this, an implementation that made the `starts_with`
    /// comparison too strict (e.g. required byte-identical paths after
    /// canonicalization but not before) would fail silently on tmpdir
    /// layouts with symlinked prefixes.
    #[test]
    fn read_font_file_accepts_regular_file_inside_canonical_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let path = tmp.path().join("inside.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside canonical root")
            .unwrap();

        let bytes = read_font_file_with_events(&path, &canonical_root, FONT_SIZE_CAP)
            .0
            .expect("regular file inside canonical root should be accepted")
            .expect("regular file inside canonical root should not be skipped");
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
    /// producing `RejectReason::PathEscape`. Closes the
    /// intermediate-directory-swap PathEscape gap.
    #[cfg(unix)]
    #[test]
    fn read_font_file_rejects_intermediate_symlink_escape() {
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

        let (result, events) =
            read_font_file_with_events(&leaf_via_intermediate, &canonical_root, FONT_SIZE_CAP);
        assert!(
            matches!(result, Ok(None)),
            "expected a skip for intermediate-symlink escape, got {result:?}"
        );
        assert_eq!(
            events,
            [OwnedWarn::ReadRejected(
                leaf_via_intermediate.clone(),
                format!(
                    "skipping {} (canonicalizes to {} which escapes fonts root {})",
                    leaf_via_intermediate.display(),
                    outside_leaf.display(),
                    canonical_root.display()
                )
            )]
        );
    }

    /// Nonexistent path is caught by the pre-open `symlink_metadata` gate
    /// and surfaced as a hard `FontError::Io` (not a
    /// `PathEscape`). Regression check — without this an implementation
    /// that reordered the canonicalize call before the pre-open gate
    /// would silently reclassify NotFound as an Io-under-canonicalize
    /// (still Io, but with confusing provenance) or worse, the pre-open
    /// error message would move.
    #[test]
    fn read_font_file_returns_io_for_nonexistent_path() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let missing = tmp.path().join("does-not-exist.ttf");

        let (result, events) = read_font_file_with_events(&missing, &canonical_root, FONT_SIZE_CAP);
        match result {
            Err(FontError::Io { path, source }) => {
                assert_eq!(path, missing);
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Io(NotFound) for missing path, got {other:?}"),
        }
        assert!(events.is_empty(), "unexpected warns: {events:?}");
    }

    /// Every reject reason the walker pre-filters is warned about and
    /// skipped, including the ones only a walk-then-read race produces, and
    /// `Io` is not. A reason dropped from the skip list would otherwise turn
    /// a single swapped file into a failure of the whole collection.
    #[test]
    fn is_skipped_read_reject_skips_every_pre_filtered_reason() {
        let skipped = [
            RejectReason::Symlink,
            RejectReason::NotRegularFile,
            RejectReason::NotRegularFilePostOpen,
            RejectReason::Oversized {
                size: 8,
                cap: 4,
                phase: OversizePhase::PreOpen,
            },
            RejectReason::Oversized {
                size: 101,
                cap: 100,
                phase: OversizePhase::DuringRead,
            },
            RejectReason::PathEscape {
                canonical: PathBuf::from("/outside/a.ttf"),
                root: PathBuf::from("/fonts"),
            },
            RejectReason::PathEscapePostOpen {
                canonical: PathBuf::from("/outside/b.ttf"),
                root: PathBuf::from("/fonts"),
            },
        ];
        for reason in &skipped {
            assert!(is_skipped_read_reject(reason), "{reason:?} must be skipped");
        }
        assert!(!is_skipped_read_reject(&RejectReason::Io(
            std::io::Error::other("boom")
        )));
    }

    /// `reject_into_io` passes an `Io` reason's own error through and folds
    /// any other reason into an `Other` error carrying its message, which is
    /// how a reason `read_font_file` does not classify fails closed.
    #[test]
    fn reject_into_io_keeps_io_and_folds_other_reasons() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let source = reject_into_io(RejectReason::Io(io_err));
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(source.to_string(), "denied");

        let folded = reject_into_io(RejectReason::Symlink);
        assert_eq!(folded.kind(), std::io::ErrorKind::Other);
        assert_eq!(folded.to_string(), "path is a symlink");
    }

    /// `FontWarn::ReadRejected`'s `Display` output for a `PathEscape` reason
    /// keeps the fonts-root wording rather than `RejectReason`'s own.
    #[test]
    fn font_warn_display_read_rejected_path_escape_contains_paths() {
        let p = Path::new("/tmp/fake.ttf");
        let s = format!(
            "{}",
            FontWarn::ReadRejected {
                path: p,
                reason: &RejectReason::PathEscape {
                    canonical: PathBuf::from("/tmp/outside/font.ttf"),
                    root: PathBuf::from("/tmp/fonts"),
                },
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
    // Structural coverage for `ifc::font::wpt_collection`:
    // - Deterministic walker sites (symlink / non-regular / oversized) fire.
    // - Deterministic font-library register-empty site fires.
    // - Default None-observer path still writes to eprintln! and does not
    //   panic.
    // - `read_font_file` warns with the read-stage reason; the
    //   `read_font_file_*` tests drive it directly, without needing a
    //   walk-then-read TOCTOU race.

    /// Owned copy of a [`FontWarn`] event for observer test assertions.
    /// `FontWarn<'a>` borrows the path from the walker's iteration variable,
    /// so tests that want to assert *after* the walker returns must snapshot
    /// each event into an owned form.
    #[derive(Debug, PartialEq, Eq)]
    enum OwnedWarn {
        WalkerSkippedSymlink(PathBuf),
        WalkerSkippedNonRegular(PathBuf),
        WalkerSkippedOversized(PathBuf, u64, u64),
        /// Path and the full warn message, as operators see it.
        ReadRejected(PathBuf, String),
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
                FontWarn::ReadRejected { path, .. } => {
                    Self::ReadRejected(path.into(), w.to_string())
                }
                FontWarn::RegisterEmpty { path } => Self::RegisterEmpty(path.into()),
            }
        }
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
    /// garbage bytes that the font library rejects.  Uses `Other.ttf` (not
    /// `Ahem.ttf`) so `PreferredFontUnavailable` does not preempt the
    /// event.
    #[test]
    fn observer_fires_register_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        // Other.ttf: garbage bytes → the font library returns no families.
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

    /// Default-None observer path: `build_wpt_font_collection` (which delegates
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

        // `build_wpt_font_collection` delegates to `wpt_collection(_, _, None)`, so
        // this exercises the eprintln! fallback path end-to-end.  The
        // aggregate result depends on Ahem.ttf presence — the check is that
        // the call returns *some* Result (Ok or Err) without panicking.
        let _ = build_wpt_font_collection(tmp.path());
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
        // "skipping <path> (<reject reason>)".
        let read_rejected = |reason: RejectReason| {
            format!(
                "{}",
                FontWarn::ReadRejected {
                    path: p,
                    reason: &reason,
                }
            )
        };
        assert_eq!(
            read_rejected(RejectReason::Symlink),
            "skipping /tmp/fake.ttf (path is a symlink)"
        );
        assert_eq!(
            read_rejected(RejectReason::NotRegularFile),
            "skipping /tmp/fake.ttf (path is not a regular file)"
        );
        // NotRegularFilePostOpen: message body pins the TOCTOU-swap
        // wording so operators can grep for "TOCTOU-swap" and distinguish
        // this from a mundane pre-open non-regular reject.
        assert_eq!(
            read_rejected(RejectReason::NotRegularFilePostOpen),
            "skipping /tmp/fake.ttf (opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open))"
        );
        assert_eq!(
            read_rejected(RejectReason::Oversized {
                size: 8,
                cap: 4,
                phase: OversizePhase::PreOpen,
            }),
            "skipping /tmp/fake.ttf (file size 8 bytes exceeds cap 4 bytes)"
        );
        assert_eq!(
            read_rejected(RejectReason::Oversized {
                size: 101,
                cap: 100,
                phase: OversizePhase::DuringRead,
            }),
            "skipping /tmp/fake.ttf (file grew past cap during read: 101 bytes read, cap 100 bytes (TOCTOU-grow))"
        );
        // PathEscapePostOpen: message
        // body pins the TOCTOU-swap wording (mirroring
        // NotRegularFilePostOpen above) so operators can grep for
        // "TOCTOU-swap" and distinguish this fd-bound, race-free reject
        // from `PathEscape`'s path-based one.
        assert_eq!(
            read_rejected(RejectReason::PathEscapePostOpen {
                canonical: PathBuf::from("/tmp/outside/font.ttf"),
                root: PathBuf::from("/tmp/fonts"),
            }),
            "skipping /tmp/fake.ttf (opened fd resolves to /tmp/outside/font.ttf which escapes fonts root /tmp/fonts (TOCTOU-swap between the open and this containment recheck))"
        );

        // Register-empty site — body from the original eprintln! after the
        // font library's empty-registration branch.
        assert_eq!(
            format!("{}", FontWarn::RegisterEmpty { path: p }),
            "skipping /tmp/fake.ttf: no family registered"
        );
    }

    // ------------------------------------------------------------------
    // build_inline_document_fonts — @font-face registration.
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

    /// Real Ahem.ttf bytes for the positive registration paths. The font library
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
        // Not a font at all — the font library must accept zero families from it.
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

    /// `build_inline_document_fonts`'s own size-cap skip (checked before
    /// `decode_web_font`, ahead of and independent from the walker's
    /// on-disk size cap for `build_wpt_font_collection`'s files).
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

    /// `build_inline_document_fonts`'s own `decode_web_font` rejection
    /// fallthrough for a URL source — distinct from
    /// `document_fonts_garbage_bytes_are_rejected_fail_closed` above,
    /// whose bytes don't match a WOFF/WOFF2 signature at all (so
    /// `decode_web_font` passes them through unchanged and they're instead
    /// rejected later by the font library). Here the signature matches but the
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
    // FontError — Display and Error::source coverage.
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
            "no font families registered from /fonts/wpt (all .ttf/.otf files rejected by the font library — check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)"
        );
        assert_eq!(
            format!(
                "{}",
                FontError::PreferredFontUnavailable {
                    name: "Ahem.ttf".to_string(),
                    dir: dir.clone(),
                }
            ),
            "preferred font 'Ahem.ttf' not registered under /fonts/wpt — missing from dir or rejected by the font library; silent fallback would break cascade determinism (check scripts/wpt/pinned_sha.txt or run scripts/wpt/fetch.sh)"
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
