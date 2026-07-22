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
///   `file_type.is_file()` gate で closed。`Read::take(N)` は memory を bound
///   するが writer 未定の FIFO に対して time は bound しないので、walk 段階で
///   排除するのが load-bearing defense
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
/// **Out of scope (planner-blessed 3-layer)**: walk 直後 regular file が FIFO に
/// 差し替わる TOCTOU-swap は memory は bound されるが time は bound されない
/// (`take(N)` は writer が close するまで block)。O_NONBLOCK + `fstat` 経由の
/// defense が必要になるまで defer。
///
/// TODO(raikiri-spike-d9y.3): Wave 0 の RenderLimits と連動させる。
const FONT_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// Open a regular file with the leaf-swap TOCTOU defense stack.
///
/// The unix impl passes `O_NOFOLLOW` to `File::open` so a symlink swapped
/// in between the pre-open `symlink_metadata` check and this open call
/// cannot cause the resolver to follow a fresh target. POSIX mandates
/// `ELOOP` for `open(O_NOFOLLOW)` on a symlink (Linux, macOS, and modern
/// FreeBSD comply); legacy BSDs (NetBSD, OpenBSD, FreeBSD <10) may return
/// `EMLINK` or `EFTYPE` instead. Callers that need to distinguish this
/// case from other I/O errors should check the errno match against
/// `libc::ELOOP` AND fall back to a `symlink_metadata` recheck (see
/// [`read_bounded_font_file`] for the portable pattern).
///
/// The non-unix fallback keeps the current default `File::open` semantics.
/// Windows equivalent tracked in raikiri-spike-akk.
///
/// bd raikiri-spike-61l (8yu sibling).
// Callsite-local defense: sharing this stack with raikiri-vrt via
// raikiri-traits is deferred to raikiri-spike-7xw for walls.md §2 crate-list
// PMO judgment. Do not lift into raikiri-traits::io without that judgment.
#[cfg(unix)]
fn safe_open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
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
    /// Pre-open `metadata.len() > cap` reject.  The file was never opened.
    /// `cap` carries the configured cap (rather than deferring to
    /// `FONT_SIZE_CAP`) so tests that pass a small cap get honest messages.
    OversizedPreOpen { size: u64, cap: u64 },
    /// The file grew past `cap` between the metadata check and the bounded
    /// read (TOCTOU-grow race), caught by the `+1-probe` pattern:
    /// `take(cap + 1)` then post-read `bytes.len() > cap`.
    OversizedDuringRead { size: u64, cap: u64 },
    /// I/O error from `symlink_metadata`, `safe_open`, or `read_to_end` that
    /// is not consistent with a symlink-swap.  Callsite propagates as
    /// [`FontError::Io`].
    Io(std::io::Error),
}

impl std::fmt::Display for FontReadReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontReadReject::Symlink => write!(f, "path is a symlink"),
            FontReadReject::NotRegularFile => write!(f, "path is not a regular file"),
            FontReadReject::OversizedPreOpen { size, cap } => {
                write!(f, "file size {size} bytes exceeds cap {cap} bytes")
            }
            FontReadReject::OversizedDuringRead { size, cap } => write!(
                f,
                "file grew past cap during read: {size} bytes read, cap {cap} bytes (TOCTOU-grow)"
            ),
            FontReadReject::Io(source) => write!(f, "I/O error: {source}"),
        }
    }
}

/// Read a regular font file with the leaf-swap TOCTOU defense stack:
/// pre-open `symlink_metadata` + `is_file()` + size-cap gates, `safe_open`
/// with `O_NOFOLLOW` at open time (unix), and a `+1-probe` bounded read.
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
/// - `!is_file()` rejects direct FIFO / device placements — `open(O_RDONLY)`
///   on a writer-less FIFO blocks *at open*, before any read.  `O_NOFOLLOW`
///   only refuses to follow a *symlink* leaf; it does not filter device
///   type.
/// - `metadata.len() > cap` is the up-front oversized reject; the
///   `+1-probe` bounded read below catches the TOCTOU-grow subclass where
///   the file expanded between the metadata check and the read.
///
/// Non-unix `safe_open` retains the follow-symlink `File::open` fallback
/// (Windows equivalent tracked in raikiri-spike-akk); the pre-open
/// `symlink_metadata` check still rejects the common shape there.
fn read_bounded_font_file(path: &Path, size_cap: u64) -> Result<Vec<u8>, FontReadReject> {
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

/// WPT bundled fonts dir から FontContext を構築する。system font
/// resolver は完全 disable、generic family (`serif`/`sans-serif`/...)
/// は register 済 family の先頭 (Ahem) に解決される。
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
///   (`FontReadReject::Io(_)` -> `FontError::Io`、他 reject reason は warn+skip)
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError> {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
    use std::sync::Arc;

    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    let paths = walk_fonts(fonts_dir)?;
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
        // - `Symlink | NotRegularFile | OversizedPreOpen | OversizedDuringRead`
        //   -> warn+skip.  `collect_recursive` already pre-filtered these, so
        //   surfacing here means the tree changed between walk and read
        //   (TOCTOU-swap or TOCTOU-grow).  Non-preferred fonts silently drop
        //   from the registry; if Ahem.ttf is affected,
        //   `PreferredFontUnavailable` fires downstream on the aggregate
        //   `registered_preferred_basenames` check.  `OversizedDuringRead`
        //   closes the silent-truncation window the prior `take(FONT_SIZE_CAP)`
        //   had (fe1 fix; the `+1-probe` surfaces TOCTOU-grow instead of
        //   returning a truncated buffer).  `Symlink` now also covers the
        //   leaf-swap TOCTOU class: safe_open's `O_NOFOLLOW` ELOOP or the
        //   post-error `symlink_metadata` recheck surface a mid-walk swap-in
        //   (raikiri-spike-61l).
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
        let bytes = match read_bounded_font_file(&path, FONT_SIZE_CAP) {
            Ok(bytes) => bytes,
            Err(FontReadReject::Io(source)) => {
                return Err(FontError::Io {
                    path: path.clone(),
                    source,
                });
            }
            Err(reason) => {
                eprintln!(
                    "[raikiri-dom::fonts] warn: skipping {} ({reason})",
                    path.display()
                );
                continue;
            }
        };
        let blob = Blob::new(Arc::new(bytes) as _);
        let registered = ctx.collection.register_fonts(blob, None);
        if registered.is_empty() {
            eprintln!(
                "[raikiri-dom::fonts] warn: skipping {}: no family registered",
                path.display()
            );
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
fn walk_fonts(dir: &Path) -> Result<Vec<PathBuf>, FontError> {
    let mut collected: Vec<PathBuf> = Vec::new();
    collect_recursive(dir, &mut collected)?;
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

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), FontError> {
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
            eprintln!(
                "[raikiri-dom::fonts] warn: skipping symlink entry {} (cycle-safe policy)",
                path.display()
            );
            continue;
        }
        if file_type.is_dir() {
            collect_recursive(&path, out)?;
            continue;
        }
        // regular file 以外 (FIFO / device / socket / BlockDevice / CharDevice) は
        // skip: FIFO は `std::fs::read` 経由で writer 未定なら無限 block、device は
        // /dev/zero symlink 経路が閉じられた後の直接配置 attack vector。
        // `Read::take(N)` は memory bound しか担保しないので、時間軸の DoS
        // (blocking read) は walk 段階で file_type filter するのが load-bearing。
        // raikiri-spike-d9y.4, Codex finding `ffe1f9c7027c8191a8f8812a6456c7d9`。
        if !file_type.is_file() {
            eprintln!(
                "[raikiri-dom::fonts] warn: skipping non-regular entry {} (file_type={:?})",
                path.display(),
                file_type
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
            eprintln!(
                "[raikiri-dom::fonts] warn: skipping oversized font {} ({} bytes > cap {})",
                path.display(),
                metadata.len(),
                FONT_SIZE_CAP
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
        let paths = walk_fonts(tmp.path()).unwrap();
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
        let paths = walk_fonts(tmp.path()).unwrap();
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
        let paths = walk_fonts(tmp.path()).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn walker_sort_order_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        // Non-alphabetical create order to exercise sort
        for name in ["z.ttf", "a.ttf", "m.ttf"] {
            write_fake_ttf(tmp.path(), name);
        }
        let first = walk_fonts(tmp.path()).unwrap();
        let second = walk_fonts(tmp.path()).unwrap();
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
        let paths = walk_fonts(tmp.path()).unwrap();
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

        let paths = walk_fonts(tmp.path()).unwrap();
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

        let paths = walk_fonts(tmp.path()).expect("walker Ok with FIFO present");
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

        let paths = walk_fonts(tmp.path()).expect("walker Ok with oversized file");
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
        let paths = walk_fonts(tmp.path()).expect("walker Ok on boundary-size font");
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
        let paths = walk_fonts(tmp.path()).expect("walker Ok even with symlink cycle");
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
        let target = tmp.path().join("target.ttf");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target")
            .unwrap();
        let link = tmp.path().join("link.ttf");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match read_bounded_font_file(&link, FONT_SIZE_CAP) {
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
        let path = tmp.path().join("regular.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"regular content")
            .unwrap();

        let bytes =
            read_bounded_font_file(&path, FONT_SIZE_CAP).expect("regular file should be accepted");
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
        match read_bounded_font_file(tmp.path(), FONT_SIZE_CAP) {
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
        let path = tmp.path().join("too-big.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[0u8; 8])
            .unwrap();

        match read_bounded_font_file(&path, cap) {
            Err(FontReadReject::OversizedPreOpen { size, cap: c }) => {
                assert_eq!(size, 8);
                assert_eq!(c, cap);
            }
            other => panic!("expected OversizedPreOpen, got {other:?}"),
        }
    }
}
