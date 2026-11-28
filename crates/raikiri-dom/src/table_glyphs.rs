//! Reuse of short, single-run text shaped during table column measurement.

use crate::{FontId, Glyph, PositionedGlyphRun};

// The header holds the blob id, face index, four metrics and byte length.
// It is followed by the text itself and an id/advance pair per ASCII byte.
const HEADER: usize = 29;
const MAX_TEXT: usize = 20;
const GLYPH_BYTES: usize = 8;
const MAX_RECORD: usize = HEADER + MAX_TEXT * (1 + GLYPH_BYTES);
// At this magnitude, f32 stops distinguishing every Shodo 1/64 px layout unit.
const EXACT_POSITION_LIMIT: f32 = (1 << 18) as f32;

fn eligible_text(text: &str) -> bool {
    (1..=MAX_TEXT).contains(&text.len())
        && text.is_ascii()
        && !text.bytes().any(|b| b.is_ascii_control())
}

fn record_len(text: &str) -> usize {
    HEADER + text.len() * (1 + GLYPH_BYTES)
}

pub(crate) fn capture(
    store: &dyn crate::TableGlyphRecorder,
    id: usize,
    text: &str,
    p: &shodo::Paragraph,
    options: &shodo::style::LineOptions,
    cx: &mut Option<shodo::LayoutContext>,
) {
    // Justification can change advances when the cell gets its final width.
    if !eligible_text(text)
        || text != p.text()
        || options.text_align == shodo::style::TextAlign::JustifyAll
        || options.text_align_last == shodo::style::TextAlignLast::Justify
        || !store.can_write(id, record_len(text))
    {
        return;
    }
    // Intrinsic sizing returns widths. Shodo exposes glyphs on a materialized line.
    let result = p.next_line(
        cx.get_or_insert_default(),
        p.start_token(),
        options,
        &shodo::LineConstraint::new(100_000.0),
        &shodo::AtomicSizes::EMPTY,
    );
    capture_line(store, id, text, result);
}

fn capture_line(
    store: &dyn crate::TableGlyphRecorder,
    id: usize,
    text: &str,
    result: shodo::LineResult,
) {
    let shodo::LineResult::Line(line) = result else {
        return;
    };
    if line.break_reason() != shodo::BreakReason::End
        || line.writing_mode() != shodo::geometry::WritingMode::HorizontalTb
        || line.used_direction() != shodo::geometry::Direction::Ltr
    {
        return;
    }
    let mut fragments = line.fragments();
    let Some(shodo::Fragment::GlyphRun(run)) = fragments.next() else {
        return;
    };
    let origin = run.inline_start();
    if fragments.next().is_some()
        || origin.abs() >= EXACT_POSITION_LIMIT
        || run.text_range() != (0..text.len())
        || run.is_ellipsis()
        || run.bidi_level() != 0
        || run.embolden()
        || run.skew().is_some()
        || !run.normalized_coords().is_empty()
        || !run.variations().is_empty()
    {
        return;
    }
    let Some(font) = run.font_data() else { return };
    let mut bytes = [0; MAX_RECORD];
    bytes[..8].copy_from_slice(&font.data.id().to_le_bytes());
    bytes[8..12].copy_from_slice(&font.index.to_le_bytes());
    let metrics = run.metrics();
    for (slot, value) in bytes[12..28].chunks_exact_mut(4).zip([
        run.font_size(),
        metrics.ascent,
        metrics.descent,
        run.inline_size(),
    ]) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    bytes[28] = text.len() as u8;
    let glyph_start = HEADER + text.len();
    bytes[HEADER..glyph_start].copy_from_slice(text.as_bytes());
    let mut glyphs = run.glyphs();
    let mut pen = 0.;
    for (cluster, slot) in bytes[glyph_start..record_len(text)]
        .chunks_exact_mut(GLYPH_BYTES)
        .enumerate()
    {
        let Some(glyph) = glyphs.next() else { return };
        if glyph.cluster as usize != cluster
            || glyph.inline_position.abs() >= EXACT_POSITION_LIMIT
            || f64::from(glyph.inline_position) - f64::from(origin) != f64::from(pen)
            || glyph.block_offset != 0.
        {
            return;
        }
        slot[..4].copy_from_slice(&glyph.id.to_le_bytes());
        slot[4..].copy_from_slice(&glyph.advance.to_le_bytes());
        pen += glyph.advance;
    }
    if glyphs.next().is_none() && pen == run.inline_size() {
        store.write(id, &bytes[..record_len(text)]);
    }
}

fn decode(record: &[u8], text: &str) -> Option<MeasuredTableCell> {
    let glyph_start = HEADER + text.len();
    if record.len() != record_len(text)
        || record[28] as usize != text.len()
        || &record[HEADER..glyph_start] != text.as_bytes()
    {
        return None;
    }
    let word = |i| u32::from_le_bytes(record[i..i + 4].try_into().unwrap());
    let metric = |i| f32::from_bits(word(i));
    let mut glyphs = Vec::with_capacity(text.len());
    let mut pen = 0.;
    for i in 0..text.len() {
        let offset = glyph_start + i * GLYPH_BYTES;
        let advance = metric(offset + 4);
        if !advance.is_finite() || advance < 0. {
            return None;
        }
        glyphs.push(Glyph {
            id: word(offset),
            advance,
            x_offset: 0.,
            y_offset: 0.,
            text_range: i..i + 1,
        });
        pen += advance;
    }
    let advance = metric(24);
    if !advance.is_finite() || pen != advance {
        return None;
    }
    Some(MeasuredTableCell {
        text: text.into(),
        face: FontId::from_blob_index(u64::from_le_bytes(record[..8].try_into().unwrap()), word(8)),
        size: metric(12),
        ascent: metric(16),
        descent: metric(20),
        advance,
        glyphs,
    })
}

/// Glyphs and metrics retained from measuring one table cell.
#[derive(Debug)]
pub struct MeasuredTableCell {
    text: Box<str>,
    face: FontId,
    size: f32,
    ascent: f32,
    descent: f32,
    advance: f32,
    glyphs: Vec<Glyph>,
}

impl MeasuredTableCell {
    /// Maximum size of an encoded record, in bytes.
    pub const MAX_RECORD_BYTES: usize = MAX_RECORD;

    /// Decode a record for the given cell text. For eligible text, the callback
    /// receives the exact record-sized buffer and returns whether it filled it.
    pub fn from_record(text: &str, read: impl FnOnce(&mut [u8]) -> bool) -> Option<Self> {
        if !eligible_text(text) {
            return None;
        }
        let mut bytes = [0; MAX_RECORD];
        let record = &mut bytes[..record_len(text)];
        if !read(record) {
            return None;
        }
        decode(record, text)
    }

    /// Replace a sample's text and glyphs when the face, metrics, cell text and
    /// available width match. The caller supplies the sample for the corresponding
    /// column, verifies the cell geometry, and assigns the target origin, line and
    /// source. The caller must verify equivalent paint styles and coordinate
    /// systems for both cells. Color, shadows and transform belong to the
    /// sample. Stored advances include the measured cell's glyph spacing.
    /// Samples with page-positioned decoration segments are declined.
    /// After replacement the sample borrows the caller's text and owns its glyphs.
    /// A declined replacement leaves the sample intact.
    pub fn apply<'a>(
        &self,
        sample: &mut PositionedGlyphRun<'a>,
        text: &'a str,
        available_width: f32,
    ) -> bool {
        if text != self.text.as_ref()
            || sample.font.id != self.face
            || sample.font_size != self.size
            || sample.ascent != self.ascent
            || sample.descent != self.descent
            || sample.text_offset != 0
            || sample.letter_box.is_some()
            || !sample.decorations.is_empty()
            || !sample.variations.is_empty()
            || !sample.normalized_coords.is_empty()
            || sample.synthesis != Default::default()
            || !available_width.is_finite()
            || self.advance > available_width
        {
            return false;
        }
        sample.text = text;
        sample.advance = self.advance;
        sample.glyphs.clone_from(&self.glyphs);
        true
    }
}

#[cfg(test)]
mod tests;
