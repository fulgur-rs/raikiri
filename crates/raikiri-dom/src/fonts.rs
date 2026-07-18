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
use std::path::{Path, PathBuf};

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
/// - [`FontError::Io`] — dir walk 中の io failure
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
        let bytes = std::fs::read(&path).map_err(|source| FontError::Io {
            path: path.clone(),
            source,
        })?;
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
    /// dir walk 中の io failure
    Io {
        /// walk 中に io error が発生した path
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
        // 通常 file: extension check
        let is_font = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| {
                let s = s.to_ascii_lowercase();
                s == "ttf" || s == "otf"
            })
            .unwrap_or(false);
        if is_font {
            out.push(path);
        }
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
}
