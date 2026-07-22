//! WPT bundled font dir を register し、system_fonts: false と generic
//! family alias で cross-machine 決定性 FontContext を構築する。
//!
//! - Fetch は `scripts/wpt/fetch.sh` (dev prerequisite)、本 module は
//!   fetch 済 `target/wpt/fonts/` を Path で受けるだけ
//! - production runtime は `parley::FontContext::new()` を今のまま使う
//! - M1 scope: `.ttf` / `.otf` のみ、WOFF/WOFF2 は M4+ (raikiri-spike-2sb)
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
//! `crates/raikiri/tests/external_consumer.rs`. bd raikiri-spike-1uq
//! (fe1 §8.2 Angle B/G observability follow-up).

use parley::FontContext;
use std::io::Read;
use std::path::{Path, PathBuf};

/// [`build_wpt_font_ctx`] が個別 font file を読み込む際に許容する最大 byte 数。
/// 100 MiB は現実の bundled font (Ahem: ~12 KiB, Noto CJK: ~20 MiB 前後) に対して
/// 十分な余裕を残しつつ、attacker が用意した巨大 regular file による memory
/// exhaustion を弾く閾値。
///
/// **Threat surface coverage** (raikiri-spike-d9y.4, Codex security finding
/// `ffe1f9c7027c8191a8f8812a6456c7d9`):
///
/// - **symlink → /dev/zero**: `collect_recursive` 側の
///   `file_type.is_symlink()` skip (roborev e93 round 4) で既に closed
/// - **FIFO / device / socket** (indefinite block): `collect_recursive` 側の
///   `file_type.is_file()` gate で walk 段階で closed。`Read::take(N)` は memory を
///   bound するが writer 未定の FIFO に対して time は bound しないので、walk 段階で
///   排除するのが 1st line of defense。read 段階の TOCTOU-swap (regular → FIFO /
///   device 差し替え) は下記 leaf-swap 項の後段で defended
/// - **oversized regular file** (memory exhaustion): `metadata.len() >
///   FONT_SIZE_CAP` skip で closed
/// - **mid-read grow (TOCTOU)**: `read_bounded_font_file` の callsite-local
///   `+1-probe` (`take(FONT_SIZE_CAP + 1) + post-read bytes.len > cap` reject)
///   が silent truncation を防ぎ、TOCTOU-grow を `OversizedDuringRead` として
///   surface (fe1 で raikiri-traits helper 経由へ切替、pre-fe1 の
///   `take(FONT_SIZE_CAP)` silent-truncation window は closed。raikiri-spike-61l
///   で helper から離脱し 8yu 同型の callsite-local pipeline に戻したため、
///   `+1-probe` は本 module 内に再度存在する)
/// - **leaf-swap (TOCTOU)**: walker と `read_bounded_font_file` の pre-open
///   `symlink_metadata` の間で regular file が symlink に差し替わる vector は、
///   `safe_open` (unix: `O_NOFOLLOW`) で closed (raikiri-spike-61l、8yu sibling)。
///   ELOOP (POSIX 準拠 Linux / macOS / modern FreeBSD) or 事後 `symlink_metadata`
///   recheck (legacy BSD の EMLINK / EFTYPE) を「leaf-swap symlink 相当」として
///   warn+skip し、Ahem の drop は下流の aggregate `PreferredFontUnavailable`
///   check が catch する。
///
/// - **FIFO / device-swap (TOCTOU) after pre-open metadata**: walker skip と
///   `read_bounded_font_file` の pre-open `symlink_metadata` の間に regular file
///   が FIFO / character device / block device に差し替わる vector は、
///   `safe_open` (unix: `O_NOFOLLOW | O_NONBLOCK`) + post-open
///   [`check_open_handle_regular`] fd-based fstat で closed。`O_NONBLOCK` が
///   writer 未定 FIFO の `open()` block を防ぎ (time-DoS の 1st defense)、
///   post-open `File::metadata().file_type().is_file()` の fd-based check が
///   非 regular kind を race-free に reject する (`NotRegularFilePostOpen`)。
///   pre-open path-based check と違い fd 発行後の stat なので path-swap TOCTOU
///   では bypass 不可能。raikiri-spike-f4j (61l Codex §8.3 finding #2)。
///
/// TODO(raikiri-spike-d9y.3): Wave 0 の RenderLimits と連動させる。
const FONT_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// Open a regular file with the leaf-swap TOCTOU defense stack.
///
/// The unix impl passes `O_NOFOLLOW | O_NONBLOCK` to `File::open`:
///
/// - `O_NOFOLLOW`: a symlink swapped in between the pre-open
///   `symlink_metadata` check and this open call cannot cause the resolver
///   to follow a fresh target. POSIX mandates `ELOOP` for
///   `open(O_NOFOLLOW)` on a symlink (Linux, macOS, and modern FreeBSD
///   comply); legacy BSDs (NetBSD, OpenBSD, FreeBSD <10) may return
///   `EMLINK` or `EFTYPE` instead. Callers that need to distinguish this
///   case from other I/O errors should check the errno match against
///   `libc::ELOOP` AND fall back to a `symlink_metadata` recheck (see
///   [`read_bounded_font_file`] for the portable pattern).
/// - `O_NONBLOCK`: **load-bearing time-DoS defense** paired with the
///   post-open fstat check in [`check_open_handle_regular`]. If a regular
///   file is swapped for a **writer-less FIFO** between the pre-open
///   `symlink_metadata` check and this `open()`, the bare
///   `open(O_RDONLY | O_NOFOLLOW)` call would block indefinitely at the
///   `open()` syscall itself — `O_NOFOLLOW` does not fire (a FIFO is not a
///   symlink), so the post-open fstat is never reached. `O_NONBLOCK` makes
///   FIFO opens return immediately (POSIX: read-side `O_RDONLY | O_NONBLOCK`
///   on a FIFO succeeds even with no writer), letting the post-open fstat
///   inspect the fd and reject non-regular kinds. Regular file semantics
///   are unaffected: POSIX specifies `O_NONBLOCK` has no effect on regular
///   files, and Linux/macOS both honor that. bd raikiri-spike-f4j
///   (61l Codex §8.3 finding #2).
///
/// The non-unix fallback keeps the current default `File::open` semantics.
/// Windows equivalent tracked in raikiri-spike-akk.
///
/// bd raikiri-spike-61l (8yu sibling), raikiri-spike-f4j (O_NONBLOCK).
// Callsite-local defense: sharing this stack with raikiri-vrt via
// raikiri-traits is deferred to raikiri-spike-7xw for walls.md §2 crate-list
// PMO judgment. Do not lift into raikiri-traits::io without that judgment.
#[cfg(unix)]
fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
    // Follow-symlink-at-open is unresolved on non-unix; tracked in
    // raikiri-spike-akk.  Regain parity when the follow-up lands.
    std::fs::File::open(path)
}

/// Reason `read_bounded_font_file` rejected a candidate font path.
///
/// Mirrors the shape of `raikiri_traits::io::RejectReason` so the callsite
/// policy (`Io` → propagate, other reasons → warn+skip) reads the same as
/// pre-61l fe1.  A local enum is used instead of the traits helper's
/// `RejectReason` because the callsite bypasses the traits helper on this
/// path — safe_open with `O_NOFOLLOW` is applied at the callsite until
/// raikiri-spike-7xw lifts safe_open into `raikiri_traits::io` (walls.md §2).
#[derive(Debug)]
enum FontReadReject {
    /// `symlink_metadata().file_type().is_symlink()` returned true, **or**
    /// safe_open returned an errno consistent with `O_NOFOLLOW` refusing to
    /// follow a symlink (POSIX `ELOOP`; a fallback `symlink_metadata` recheck
    /// covers legacy-BSD `EMLINK` / `EFTYPE`).  Either way the leaf was a
    /// symlink at reject time and no read happened through it.
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
    /// per-stage split).  bd raikiri-spike-f4j, closes 61l Codex §8.3
    /// finding #2.
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
    /// `raikiri_vrt::reference::FixtureError::PathEscape` (bd raikiri-spike-d9y.6);
    /// re-consolidation with the traits helper tracked in raikiri-spike-7xw
    /// (walls.md §2).  bd raikiri-spike-zr8, closes 61l Codex §8.3 finding #1.
    PathEscape {
        /// Canonicalized target path that fell outside the root.
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
            FontReadReject::Io(source) => write!(f, "I/O error: {source}"),
        }
    }
}

/// Post-open fd-based fstat: verify the handle `safe_open` returned still
/// resolves to a regular file.  Load-bearing supplement to the pre-open
/// path-based `symlink_metadata + is_file()` gate.
///
/// The pre-open path-based check is race-vulnerable — a regular file can be
/// swapped for a FIFO / character device / block device between the pre-open
/// metadata call and the subsequent `safe_open`.  `O_NOFOLLOW` does not
/// filter file kind (a FIFO is not a symlink), so the swapped-in kind slips
/// through `safe_open`.  Calling `File::metadata()` on the returned fd
/// consults the inode already bound to the descriptor, so no path lookup
/// re-runs and the race window is closed by construction.
///
/// Paired with `O_NONBLOCK` in `safe_open` (unix): without it, opening a
/// writer-less FIFO would block at the `open()` syscall itself and this
/// fstat would never be reached.  Together they close the FIFO-swap
/// time-DoS the walker-only `is_file()` gate cannot cover.
///
/// Portable: the fstat is via `std::fs::File::metadata`, which delegates to
/// the platform's fd-based stat (Linux `fstat`, Windows
/// `GetFileInformationByHandle`).  Called unconditionally after safe_open
/// Ok so non-unix builds get the defense-in-depth too, even where
/// `safe_open`'s `O_NOFOLLOW`/`O_NONBLOCK` custom flags are absent.
///
/// bd raikiri-spike-f4j, closes 61l Codex §8.3 finding #2.
fn check_open_handle_regular(file: &std::fs::File) -> Result<(), FontReadReject> {
    let metadata = file.metadata().map_err(FontReadReject::Io)?;
    if !metadata.file_type().is_file() {
        return Err(FontReadReject::NotRegularFilePostOpen);
    }
    Ok(())
}

/// Read a regular font file with the leaf-swap TOCTOU defense stack:
/// pre-open `symlink_metadata` + `is_file()` + size-cap gates, `safe_open`
/// with `O_NOFOLLOW | O_NONBLOCK` at open time (unix), a post-open
/// fd-based fstat via [`check_open_handle_regular`] that rejects
/// non-regular kinds bound to the descriptor (race-free against
/// pre-open→open path-swap), and a `+1-probe` bounded read.
///
/// Callsite-local variant of `raikiri_traits::io::read_bounded_regular_file`.
/// The traits helper's `File::open` follows symlinks, leaving a leaf-swap
/// TOCTOU window between its `symlink_metadata` check and its open call
/// (bd raikiri-spike-61l).  This function closes that window by holding the
/// file descriptor `safe_open` returns and reading from it directly, so the
/// pre-open gates and the actual read are one uninterrupted protected
/// sequence.  Re-consolidation with the traits helper is tracked in
/// raikiri-spike-7xw (walls.md §2 escalation).
///
/// Pre-open gates are load-bearing beyond what `O_NOFOLLOW` covers:
/// - `!is_file()` rejects direct FIFO / device placements at pre-open —
///   defense-in-depth 1st layer.  `O_NOFOLLOW` only refuses to follow a
///   *symlink* leaf; it does not filter device type.  The TOCTOU-swap
///   subclass (regular file → FIFO / device between this pre-open check
///   and `safe_open`) is caught by the post-open fd-based fstat in
///   [`check_open_handle_regular`], paired with `O_NONBLOCK` in
///   `safe_open` (unix) so a writer-less FIFO cannot block `open()`
///   before the fstat is reached (raikiri-spike-f4j).
/// - `metadata.len() > cap` is the up-front oversized reject; the
///   `+1-probe` bounded read below catches the TOCTOU-grow subclass where
///   the file expanded between the metadata check and the read.
///
/// Non-unix `safe_open` retains the follow-symlink `File::open` fallback
/// (Windows equivalent tracked in raikiri-spike-akk); the pre-open
/// `symlink_metadata` check still rejects the common shape there.
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
/// `raikiri_vrt::reference::read_bounded_fixture_file`
/// (bd raikiri-spike-d9y.6); re-consolidation with the traits helper
/// tracked in raikiri-spike-7xw (walls.md §2).
/// bd raikiri-spike-zr8, closes 61l Codex §8.3 finding #1.
fn read_bounded_font_file(
    path: &Path,
    canonical_root: &Path,
    size_cap: u64,
) -> Result<Vec<u8>, FontReadReject> {
    let metadata = std::fs::symlink_metadata(path).map_err(FontReadReject::Io)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(FontReadReject::Symlink);
    }
    if !file_type.is_file() {
        return Err(FontReadReject::NotRegularFile);
    }
    if metadata.len() > size_cap {
        return Err(FontReadReject::OversizedPreOpen {
            size: metadata.len(),
            cap: size_cap,
        });
    }
    // Containment check via canonicalization.  Given the leaf `is_symlink()`
    // gate above, an unresolvable escape via a *leaf* symlink is already
    // rejected; this branch covers *intermediate*-directory-symlink escapes
    // (e.g. `fonts_dir/subdir/` swapped into a symlink to `/tmp/evil/`
    // between walk and read, with `subdir/Ahem.ttf` still a regular file
    // leaf so `is_symlink()` on the leaf does not fire).  Mirrors
    // `raikiri_vrt::reference::read_bounded_fixture_file` (bd raikiri-spike-d9y.6);
    // walls.md §2 re-consolidation deferred to raikiri-spike-7xw.
    // bd raikiri-spike-zr8, closes 61l Codex §8.3 finding #1.
    let canonical = std::fs::canonicalize(path).map_err(FontReadReject::Io)?;
    if !canonical.starts_with(canonical_root) {
        return Err(FontReadReject::PathEscape {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    // safe_open adds O_NOFOLLOW on unix so a leaf-symlink swapped in between
    // the above symlink_metadata check and this open call cannot cause a
    // fresh symlink target to be followed.  bd raikiri-spike-61l (8yu sibling).
    //
    // cov:ignore: the safe_open `Err` arm here needs a leaf-swap race (or a
    // transient stat failure) to fire on a path that already passed the
    // pre-open `symlink_metadata` gate above.  Neither is deterministically
    // unit-testable; the `O_NOFOLLOW`-rejects-a-symlink behavior is instead
    // pinned by the standalone `safe_open_rejects_symlink_at_open_time` test.
    let mut file = match safe_open(path) {
        Ok(f) => f,
        Err(e) => {
            #[cfg(unix)]
            {
                // POSIX mandates ELOOP for O_NOFOLLOW on symlink (Linux, macOS,
                // and modern FreeBSD comply). Legacy BSDs may return EMLINK or
                // EFTYPE instead; the post-Err symlink_metadata recheck catches
                // those cases portably at the cost of one extra stat syscall on
                // the reject path. Not race-perfect (an attacker could swap the
                // symlink back to a regular file between safe_open and this
                // recheck), but covers the common attack shape while remaining
                // simple. Full inode-verify would require fstat-after-open on
                // the handle safe_open never returned.
                let looks_like_symlink_swap = e.raw_os_error() == Some(libc::ELOOP)
                    || std::fs::symlink_metadata(path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                if looks_like_symlink_swap {
                    return Err(FontReadReject::Symlink);
                }
            }
            return Err(FontReadReject::Io(e));
        }
    };
    // Post-open fd-based fstat: reject if the descriptor `safe_open` bound
    // does not resolve to a regular file.  Closes the FIFO / device swap
    // TOCTOU window that the pre-open path-based `symlink_metadata` +
    // `is_file()` gate cannot cover (an attacker can swap regular file ->
    // FIFO between pre-open metadata and open; `O_NOFOLLOW` does not
    // filter file kind).  Paired with `O_NONBLOCK` in `safe_open` on
    // unix so the swapped-in writer-less FIFO cannot block `open()`
    // before this check runs.  bd raikiri-spike-f4j, closes 61l Codex
    // §8.3 finding #2.
    //
    // cov:ignore: the swap window between pre-open `symlink_metadata` and
    // `safe_open` is not deterministically unit-testable.  The
    // `check_open_handle_regular` helper is exercised in isolation
    // (regular file passes, FIFO / char device reject via `safe_open`
    // fed directly) and the `read_reject_to_warn` mapping is pinned
    // deterministically via `read_reject_to_warn_maps_all_non_io_variants`.
    check_open_handle_regular(&file)?;
    // Preallocate against the known-good `metadata.len()` upper bound (mirrors
    // raikiri_traits::io helper's happy-path allocation to avoid log2(N)
    // reallocations on large fonts).  `saturating_add(1)` guards against a
    // future `size_cap == u64::MAX` caller — same regression pattern the
    // traits helper codified.
    let mut bytes = Vec::with_capacity(std::cmp::min(metadata.len(), size_cap) as usize);
    let read_limit = size_cap.saturating_add(1);
    file.by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(FontReadReject::Io)?;
    if bytes.len() as u64 > size_cap {
        // cov:ignore: TOCTOU-grow race requires a concurrent writer to grow
        // the file between the metadata check above and this post-read
        // gate; the `+1-probe` shape itself is exercised by the traits
        // helper's `t19` deterministic test on the sibling implementation.
        // Escalating to a raikiri-spike bd would duplicate that.
        return Err(FontReadReject::OversizedDuringRead {
            size: bytes.len() as u64,
            cap: size_cap,
        });
    }
    Ok(bytes)
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
/// `crates/raikiri-traits/src/error.rs` (bd raikiri-spike-37n).  Variants are
/// **split by callsite** (walker vs read) rather than sharing a single
/// [`FontReadReject`]-shaped taxonomy because the walker-vs-read distinction
/// is the exact TOCTOU signal fe1 §8.2 wants: a walker `Symlink` is a
/// mundane cycle-safe skip, whereas a read-time `Symlink` means the tree
/// changed between walk and read (leaf-swap TOCTOU).  Collapsing them would
/// destroy that signal.  bd raikiri-spike-1uq.
///
/// # Not surfaced
///
/// `FontReadReject::Io(_)` never becomes a `FontWarn` variant.  It is
/// hard-propagated as [`FontError::Io`] (preserves the roborev e93 round 3
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
    /// time-DoS defense (raikiri-spike-d9y.4).
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
    /// **leaf-swap TOCTOU race** between walk and read (raikiri-spike-61l).
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
    /// bd raikiri-spike-f4j, closes 61l Codex §8.3 finding #2.
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
    /// the bounded read.  The load-bearing observability signal fe1 §8.2
    /// Angle B/G was designed for.
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
    /// `raikiri_vrt::reference::FixtureError::PathEscape`.  bd
    /// raikiri-spike-zr8, closes 61l Codex §8.3 finding #1.
    ReadRejectedPathEscape {
        /// Walker-observed candidate path (pre-canonicalization).
        path: &'a Path,
        /// Canonicalized target path that fell outside the root.
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
    /// operators see on stderr (fe1 §8.2 "behavior-change" caution).  The
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
fn emit_warn(observer: &mut FontWarnObserver<'_>, event: FontWarn<'_>) {
    if let Some(cb) = observer.as_mut() {
        cb(&event);
    } else {
        eprintln!("[raikiri-dom::fonts] warn: {event}");
    }
}

/// Map a non-`Io` [`FontReadReject`] to its [`FontWarn::ReadRejected*`]
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
/// structured observer callback for TOCTOU-swap/grow anomalies (fe1 §8.2
/// Angle B/G) should call `_with_observer` directly.  Signature preserved
/// for the `crates/raikiri/tests/external_consumer.rs` pin (bd raikiri-spike-e93).
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
/// legacy `eprintln!` shape so CLI use is unaffected.  bd raikiri-spike-1uq
/// (fe1 §8.2 Angle B/G observability follow-up).
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
    // panic.  bd raikiri-spike-zr8 (closes 61l Codex §8.3 finding #1).
    let canonical_root = std::fs::canonicalize(fonts_dir).map_err(|source|
        // cov:ignore: the Err arm here needs a TOCTOU race (fonts_dir
        // unlinked between the `.exists()` check above and canonicalize)
        // to fire, which is not deterministically unit-testable.  The
        // sibling races (safe_open Err arm at ~L254, TOCTOU-grow at ~L296,
        // read-time reject arm at ~L699) use the same cov:ignore shape.
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
    // (roborev Medium finding e93 round 3): 従来の eprintln! warn skip では、
    // Ahem.ttf (PREFERRED_FIRST[0]) が read failed 時に silently 次候補
    // (CSSTest 等) が register され、cascade "serif" が想定外の font に解決
    // されてしまう。walker が返した path は既に存在確認済 (read_dir で
    // 列挙された) なので、read 段階で失敗するのは permission 変更や symlink
    // 損傷など明確な異常。ここで停止する方が「default で silent regression」
    // より安全。個別 file の fontique reject (register.is_empty) は
    // aggregate check (`family_ids.is_empty` → NoFontsRegistered) が catch する
    // ので warn+skip のまま維持。
    let mut family_ids = Vec::new();
    // PREFERRED_FIRST invariant tracking (roborev Medium finding e93 round 5):
    // path が PREFERRED_FIRST member かつ register 成功したものを basename
    // 単位で記録。loop 後にこの set と PREFERRED_FIRST を照合し、欠落 or
    // register-empty があれば PreferredFontUnavailable。silent fallback を防ぐ。
    let mut registered_preferred_basenames: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for path in paths {
        // Bounded read via callsite-local `read_bounded_font_file`
        // (raikiri-spike-61l).  fe1 initially routed through the shared
        // `raikiri_traits::io::read_bounded_regular_file`, but the traits
        // helper's `File::open` follows symlinks and leaves a leaf-swap TOCTOU
        // window between its `symlink_metadata` check and the open.  61l
        // closes that window here by holding the descriptor `safe_open`
        // returns (unix `O_NOFOLLOW`) and reading from it directly.
        // Re-consolidating with the traits helper is deferred to
        // raikiri-spike-7xw (walls.md §2).
        //
        // Callsite policy (preserved from fe1):
        // - `Io(_)` -> hard-error propagate.  Preserves the "Ahem.ttf
        //   silent-fallback prevention" guarantee (roborev e93 round 3): an
        //   unexpected Io error at read time aborts the build directly, so
        //   the registry cannot silently drop the preferred font without a
        //   caller-visible error.  `PreferredFontUnavailable` is not the
        //   catch for this branch — it fires only for the warn+skip arm
        //   below (see next bullet).
        // - `Symlink | NotRegularFile | NotRegularFilePostOpen |
        //   OversizedPreOpen | OversizedDuringRead | PathEscape` -> warn+skip.
        //   `collect_recursive` already pre-filtered symlink/non-regular/
        //   oversized, so surfacing here means the tree changed between walk
        //   and read (TOCTOU-swap / TOCTOU-grow / intermediate-symlink swap).
        //   Non-preferred fonts silently drop from the registry; if Ahem.ttf
        //   is affected, `PreferredFontUnavailable` fires downstream on the
        //   aggregate `registered_preferred_basenames` check.
        //   `OversizedDuringRead` closes the silent-truncation window the
        //   prior `take(FONT_SIZE_CAP)` had (fe1 fix; the `+1-probe` surfaces
        //   TOCTOU-grow instead of returning a truncated buffer).  `Symlink`
        //   covers the leaf-swap TOCTOU class: safe_open's `O_NOFOLLOW`
        //   ELOOP or the post-error `symlink_metadata` recheck surface a
        //   mid-walk swap-in (raikiri-spike-61l).  `PathEscape` covers the
        //   intermediate-symlink-swap class (walker recorded
        //   `subdir/font.ttf`, attacker swapped `subdir/` into a symlink to
        //   `/tmp/evil/` between walk and read; the leaf still stats as a
        //   regular file so `is_symlink()` does not fire but canonicalize
        //   resolves outside `canonical_root`).  bd raikiri-spike-zr8,
        //   closes 61l Codex §8.3 finding #1.  `NotRegularFilePostOpen`
        //   covers the FIFO/character device / block device swap TOCTOU
        //   class (walker + pre-open metadata saw a regular file, attacker
        //   swapped it for a FIFO / device between pre-open metadata and
        //   `safe_open`).  `O_NONBLOCK` in `safe_open` prevents the
        //   writer-less FIFO from blocking `open()`, and the fd-based
        //   `File::metadata()` fstat inspects the inode already bound to the
        //   descriptor (race-free by construction).  bd raikiri-spike-f4j,
        //   closes 61l Codex §8.3 finding #2.
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

    // PREFERRED_FIRST invariant enforce (roborev Medium finding e93 round 5):
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
    // PREFERRED_FIRST=["Ahem.ttf"] の invariant check が先に fire する)
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
    /// fontique が register を拒否した (roborev finding e93 round 5)。
    /// silent fallback で cascade 決定性を破壊しないよう dedicated Err。
    PreferredFontUnavailable {
        /// 期待されたが register 成功しなかった font の basename
        name: String,
        /// scan 対象の fonts dir
        dir: PathBuf,
    },
    /// dir walk 中の io failure、または font read (callsite-local
    /// `read_bounded_font_file` 経由) の `FontReadReject::Io(_)` propagate。
    /// 他 reject reason (Symlink / NotRegularFile / OversizedPreOpen /
    /// OversizedDuringRead) は warn+skip される (詳細は
    /// [`build_wpt_font_ctx`] の callsite comment 参照)。
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
    // ordered_preferred からも rest からも silently drop されてしまう
    // (M1 finding)。`Vec::retain` で target にマッチする要素を全部
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
    // 防ぐ (M1 finding の根本対策)。
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
    // 各 DirEntry を preserve して `file_type()` で kind を照会する
    // (roborev Medium finding e93 round 4)。過去の `Path::is_dir()` 経由は:
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
        // raikiri-spike-d9y.4, Codex finding `ffe1f9c7027c8191a8f8812a6456c7d9`。
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
        // exhaustion を弾く (raikiri-spike-d9y.4)。境界値 (== FONT_SIZE_CAP) は
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
        // 効いていることを検証する (M2 finding: これが無いと Ahem が
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
        // Ahem (PREFERRED_FIRST[0]) が alphabetically 先頭の
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
        // M1 regression pin: 同一 basename (Ahem.ttf) が top-level と
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
        let bogus = Path::new("/definitely/does/not/exist/raikiri-spike-e93");
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
        // raikiri-spike-d9y.4 regression pin (Codex finding
        // `ffe1f9c7027c8191a8f8812a6456c7d9`): 攻撃者が制御下 fonts dir に
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
        // raikiri-spike-d9y.4 regression pin: FONT_SIZE_CAP + 1 byte の
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
        // raikiri-spike-d9y.4: filter が silently over-reject していないことを
        // pin する (境界値 == FONT_SIZE_CAP は通す — build_wpt_font_ctx 側の
        // `take(FONT_SIZE_CAP)` bounded read は境界を全 consume する)。
        // boundary.ttf: `File::set_len(FONT_SIZE_CAP)` で sparse file を作り、
        // 境界値ちょうど (`metadata.len() == FONT_SIZE_CAP`) が accept 側に
        // 落ちる (`>` cap で skip、`<= cap` で accept) ことを直接 pin する
        // (codex final review 軽微 finding fix — tiny file では境界を実際に
        // 触れず silent over-reject を捕捉できない)。
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
        // roborev Medium finding e93 round 4 regression pin: `Path::is_dir()`
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
        // roborev Medium finding e93 round 5 regression pin: PREFERRED_FIRST
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
        // roborev Medium finding e93 round 5 regression pin: PREFERRED_FIRST
        // font (Ahem.ttf) が disk に存在するが fontique に reject された場合、
        // 他の valid font が silent fallback として cascade "serif" に解決
        // されてはならない (Round 3 の read failure fix と同じ精神で、
        // register failure も dedicated Err に昇格)。
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
        // に解決されることを assert する完全な検証は Task 8 の end-to-end VRT
        // が担保する。ここでは build_wpt_font_ctx が real WPT font dir
        // (Ahem.ttf 含む) に対して panic せず Ok を返すことのみを smoke
        // check する (M1 scope、controller ambiguity resolution 済)。
        let _ = ctx;
    }

    /// safe_open must reject a symlink at open time on unix. POSIX mandates
    /// ELOOP (Linux, macOS, modern FreeBSD comply); legacy BSDs (NetBSD,
    /// OpenBSD, FreeBSD <10) return EMLINK or EFTYPE. The test accepts any
    /// Err on unix, because a passing implementation must not follow the
    /// symlink regardless of the exact errno. The Ok arm is the regression
    /// pin — an implementation that drops custom_flags(O_NOFOLLOW) would
    /// silently follow the link and return Ok(file), failing this test.
    /// bd raikiri-spike-61l (8yu sibling).
    #[cfg(unix)]
    #[test]
    fn safe_open_rejects_symlink_at_open_time() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.ttf");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target contents")
            .unwrap();
        let link = tmp.path().join("link.ttf");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match safe_open(&link) {
            Err(_) => {
                // Sanity-check that the path is still a symlink at
                // observation time — proves the Err is due to O_NOFOLLOW
                // (portable across ELOOP / EMLINK / EFTYPE) rather than
                // an unrelated I/O error like permission or NotFound.
                let post = std::fs::symlink_metadata(&link)
                    .expect("symlink still present after safe_open Err");
                assert!(
                    post.file_type().is_symlink(),
                    "link at test observation time was not a symlink"
                );
            }
            Ok(_) => panic!("safe_open followed the symlink (O_NOFOLLOW not applied)"),
        }
    }

    /// End-to-end pin: `read_bounded_font_file` rejects a symlink at the
    /// pre-open `symlink_metadata` check.  This test does NOT exercise the
    /// O_NOFOLLOW path (safe_open never runs because the pre-open check
    /// short-circuits) — that unit is covered by
    /// `safe_open_rejects_symlink_at_open_time`.  Kept to pin the full-path
    /// behavior against future refactors that might reorder the checks.
    /// bd raikiri-spike-61l (8yu sibling).
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
    /// bd raikiri-spike-61l (8yu sibling).
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
    /// bd raikiri-spike-61l.
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
    /// bd raikiri-spike-61l.
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
    /// layouts with symlinked prefixes. bd raikiri-spike-zr8.
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
    /// producing `FontReadReject::PathEscape`. Closes 61l Codex §8.3
    /// finding #1. bd raikiri-spike-zr8.
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
    /// error message would move. bd raikiri-spike-zr8.
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

    // ------------------------------------------------------------------
    // Post-open fd-based fstat tests (bd raikiri-spike-f4j — 61l Codex
    // §8.3 finding #2 FIFO/device swap TOCTOU defense)
    // ------------------------------------------------------------------
    //
    // The FIFO/device swap window between the pre-open `symlink_metadata`
    // check and `safe_open` is not deterministically reachable through
    // `read_bounded_font_file` (walker + pre-open would need to see a
    // regular file at path-lookup time, then swap kind before the fd is
    // bound — that requires a threaded race).  The defense is instead
    // exercised at the primitive level: `safe_open` + the
    // `check_open_handle_regular` helper called directly on FIFO / char
    // device paths, plus a happy-path regression pin that the O_NONBLOCK
    // addition did not break regular-file reads.

    /// `check_open_handle_regular` accepts a regular file opened via
    /// `safe_open` — the primary regression pin that adding `O_NONBLOCK`
    /// to `safe_open`'s custom_flags does not break the happy path.  POSIX
    /// specifies `O_NONBLOCK` has no effect on regular files (Linux honors
    /// this), so `safe_open` returns immediately and `File::metadata()`
    /// resolves via fd-based fstat to `is_file() == true`.
    /// bd raikiri-spike-f4j.
    #[test]
    fn check_open_handle_regular_accepts_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("regular.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"regular content")
            .unwrap();

        let file = safe_open(&path).expect("safe_open should succeed on regular file");
        check_open_handle_regular(&file).expect("regular file must pass post-open fstat");
    }

    /// `safe_open` + `check_open_handle_regular` rejects a FIFO.  Two
    /// asserts pin the composite defense:
    ///
    /// 1. `safe_open` returns `Ok` (without hanging) — proves the
    ///    `O_NONBLOCK` addition prevents `open()` from blocking on a
    ///    writer-less FIFO.  Without `O_NONBLOCK`, `open(O_RDONLY)` on a
    ///    writer-less FIFO blocks indefinitely at the syscall itself and
    ///    the test would deadlock (never reach the fstat).
    /// 2. `check_open_handle_regular` returns
    ///    `Err(FontReadReject::NotRegularFilePostOpen)` — proves the
    ///    fd-based `File::metadata()` fstat correctly identifies the FIFO
    ///    kind and would reject before any read from the pipe.
    ///
    /// The regular-file `NotRegularFile` pre-open reject is bypassed here
    /// because we call `safe_open` directly rather than through
    /// `read_bounded_font_file` — that isolation is intentional so the
    /// test exercises the post-open detection path, which in the composed
    /// pipeline only fires on a real TOCTOU race (cov:ignore in the loop).
    /// bd raikiri-spike-f4j, closes 61l Codex §8.3 finding #2.
    #[cfg(unix)]
    #[test]
    fn check_open_handle_regular_rejects_fifo() {
        let tmp = tempfile::tempdir().unwrap();
        let fifo = tmp.path().join("evil.ttf");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        // safe_open must return Ok(fd) without blocking — O_NONBLOCK
        // regression guard.  A test process reaching this assertion
        // proves `open()` did not hang on the writer-less FIFO.
        let file = safe_open(&fifo).expect(
            "safe_open on writer-less FIFO should return Ok immediately via O_NONBLOCK, \
             not block or Err — if this fails the O_NONBLOCK flag was dropped or the \
             open path regressed",
        );

        match check_open_handle_regular(&file) {
            Err(FontReadReject::NotRegularFilePostOpen) => {}
            other => {
                panic!("expected NotRegularFilePostOpen for FIFO post-open fstat, got {other:?}")
            }
        }
    }

    /// `safe_open` + `check_open_handle_regular` rejects `/dev/null`
    /// (character device).  Complements the FIFO test by pinning that the
    /// fd-based fstat also rejects device kinds — the same swap-window
    /// TOCTOU attack could substitute `/dev/null` (or any device) rather
    /// than a FIFO, and the defense must catch both.
    ///
    /// Uses `/dev/null` directly rather than a tempdir path because
    /// `safe_open` performs no containment check (containment is
    /// `read_bounded_font_file`'s responsibility).  bd raikiri-spike-f4j.
    #[cfg(unix)]
    #[test]
    fn check_open_handle_regular_rejects_char_device() {
        let file = safe_open(Path::new("/dev/null")).expect(
            "safe_open on /dev/null should succeed (regular open semantics on char device)",
        );
        match check_open_handle_regular(&file) {
            Err(FontReadReject::NotRegularFilePostOpen) => {}
            other => panic!(
                "expected NotRegularFilePostOpen for /dev/null post-open fstat, got {other:?}"
            ),
        }
    }

    // ------------------------------------------------------------------
    // Observer tests (bd raikiri-spike-1uq — fe1 §8.2 Angle B/G follow-up)
    // ------------------------------------------------------------------
    //
    // Structural coverage for `build_wpt_font_ctx_with_observer`:
    // - Deterministic walker sites (symlink / non-regular / oversized) fire.
    // - Deterministic fontique register-empty site fires.
    // - Default None-observer path still writes to eprintln! and does not
    //   panic.
    // - The `read_reject_to_warn` mapping covers all non-Io variants
    //   (including `NotRegularFilePostOpen` from raikiri-spike-f4j)
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
                FontWarn::RegisterEmpty { path } => Self::RegisterEmpty(path.into()),
            }
        }
    }

    /// Observer fires `WalkerSkippedSymlink` when a symlink entry sits
    /// alongside real fonts.  Regression pin: the walker's cycle-safe skip
    /// must route through the shared observer, not just the legacy
    /// eprintln!.  bd raikiri-spike-1uq.
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
    /// bd raikiri-spike-1uq.
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
    /// [`FONT_SIZE_CAP`].  Sparse `set_len(FONT_SIZE_CAP + 1)` avoids
    /// consuming 100 MiB of test disk; `metadata.len()` returns the logical
    /// size regardless.  bd raikiri-spike-1uq.
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
    /// event.  bd raikiri-spike-1uq.
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
    /// bd raikiri-spike-1uq.
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

    /// `read_reject_to_warn` maps every non-Io [`FontReadReject`] variant to
    /// its [`FontWarn::ReadRejected*`] counterpart.  The read-time observer
    /// arm in `build_wpt_font_ctx_with_observer` is `cov:ignore` (the
    /// walker pre-filters symlink/non-regular/oversized, so the arm only
    /// fires on a real TOCTOU race), which would otherwise leave the
    /// TOCTOU-observability deliverable untested.  This unit test closes
    /// that coverage gap deterministically.  bd raikiri-spike-1uq.
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
        // bd raikiri-spike-f4j.
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
        // bd raikiri-spike-zr8.
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
    }

    /// `FontWarn`'s `Display` output must reproduce the pre-observer
    /// `eprintln!` message bodies verbatim.  fe1 §8.2 flagged fonts.rs
    /// warn-message wording as a behavior-change surface; this test pins
    /// that the None-observer fallback is byte-identical (modulo the
    /// callsite prefix `[raikiri-dom::fonts] warn: `).  bd raikiri-spike-1uq.
    #[test]
    fn font_warn_display_reproduces_legacy_message_bodies() {
        let p = Path::new("/tmp/fake.ttf");

        // Walker sites — bodies from the pre-1uq eprintln! calls in
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

        // Read-time sites — bodies mirror the pre-1uq eprintln! wording
        // "skipping <path> (<FontReadReject Display>)".
        assert_eq!(
            format!("{}", FontWarn::ReadRejectedSymlink { path: p }),
            "skipping /tmp/fake.ttf (path is a symlink)"
        );
        assert_eq!(
            format!("{}", FontWarn::ReadRejectedNotRegularFile { path: p }),
            "skipping /tmp/fake.ttf (path is not a regular file)"
        );
        // NotRegularFilePostOpen: new variant (bd raikiri-spike-f4j).
        // Message body pins the TOCTOU-swap wording so operators can
        // grep for "TOCTOU-swap" and distinguish this from a mundane
        // pre-open non-regular reject.
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

        // Register-empty site — body from the pre-1uq eprintln! after the
        // fontique register_fonts empty branch.
        assert_eq!(
            format!("{}", FontWarn::RegisterEmpty { path: p }),
            "skipping /tmp/fake.ttf: no family registered"
        );
    }
}
