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

    // Register 順 = fallback 順。walker が PREFERRED_FIRST を先頭に置く
    let mut family_ids = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(source) => {
                eprintln!(
                    "[raikiri-dom::fonts] warn: skipping {}: read failed: {}",
                    path.display(),
                    source
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
        family_ids.extend(registered.iter().map(|(id, _)| *id));
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
    // 3. preferred は PREFERRED_FIRST の配列 index 順に再ソート
    let mut ordered_preferred: Vec<PathBuf> = Vec::new();
    for target in PREFERRED_FIRST {
        if let Some(p) = preferred
            .iter()
            .find(|p| p.file_name().and_then(|f| f.to_str()) == Some(*target))
        {
            ordered_preferred.push(p.clone());
        }
    }
    // 4. preferred + rest を結合
    let mut result = ordered_preferred;
    result.extend(rest);
    Ok(result)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), FontError> {
    let entries = std::fs::read_dir(dir).map_err(|source| FontError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_recursive(&path, out)?;
            continue;
        }
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
        // Ahem.ttf は sort 順でも先頭 (A) だが、PREFERRED_FIRST[0] として
        // **explicit に**先頭に来ることを regression pin。将来 PREFERRED_FIRST
        // に Lato-Medium 等が追加されたとき Ahem が override されないよう
        // 意図明示 (現在は array 1 要素なので default sort と重複するが OK)。
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        write_fake_ttf(tmp.path(), "CSSTest-Regular.ttf");
        write_fake_ttf(tmp.path(), "Lato-Bold.ttf");
        let paths = walk_fonts(tmp.path()).unwrap();
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        // Ahem (PREFERRED_FIRST[0]) → 残りは path sort (CSSTest, Lato-Bold)
        assert_eq!(
            names,
            vec!["Ahem.ttf", "CSSTest-Regular.ttf", "Lato-Bold.ttf",]
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
