//! Document-local backing storage for serialized body rows.

mod compact;
mod glyph_store;

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Range;

pub(super) struct RowStore {
    file: BufWriter<File>,
    ends: Vec<u64>,
    at_end: bool,
    closed: Vec<usize>,
    retained_source_ids: Vec<usize>,
    removed_nodes: usize,
    row_source_ids: Vec<usize>,
}

struct StoredCell<'a> {
    source_id: usize,
    text_id: Option<usize>,
    text: &'a str,
}

struct StoredRow<'a>(&'a [u8]);

impl<'a> StoredRow<'a> {
    fn next_cell(&mut self) -> io::Result<Option<StoredCell<'a>>> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid stored table cell");
        let (cell_id, tail) = self.0.split_first_chunk::<8>().ok_or_else(invalid)?;
        let (text_id, tail) = tail.split_first_chunk::<8>().ok_or_else(invalid)?;
        let (len, tail) = tail.split_first_chunk::<8>().ok_or_else(invalid)?;
        let index = |bytes| usize::try_from(u64::from_le_bytes(bytes)).map_err(|_| invalid());
        let (value, rest) = tail.split_at_checked(index(*len)?).ok_or_else(invalid)?;
        let text = std::str::from_utf8(value).map_err(|_| invalid())?;
        self.0 = rest;
        Ok(Some(StoredCell {
            source_id: index(*cell_id)?,
            text_id: if text.is_empty() {
                None
            } else {
                Some(index(*text_id)?)
            },
            text,
        }))
    }
}

impl RowStore {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            file: BufWriter::new(tempfile::tempfile()?),
            ends: vec![0],
            at_end: true,
            closed: Vec::new(),
            retained_source_ids: Vec::new(),
            removed_nodes: 0,
            row_source_ids: Vec::new(),
        })
    }

    fn visit_rows(
        &mut self,
        range: Range<usize>,
        mut visit: impl FnMut(usize, StoredRow<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        let bytes = self.read_rows(range.clone())?;
        let base = self.ends[range.start];
        for (bounds, &source_id) in self.ends[range.start..=range.end]
            .windows(2)
            .zip(&self.row_source_ids[range])
        {
            let record = &bytes[(bounds[0] - base) as usize..(bounds[1] - base) as usize];
            visit(source_id, StoredRow(record))?;
        }
        Ok(())
    }

    fn source_id(&self, local: usize) -> usize {
        self.retained_source_ids
            .get(local)
            .copied()
            .unwrap_or_else(|| local + self.removed_nodes)
    }

    pub(super) fn append(&mut self, row: &[u8]) -> io::Result<()> {
        let start = *self.ends.last().expect("initial zero offset");
        let end = start
            .checked_add(row.len() as u64)
            .ok_or_else(|| io::Error::other("table row store is too large"))?;
        if !self.at_end {
            self.file.seek(SeekFrom::Start(start))?;
        }
        self.at_end = false;
        self.file.write_all(row)?;
        self.ends.push(end);
        self.at_end = true;
        Ok(())
    }

    pub(super) fn read_rows(&mut self, rows: Range<usize>) -> io::Result<Vec<u8>> {
        let bounds = self
            .ends
            .get(rows.start..=rows.end)
            .filter(|bounds| !bounds.is_empty())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid table row range")
            })?;
        let start = bounds[0];
        let len = usize::try_from(bounds[bounds.len() - 1] - start)
            .map_err(|_| io::Error::other("table row window is too large"))?;
        let mut data = vec![0; len];
        self.at_end = false;
        self.file.seek(SeekFrom::Start(start))?;
        self.file.get_mut().read_exact(&mut data)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests;

pub(super) enum RowCapture {
    Recording(RowStore),
    Unsupported,
    Failed(io::Error),
}

impl RowCapture {
    pub(super) fn finish(self) -> io::Result<Option<RowStore>> {
        match self {
            Self::Recording(mut store) => {
                store.file.flush()?;
                Ok(Some(store))
            }
            Self::Unsupported => Ok(None),
            Self::Failed(error) => Err(error),
        }
    }
}

impl super::RaikiriTreeSink {
    fn capture_table_rows(&self) -> io::Result<std::rc::Rc<std::cell::RefCell<RowCapture>>> {
        let capture = std::rc::Rc::new(std::cell::RefCell::new(RowCapture::Recording(
            RowStore::new()?,
        )));
        self.row_capture.replace(Some(capture.clone()));
        Ok(capture)
    }

    pub(super) fn record_closed_table_row(
        &self,
        row: usize,
        capture: &std::cell::RefCell<RowCapture>,
    ) {
        use markup5ever::ns;

        let mut state = capture.borrow_mut();
        let RowCapture::Recording(store) = &mut *state else {
            return;
        };
        let names = self.qual_names.borrow();
        let html_tag = |id, tag: &str| {
            names.get(&id).is_some_and(|name| {
                name.ns == ns!(html) && AsRef::<str>::as_ref(&name.local) == tag
            })
        };
        if !html_tag(row, "tr")
            || !self
                .document
                .borrow()
                .parent_of(row)
                .is_some_and(|p| html_tag(p, "tbody"))
        {
            return;
        }
        self.flush_text();
        let document = self.document.borrow();
        let attrs = self.attributes.borrow();
        let has_attrs = |id| attrs.get(&id).is_some_and(|list| !list.is_empty());
        let serialize = || {
            if has_attrs(row) {
                return None;
            }
            let mut bytes = Vec::new();
            for &cell in &document.get_node(row)?.children {
                if !html_tag(cell, "td") || has_attrs(cell) {
                    return None;
                }
                let children = &document.get_node(cell)?.children;
                if children.len() > 1 {
                    return None;
                }
                bytes.extend_from_slice(&(store.source_id(cell) as u64).to_le_bytes());
                let text_id = children
                    .first()
                    .map_or(0, |&child| store.source_id(child) as u64);
                bytes.extend_from_slice(&text_id.to_le_bytes());
                let length_at = bytes.len();
                bytes.extend_from_slice(&0u64.to_le_bytes());
                let start = bytes.len();
                for &child in &document.get_node(cell)?.children {
                    bytes.extend_from_slice(document.get_node(child)?.text_content()?.as_bytes());
                }
                let len = (bytes.len() - start) as u64;
                bytes[length_at..start].copy_from_slice(&len.to_le_bytes());
            }
            Some(bytes)
        };
        let row_id = row;
        match serialize() {
            Some(row) => {
                if let Err(error) = store.append(&row) {
                    *state = RowCapture::Failed(error);
                } else {
                    store.row_source_ids.push(store.source_id(row_id));
                    store.closed.push(row_id);
                }
            }
            None => *state = RowCapture::Unsupported,
        }
    }
}

// Retain the exact input until the whole table and its styles are known to be
// eligible. A fallback can then feed this prefix through ordinary checkpoints.
pub(crate) struct TableInput {
    parser: crate::input::Utf8Feed<html5ever::Parser<compact::CompactSink>>,
    capture: std::rc::Rc<std::cell::RefCell<RowCapture>>,
    input: BufWriter<File>,
    bytes: u64,
    byte_limit: Option<u64>,
}

impl TableInput {
    pub(crate) fn new(limits: &raikiri_traits::RenderLimits) -> io::Result<Self> {
        let sink =
            super::RaikiriTreeSink::with_limits(limits.max_parse_warnings, limits.max_dom_nodes);
        let capture = sink.capture_table_rows()?;
        Ok(Self {
            parser: crate::input::Utf8Feed::new(html5ever::parse_document(
                compact::CompactSink(sink),
                Default::default(),
            )),
            capture,
            input: BufWriter::new(tempfile::tempfile()?),
            bytes: 0,
            byte_limit: limits.max_input_bytes,
        })
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<bool, raikiri_traits::ParseError> {
        use raikiri_traits::{LimitKind, ParseError};
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        if let Some(limit) = self.byte_limit
            && self.bytes > limit
        {
            return Err(ParseError::LimitExceeded {
                kind: LimitKind::InputBytes,
                limit,
                actual: self.bytes,
            });
        }
        self.input.write_all(bytes).map_err(ParseError::Io)?;
        for chunk in bytes.chunks(32 * 1024) {
            self.parser.feed(chunk)?;
            let builder = &self.parser.parser().tokenizer.sink;
            if let Some((limit, actual)) = builder.sink.dom_node_limit_error() {
                return Err(ParseError::LimitExceeded {
                    kind: LimitKind::DomNodes,
                    limit,
                    actual,
                });
            }
            compact::reclaim(builder, &self.capture);
            if !matches!(*self.capture.borrow(), RowCapture::Recording(_)) {
                break;
            }
        }
        let mut capture = self.capture.borrow_mut();
        if matches!(*capture, RowCapture::Failed(_))
            && let RowCapture::Failed(error) =
                std::mem::replace(&mut *capture, RowCapture::Unsupported)
        {
            return Err(ParseError::Io(error));
        }
        Ok(matches!(*capture, RowCapture::Recording(_)))
    }

    pub(super) fn finish(
        self,
    ) -> Result<
        (crate::types::UncascadedDocument, Option<RowStore>, File),
        raikiri_traits::ParseError,
    > {
        use raikiri_traits::{LimitKind, ParseError};
        let sink = &self.parser.parser().tokenizer.sink.sink;
        let limit = sink.max_dom_nodes;
        let signal = sink.dom_node_limit_signal();
        let document = self.parser.finish()?;
        if let Some(actual) = signal.get() {
            return Err(ParseError::LimitExceeded {
                kind: LimitKind::DomNodes,
                actual,
                limit: limit.expect("node limit exceeded"),
            });
        }
        let rows = self
            .capture
            .replace(RowCapture::Unsupported)
            .finish()
            .map_err(ParseError::Io)?;
        let input = Self::rewind_input(self.input).map_err(ParseError::Io)?;
        Ok((document, rows, input))
    }

    pub(crate) fn finish_windows(
        self,
        options: &crate::ParseOptions<'_>,
    ) -> Result<(Option<TableWindows>, File), raikiri_traits::ParseError> {
        let (shell, rows, input) = self.finish()?;
        let Some(mut windows) = rows.and_then(|rows| TableWindows::new(shell, rows)) else {
            return Ok((None, input));
        };
        windows.shell = crate::parse::finish_document(windows.shell, options)?;
        Ok((Some(windows), input))
    }

    pub(crate) fn recorded_rows(&self) -> usize {
        match &*self.capture.borrow() {
            RowCapture::Recording(store) => store.ends.len() - 1,
            _ => 0,
        }
    }

    pub(crate) fn into_replay(self) -> io::Result<File> {
        Self::rewind_input(self.input)
    }

    fn rewind_input(input: BufWriter<File>) -> io::Result<File> {
        let mut file = input.into_inner().map_err(|error| error.into_error())?;
        file.rewind()?;
        Ok(file)
    }
}

pub(crate) struct PlannedTableWindow {
    pub(crate) rows: Range<usize>,
    pub(crate) pages: Range<usize>,
}

pub(crate) struct TableWindows {
    shell: crate::types::UncascadedDocument,
    rows: RowStore,
    group: usize,
    columns: Option<raikiri_dom::layout::MeasuredTableColumns>,
    glyphs: Option<glyph_store::GlyphStore>,
}

impl TableWindows {
    fn new(shell: crate::types::UncascadedDocument, rows: RowStore) -> Option<Self> {
        let dom = &shell.dom;
        let mut table = None;
        let mut group = None;
        for id in dom.preorder(dom.root_index()).skip(1) {
            let node = dom.get_node(id)?;
            let parent = dom.parent_of(id).and_then(|p| dom.get_node(p)?.tag_name());
            if let Some(text) = node.text_content() {
                if !matches!(parent, Some("style" | "title" | "th" | "td"))
                    && !text
                        .bytes()
                        .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
                {
                    return None;
                }
                continue;
            }
            let Some(tag) = node.tag_name() else { continue };
            if node.namespace_uri().is_some()
                || [
                    "id", "name", "href", "rowspan", "colspan", "style", "hidden",
                ]
                .iter()
                .any(|attr| node.attribute(attr).is_some())
            {
                return None;
            }
            match (parent, tag) {
                (None, "html")
                | (Some("html"), "head" | "body")
                | (Some("head"), "style" | "title" | "meta")
                | (Some("table"), "thead")
                | (Some("thead"), "tr")
                | (Some("tr"), "th" | "td") => {}
                (Some("body"), "table") if table.replace(id).is_none() => {}
                (Some("table"), "tbody") if group.is_none() && dom.parent_of(id) == table => {
                    group = Some(id);
                }
                _ => return None,
            }
        }
        Some(Self {
            shell,
            rows,
            group: group?,
            columns: None,
            glyphs: None,
        })
    }

    pub(crate) fn permits_window_styles(
        &self,
        registrations: &[crate::ConsumerPropertyRegistration],
        limits: &raikiri_traits::RenderLimits,
    ) -> Result<(bool, bool), raikiri_traits::RenderError> {
        let tree = crate::cascade::build_rule_tree_with_limits(
            &self.shell,
            registrations,
            &limits.rule_tree_limits(),
        )?;
        let windows = tree.permits_table_window_styles();
        Ok((windows, windows && tree.permits_table_page_templates()))
    }

    pub(crate) fn preferred_width(&self) -> Option<f32> {
        self.columns
            .as_ref()
            .map(|columns| columns.preferred_width())
    }

    pub(crate) fn table(&self) -> usize {
        self.shell
            .dom
            .parent_of(self.group)
            .expect("validated table body")
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.ends.len() - 1
    }

    pub(crate) fn read_text_rows(
        &mut self,
        range: Range<usize>,
        next_cell: &mut usize,
    ) -> io::Result<Vec<Vec<crate::TableTextCell>>> {
        let mut ordinal = *next_cell;
        let mut rows = Vec::with_capacity(range.len());
        self.rows.visit_rows(range, |_, mut record| {
            let mut cells = Vec::new();
            while let Some(cell) = record.next_cell()? {
                cells.push(crate::TableTextCell {
                    ordinal,
                    cell: raikiri_traits::NodeId(cell.source_id as u64),
                    node: cell.text_id.map(|id| raikiri_traits::NodeId(id as u64)),
                    text: cell.text.to_owned(),
                });
                ordinal += 1;
            }
            rows.push(cells);
            Ok(())
        })?;
        *next_cell = ordinal;
        Ok(rows)
    }

    pub(crate) fn measured_cell(
        &mut self,
        cell: &crate::TableTextCell,
    ) -> Option<raikiri_dom::MeasuredTableCell> {
        let glyphs = self.glyphs.as_mut()?;
        raikiri_dom::MeasuredTableCell::from_record(&cell.text, |bytes| {
            glyphs.read(cell.ordinal, bytes)
        })
    }

    pub(crate) fn discard_glyph_records(&mut self) {
        self.glyphs = None;
    }

    pub(crate) fn measure_columns(
        &mut self,
        window_size: std::num::NonZeroUsize,
        glyph_budget: u64,
        measure: impl Fn(crate::types::UncascadedDocument) -> Result<(), raikiri_traits::RenderError>
        + Sync,
    ) -> Result<(), raikiri_traits::RenderError> {
        use raikiri_dom::layout::TableColumnMeasurements;

        let measurements = TableColumnMeasurements::default();
        let mut glyphs = glyph_store::GlyphStore::new(glyph_budget);
        let mut next_cell = 0;
        let len = self.len();
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        for start in (0..len).step_by(window_size.get()) {
            let end = start.saturating_add(window_size.get()).min(len);
            let count = workers.min(((end - start) / 128).max(1));
            let chunk = (end - start).div_ceil(count);
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(count - 1);
                let mut recorders = Vec::new();
                let measure = &measure;
                for first in (start..end).step_by(chunk) {
                    let last = first.saturating_add(chunk).min(end);
                    let mut sources = Vec::new();
                    let store = glyphs
                        .as_ref()
                        .filter(|store| store.can_record_from(next_cell));
                    let mut document = self
                        .document_with_cell_sources(first..last, store.map(|_| &mut sources))
                        .map_err(raikiri_traits::ParseError::Io)?;
                    let first_cell = next_cell;
                    next_cell += sources.len();
                    if let Some(recorder) =
                        store.and_then(|store| store.window(first_cell, sources))
                    {
                        measurements
                            .measure_window_with_recorder(&mut document.dom, recorder.clone());
                        recorders.push(recorder);
                    } else {
                        measurements.measure_window(&mut document.dom);
                    }
                    if last == end {
                        measure(document)?;
                    } else {
                        handles.push(
                            std::thread::Builder::new()
                                .spawn_scoped(scope, move || measure(document))
                                .map_err(raikiri_traits::RenderError::Io)?,
                        );
                    }
                }
                for handle in handles {
                    handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
                }
                if let Some(store) = &mut glyphs {
                    for recorder in recorders {
                        store.append(&recorder);
                    }
                }
                Ok::<_, raikiri_traits::RenderError>(())
            })?;
        }
        self.glyphs = glyphs;
        self.columns = Some(measurements.finish());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn plan_pages(
        &mut self,
        lookahead: std::num::NonZeroUsize,
        layout: impl FnMut(
            crate::types::UncascadedDocument,
        ) -> Result<
            (crate::HtmlDocument, crate::DocumentLayout),
            raikiri_traits::RenderError,
        >,
    ) -> Result<Option<Vec<PlannedTableWindow>>, raikiri_traits::RenderError> {
        self.plan_pages_with(lookahead, layout, |_, _| Ok(()))
    }

    pub(crate) fn plan_pages_with(
        &mut self,
        lookahead: std::num::NonZeroUsize,
        mut layout: impl FnMut(
            crate::types::UncascadedDocument,
        ) -> Result<
            (crate::HtmlDocument, crate::DocumentLayout),
            raikiri_traits::RenderError,
        >,
        mut visit: impl FnMut(
            &crate::DocumentLayout,
            Range<usize>,
        ) -> Result<(), raikiri_traits::RenderError>,
    ) -> Result<Option<Vec<PlannedTableWindow>>, raikiri_traits::RenderError> {
        let mut plan = Vec::new();
        let mut overlap = Vec::new();
        let mut start = 0;
        let mut first_new = 0usize;
        let mut read_ahead = lookahead.get();
        while first_new < self.len() {
            // Replaying the preceding page makes the first emitted header a
            // repeated header with the same paint treatment as the full table.
            let skip = usize::from(!plan.is_empty());
            let end = if self.len() - first_new <= lookahead.get() {
                self.len()
            } else {
                first_new + read_ahead
            };
            let rows = start..end;
            let shell = self
                .document(rows.clone())
                .map_err(raikiri_traits::ParseError::Io)?;
            let (document, result) = layout(shell)?;
            let Some((complete_pages, guard, next)) = self.page_prefix(
                rows.clone(),
                &document,
                &result,
                first_new,
                skip,
                &mut overlap,
            ) else {
                return Ok(None);
            };
            if next <= first_new || complete_pages <= skip {
                return Ok(None);
            }
            // Keep one row beyond a predicted full page to confirm its break.
            // The prefix check still decides which pages are final.
            let last_page_rows = next - guard;
            if last_page_rows > 0 && last_page_rows < lookahead.get() {
                read_ahead = ((lookahead.get() - 1) / last_page_rows) * last_page_rows + 1;
            }
            visit(&result, skip..complete_pages)?;
            plan.push(PlannedTableWindow {
                rows,
                pages: skip..complete_pages,
            });
            start = guard;
            first_new = next;
        }
        Ok(Some(plan))
    }

    // Return the complete page count, preceding page start and next output row. The
    // last page of a nonfinal window can still change as more rows arrive.
    fn page_prefix(
        &self,
        range: Range<usize>,
        document: &crate::HtmlDocument,
        layout: &crate::DocumentLayout,
        first_new: usize,
        skip: usize,
        overlap: &mut Vec<raikiri_traits::PaintRect>,
    ) -> Option<(usize, usize, usize)> {
        let dom = document.dom();
        let rows: Vec<_> = dom
            .get_node(self.group)?
            .children
            .iter()
            .copied()
            .filter(|&id| dom.get_node(id).and_then(|node| node.tag_name()) == Some("tr"))
            .collect();
        if rows.len() != range.len() {
            return None;
        }
        let page_count = layout.pages().len();
        let complete = if range.end == self.len() {
            page_count
        } else {
            page_count.checked_sub(1)?
        };
        let mut next = 0;
        let mut confirmed = 0;
        let mut guard = 0;
        let previous = std::mem::take(overlap);
        for (index, page) in layout.pages().enumerate() {
            let before = next;
            if index == skip && range.start + before != first_new {
                return None;
            }
            if index + 1 == complete {
                guard = before;
            }
            for fragment in page.fragments() {
                let id = fragment.node().0 as usize;
                if let Ok(position) = rows.binary_search(&id) {
                    if position != next {
                        return None;
                    }
                    if let Some(expected) = (range.start + position)
                        .checked_sub(first_new)
                        .and_then(|n| previous.get(n))
                        && (index != skip || *expected != fragment.rect())
                    {
                        return None;
                    }
                    if index >= complete {
                        overlap.push(fragment.rect());
                    }
                    next += 1;
                }
            }
            if next == before {
                return None;
            }
            if index < complete {
                confirmed = next;
            }
        }
        (next == rows.len()).then_some((complete, range.start + guard, range.start + confirmed))
    }

    pub(crate) fn document(
        &mut self,
        range: Range<usize>,
    ) -> io::Result<crate::types::UncascadedDocument> {
        self.document_with_cell_sources(range, None)
    }

    fn document_with_cell_sources(
        &mut self,
        range: Range<usize>,
        mut cell_sources: Option<&mut Vec<usize>>,
    ) -> io::Result<crate::types::UncascadedDocument> {
        let mut dom = self.shell.dom.clone();
        if let Some(columns) = &self.columns {
            columns.apply_to_window(&mut dom);
        }
        let mut ids: Vec<_> = (0..dom.node_count())
            .map(|id| self.rows.source_id(id))
            .collect();
        self.rows.visit_rows(range, |row_id, mut stored| {
            ids.push(row_id);
            let row = dom.append_element(Some(self.group), "tr", None::<&str>);
            while let Some(cell) = stored.next_cell()? {
                ids.push(cell.source_id);
                if let Some(sources) = &mut cell_sources {
                    sources.push(cell.source_id);
                }
                let id = dom.append_element(Some(row), "td", None::<&str>);
                if let Some(text_id) = cell.text_id {
                    dom.append_text(id, cell.text);
                    ids.push(text_id);
                }
            }
            Ok(())
        })?;
        dom.set_window_source_ids(ids);
        Ok(crate::types::UncascadedDocument {
            dom,
            stylesheet_sources: self.shell.stylesheet_sources.clone(),
            user_stylesheet_sources: self.shell.user_stylesheet_sources.clone(),
            user_stylesheet_insertion_index: self.shell.user_stylesheet_insertion_index,
            warnings: self.shell.warnings.clone(),
            quirks_mode: self.shell.quirks_mode,
        })
    }
}
