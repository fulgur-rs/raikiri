//! Prepared real-cache workloads for the external allocation benchmark.

use super::{CachedBackgroundSource, DecodedImageCache, ProcessImageBudget};
use raikiri_svg::SvgDocument;
use raikiri_traits::DecodedImage;
use std::sync::Arc;
use url::Url;

/// Insertion patterns whose source payloads are prepared before measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum CacheAccountingCase {
    NewSources,
    TreeInput,
    SameUrl,
    Shared,
    DistinctEqual,
    Mixed,
    CacheDenied,
    ProcessDenied,
    Sparse,
}

impl CacheAccountingCase {
    pub const ALL: [Self; 9] = [
        Self::NewSources,
        Self::TreeInput,
        Self::SameUrl,
        Self::Shared,
        Self::DistinctEqual,
        Self::Mixed,
        Self::CacheDenied,
        Self::ProcessDenied,
        Self::Sparse,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::NewSources => "new-sources",
            Self::TreeInput => "tree-input",
            Self::SameUrl => "same-url",
            Self::Shared => "shared-arc",
            Self::DistinctEqual => "distinct-equal",
            Self::Mixed => "mixed-raster",
            Self::CacheDenied => "cache-denied",
            Self::ProcessDenied => "process-denied",
            Self::Sparse => "sparse-ledger",
        }
    }
}

/// Cache ownership and operation counts, not a physical heap measurement.
#[derive(Debug, Clone, Copy)]
#[doc(hidden)]
pub struct CacheAccountingResult {
    pub attempts: usize,
    pub rejected: usize,
    pub entries: usize,
    pub svg_entries: usize,
    pub accounted_bytes: u64,
    pub process_bytes: u64,
    /// Logical cache/ledger map operations; each BTree operation costs O(log N).
    pub work: u64,
    pub ledger_entries: usize,
    pub ledger_accounted_bytes: u64,
}

/// An opaque prepared cache: source parsing and URL creation precede run().
#[doc(hidden)]
pub struct CacheAccountingProbe {
    cache: DecodedImageCache,
    operations: Vec<(Url, CachedBackgroundSource)>,
    attempts: usize,
    rejected: usize,
}

impl CacheAccountingProbe {
    pub fn prepare(case: CacheAccountingCase, count: usize) -> Self {
        assert!(count > 0, "probe count must be positive");
        let mut cache = DecodedImageCache::new(Arc::new(ProcessImageBudget::new(u64::MAX, 4)));
        cache.max_bytes = u64::MAX;
        let prefill = match case {
            CacheAccountingCase::SameUrl => 1,
            CacheAccountingCase::CacheDenied
            | CacheAccountingCase::ProcessDenied
            | CacheAccountingCase::Sparse => count,
            _ => 0,
        };
        for index in 0..prefill {
            let url = if case == CacheAccountingCase::SameUrl {
                probe_url("same", 0)
            } else {
                probe_url("initial", index)
            };
            cache
                .insert_source(url, CachedBackgroundSource::Svg(probe_source(Some(index))))
                .expect("prepared cache fits its unlimited budget");
        }
        if case == CacheAccountingCase::CacheDenied {
            cache.max_bytes = cache.decoded_bytes;
        }
        if case == CacheAccountingCase::ProcessDenied {
            let retained = cache.process_budget.retained();
            Arc::get_mut(&mut cache.process_budget)
                .expect("the prepared probe exclusively owns its budget")
                .max_retained = retained;
        }
        let shared = (case == CacheAccountingCase::Shared).then(|| probe_source(None));
        let raster = matches!(
            case,
            CacheAccountingCase::Mixed | CacheAccountingCase::Sparse
        )
        .then(|| Arc::new(DecodedImage::new(1, 1, vec![0; 4])));
        let operations = (0..if case == CacheAccountingCase::Sparse {
            count - 1
        } else {
            count
        })
            .map(|index| {
                let url = match case {
                    CacheAccountingCase::SameUrl => probe_url("same", 0),
                    CacheAccountingCase::Sparse => probe_url("initial", index),
                    _ => probe_url("new", index),
                };
                let source = if case == CacheAccountingCase::Sparse
                    || (case == CacheAccountingCase::Mixed && index % 2 == 1)
                {
                    CachedBackgroundSource::Raster(Arc::clone(
                        raster.as_ref().expect("raster fixture"),
                    ))
                } else if let Some(shared) = &shared {
                    CachedBackgroundSource::Svg(Arc::clone(shared))
                } else {
                    let id = (case != CacheAccountingCase::DistinctEqual).then_some(index + count);
                    CachedBackgroundSource::Svg(probe_document(
                        id,
                        case != CacheAccountingCase::TreeInput,
                    ))
                };
                (url, source)
            })
            .collect();
        Self {
            cache,
            operations,
            attempts: 0,
            rejected: 0,
        }
    }

    pub fn run(&mut self, count_work: bool) {
        self.cache.work.begin(count_work);
        for (url, source) in std::mem::take(&mut self.operations) {
            self.attempts += 1;
            if self.cache.insert_source(url, source).is_err() {
                self.rejected += 1;
            }
        }
        self.cache.work.finish();
    }

    pub fn result(&self) -> CacheAccountingResult {
        CacheAccountingResult {
            attempts: self.attempts,
            rejected: self.rejected,
            entries: self.cache.entries.len(),
            svg_entries: self
                .cache
                .entries
                .values()
                .filter(|entry| matches!(entry.source, CachedBackgroundSource::Svg(_)))
                .count(),
            accounted_bytes: self.cache.decoded_bytes,
            process_bytes: self.cache.process_budget.retained(),
            work: self.cache.work.value(),
            ledger_entries: self.cache.source_ledger.len(),
            ledger_accounted_bytes: self.cache.source_ledger.accounted_bytes(),
        }
    }
}

fn probe_url(group: &str, index: usize) -> Url {
    Url::parse(&format!("https://account.test/{group}/{index:04}.svg")).expect("fixture URL")
}

fn probe_source(index: Option<usize>) -> Arc<SvgDocument> {
    probe_document(index, true)
}

fn probe_document(index: Option<usize>, compact: bool) -> Arc<SvgDocument> {
    let suffix = index.map_or_else(String::new, |index| format!("<!--source {index:04}-->"));
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/>{suffix}</svg>"
    );
    let document = Arc::new(SvgDocument::parse(source.as_bytes()).expect("fixture SVG"));
    if compact {
        document.into_source_only()
    } else {
        document
    }
}
