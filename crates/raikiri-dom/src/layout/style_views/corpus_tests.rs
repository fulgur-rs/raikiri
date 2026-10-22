use crate::{
    Document,
    snapshot::{LogicalData, LogicalSnapshot},
};
use raikiri_traits::{FetchOutcome, NetworkError, NetworkProvider, Request};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn transport(parsed: &raikiri_html::UncascadedDocument) -> Document {
    let snapshot: LogicalSnapshot =
        serde_json::from_value(serde_json::to_value(parsed.dom.logical_snapshot()).unwrap())
            .unwrap();
    Document::from_logical_snapshot(snapshot, 1_000_000).unwrap()
}

#[derive(Clone)]
struct LocalResources {
    root: PathBuf,
}

impl LocalResources {
    fn rebase(&self, url: &url::Url) -> url::Url {
        let root = url::Url::from_directory_path(&self.root).unwrap();
        if url.scheme() == "file" && !url.path().starts_with(root.path()) {
            root.join(url.path().trim_start_matches('/')).unwrap()
        } else {
            url.clone()
        }
    }

    fn local_path(&self, url: &url::Url) -> Option<PathBuf> {
        let path = self.rebase(url).to_file_path().ok()?.canonicalize().ok()?;
        path.starts_with(&self.root).then_some(path)
    }
}

impl NetworkProvider for LocalResources {
    fn fetch_one_hop(&self, mut request: Request) -> Result<FetchOutcome, NetworkError> {
        request.url = self.rebase(&request.url);
        if request.url.scheme() == "file" && self.local_path(&request.url).is_none() {
            return Err(NetworkError::Other(format!(
                "unavailable local resource: {}",
                request.url
            )));
        }
        raikiri_net::FileNetworkProvider.fetch_one_hop(request)
    }
}

struct FontLoader<'a> {
    resources: &'a LocalResources,
    base: &'a url::Url,
}
impl crate::FontFaceLoader for FontLoader<'_> {
    fn load(&self, value: &str) -> Option<Vec<u8>> {
        let path = self.resources.local_path(&self.base.join(value).ok()?)?;
        if path.metadata().ok()?.len() > 100 * 1024 * 1024 {
            return None;
        }
        std::fs::read(path).ok()
    }
}

struct BrokenImageFallback<'a>(&'a dyn raikiri_traits::ReplacedResolver);
impl raikiri_traits::ReplacedResolver for BrokenImageFallback<'_> {
    fn resolve(
        &self,
        request: raikiri_traits::ResolverRequest<'_>,
    ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
        use raikiri_traits::{IntrinsicBox, ResolveDisposition, ResolvedIntrinsic, ResolverError};
        match self.0.resolve(request) {
            Ok(value) => Ok(value),
            Err(ResolverError::Decode(message))
                if message == "image cache resource limit exceeded" =>
            {
                Err(ResolverError::Decode(message))
            }
            Err(error) => Ok(ResolvedIntrinsic {
                intrinsic: IntrinsicBox::default(),
                disposition: ResolveDisposition::Fallback {
                    reason: error.to_string(),
                },
            }),
        }
    }
}

#[derive(Default, Debug)]
struct Totals {
    documents: usize,
    roles: [usize; 8],
    iterators: usize,
    calcs: usize,
    pass: usize,
    scoped: usize,
    restored: usize,
    expected_missing_stylesheets: usize,
}

fn expected_missing_stylesheet(document: &Path, resource: &url::Url, root: &Path) -> bool {
    // git ls-tree at WPT 97ea26e26a2aac3eec7e770650b25e7049ed4a4e has neither target.
    // Keep the normal browser fallback; rewriting them to support/flexbox.css changes the input.
    const DANGLING: [(&str, &str); 2] = [
        (
            "css/css-flexbox/percentage-descendant-of-anonymous-flex-item.html",
            "css/css-flexbox/resources/flexbox.css",
        ),
        (
            "css/css-flexbox/reference/flexbox-with-multi-column-property-ref.html",
            "css/css-flexbox/reference/resources/flexbox.css",
        ),
    ];
    let Ok(resource) = resource.to_file_path() else {
        return false;
    };
    DANGLING.iter().any(|(page, stylesheet)| {
        document == root.join(page)
            && resource == root.join(stylesheet)
            && std::fs::symlink_metadata(&resource)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    })
}

fn compare_document(
    path: &Path,
    resources: &LocalResources,
    shared: &shodo::font::FontCollection,
    totals: &mut Totals,
) -> Vec<PathBuf> {
    let base = url::Url::from_file_path(path).unwrap();
    let parsed = raikiri_html::parse(
        std::fs::File::open(path).unwrap(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: Some(resources),
            base_url: Some(base.clone()),
        },
    )
    .unwrap();
    let resource_failures: Vec<_> = parsed
        .warnings
        .iter()
        .filter(|warning| {
            if let raikiri_traits::WarningKind::NetworkFallback { url } = &warning.kind
                && expected_missing_stylesheet(path, url, &resources.root)
            {
                totals.expected_missing_stylesheets += 1;
                return false;
            }
            matches!(
                warning.kind,
                raikiri_traits::WarningKind::NetworkFallback { .. }
                    | raikiri_traits::WarningKind::PolicyWarning { .. }
            )
        })
        .collect();
    assert!(
        resource_failures.is_empty(),
        "stylesheet inputs unavailable: {resource_failures:?}"
    );
    let mut document = transport(&parsed);
    let mut references = Vec::new();
    for node in &document.logical_snapshot().nodes {
        if let LogicalData::Element {
            tag, attributes, ..
        } = &node.data
            && tag == "link"
            && attributes.iter().any(|a| {
                a.local == "rel"
                    && a.value
                        .split_ascii_whitespace()
                        .any(|v| v == "match" || v == "mismatch")
            })
            && let Some(href) = attributes.iter().find(|a| a.local == "href")
        {
            let reference = base.join(&href.value).unwrap();
            references.push(
                resources
                    .local_path(&reference)
                    .unwrap_or_else(|| panic!("missing reference: {reference}")),
            );
        }
    }
    let tree = raikiri_html::build_rule_tree(&parsed);
    let media =
        raikiri_style::MediaContext::with_viewport(raikiri_style::MediaType::Screen, 800, 600);
    let (fonts, font_report) = crate::build_inline_document_fonts(
        shared,
        &tree.font_faces_for(&media),
        &FontLoader {
            resources,
            base: &base,
        },
    );
    assert!(
        font_report.skipped.is_empty(),
        "font inputs unavailable: {font_report:?}"
    );
    document.set_font_collection(fonts);
    let resolver = raikiri_net::ImageResolver::new(resources.clone());
    crate::image_resolve::resolve_images_with_base(
        &mut document,
        &BrokenImageFallback(&resolver),
        Some(&base),
    )
    .unwrap();
    let mut options = raikiri_style::CascadeOptions::default();
    options.viewport = Some((800.0, 600.0));
    options.border_width_device_scale = Some(1.0);
    let cascade = raikiri_style::cascade_with_options(
        &document,
        &tree,
        &media,
        &raikiri_style::PageContextQuery::default(),
        &options,
    )
    .unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    crate::layout::page_pipeline::layout_single_page_checked(&mut document, &cascade, page)
        .unwrap();
    let stats = &document.style_view_comparisons;
    totals.documents += 1;
    for (count, reads) in totals.roles.iter_mut().zip(&stats.getter_reads) {
        *count += reads.get();
    }
    totals.iterators += stats.iterator_reads.get();
    totals.calcs += stats.calc_comparisons.get();
    totals.pass += stats.pass_reads.get();
    totals.scoped += stats.scoped_reads.get();
    totals.restored += stats.restored_reads.get();
    references
}

#[test]
#[ignore = "requires the fetched WPT checkout and bundled fonts"]
fn flex_grid_baseline_getters_agree() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repo
        .join("target/wpt")
        .canonicalize()
        .expect("run scripts/wpt/fetch.sh first");
    let resources = LocalResources { root: root.clone() };
    let fonts = crate::build_wpt_font_collection(&root.join("fonts")).unwrap();
    let ids: Vec<_> = include_str!("../../../../../expectations/raikiri-baseline.txt")
        .lines()
        .map(str::trim)
        .filter(|id| {
            id.starts_with("css/css-flexbox/")
                || id.starts_with("css/css-grid/")
                || id.starts_with("css/CSS2/normal-flow/")
                || id.starts_with("css/CSS2/positioning/")
                || id.starts_with("css/CSS2/abspos/")
                || id.starts_with("css/CSS2/visuren/")
                || id.starts_with("css/CSS2/visudet/")
                || id.starts_with("css/CSS2/floats/")
                || id.starts_with("css/CSS2/floats-clear/")
                || id.starts_with("css/CSS2/margin-padding-clear/")
        })
        .collect();
    assert_eq!(ids.len(), 953, "baseline changed: review corpus coverage");
    let filter = std::env::var("RAIKIRI_STYLE_CORPUS_FILTER").ok();
    let mut pending: BTreeSet<_> = ids
        .iter()
        .filter(|id| filter.as_ref().is_none_or(|f| id.contains(f)))
        .map(|id| root.join(id.trim_start_matches('/')))
        .collect();
    assert!(!pending.is_empty());
    let mut visited = BTreeSet::new();
    let mut totals = Totals::default();
    let mut failures = Vec::new();
    while let Some(path) = pending.pop_first() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compare_document(&path, &resources, &fonts, &mut totals)
        }));
        match result {
            Ok(references) => {
                pending.extend(references.into_iter().filter(|p| !visited.contains(p)))
            }
            Err(error) => {
                let message = error
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into());
                failures.push(format!("{}: {message}", path.display()));
            }
        }
    }
    eprintln!(
        "STYLE CORPUS: {totals:?}; visited={}; failures={}",
        visited.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    if filter.is_none() {
        assert_eq!(
            totals.expected_missing_stylesheets, 2,
            "review stale WPT resource exceptions"
        );
        assert!(totals.documents >= ids.len());
        assert!(totals.roles[..4].iter().all(|count| *count > 0));
        assert!(
            totals.iterators > 0
                && totals.calcs > 0
                && totals.pass > 0
                && totals.scoped > 0
                && totals.restored > 0
        );
    }
}

#[test]
fn parser_transport_preserves_logical_inputs() {
    let parsed = raikiri_html::parse(
        &b"<html><head><style>.x { display: grid }</style></head><body><div class=x style='width:23px' data-value='kept'>text</div><template><b>inert</b></template></body></html>"[..],
        &raikiri_html::ParseOptions { extra_stylesheets: &[], network: None, base_url: None },
    )
    .unwrap();
    let expected = serde_json::to_value(parsed.dom.logical_snapshot()).unwrap();
    let document = transport(&parsed);
    assert_eq!(
        serde_json::to_value(document.logical_snapshot()).unwrap(),
        expected
    );
    assert_ne!(document.logical_snapshot().quirks, 0);
    let tree = raikiri_html::build_rule_tree(&parsed);
    let cascade = raikiri_style::cascade(&document, &tree).unwrap();
    assert!(
        cascade
            .computed
            .iter()
            .any(|cv| cv.width == raikiri_style::ComputedLengthPercentageOrAuto::Px(23.0))
    );
}

#[test]
fn missing_stylesheet_or_font_is_not_a_successful_comparison() {
    for head in [
        "<link rel=stylesheet href=missing.css>",
        "<style>@font-face { font-family:Missing; src:url(missing.ttf) } body {font-family:Missing}</style>",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("test.html");
        std::fs::write(&path, format!("<!doctype html><html><head>{head}</head><body style='display:flex'><div>test</div></body></html>")).unwrap();
        let resources = LocalResources { root };
        let fonts = crate::layout::test_support::ifc_ahem_fonts();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compare_document(&path, &resources, &fonts, &mut Totals::default());
        }));
        assert!(
            result.is_err(),
            "missing input was silently accepted: {head}"
        );
    }
}

#[test]
fn missing_stylesheet_exceptions_are_exact_and_expire_when_the_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let page = root.join("css/css-flexbox/percentage-descendant-of-anonymous-flex-item.html");
    let missing = root.join("css/css-flexbox/resources/flexbox.css");
    let url = url::Url::from_file_path(&missing).unwrap();
    assert!(expected_missing_stylesheet(&page, &url, &root));
    assert!(!expected_missing_stylesheet(
        &root.join("another.html"),
        &url,
        &root
    ));
    assert!(!expected_missing_stylesheet(
        &page,
        &url.join("another.css").unwrap(),
        &root
    ));
    std::fs::create_dir_all(missing.parent().unwrap()).unwrap();
    std::fs::write(&missing, "body { display:flex }").unwrap();
    assert!(!expected_missing_stylesheet(&page, &url, &root));
}
