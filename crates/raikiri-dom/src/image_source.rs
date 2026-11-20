//! The source an `<img>` shows: the image candidate chosen from its `srcset`,
//! from a `<source>` of the `<picture>` around it, or its `src`.
//!
//! Spec: <https://html.spec.whatwg.org/multipage/images.html#update-the-source-set>
//! and <https://html.spec.whatwg.org/multipage/images.html#select-an-image-source>.

use raikiri_style::MediaContext;

use crate::document::Document;

/// The image an `<img>` shows and the pixel density it was written for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageSource<'a> {
    /// The URL as written, not yet resolved against a base.
    pub(crate) url: &'a str,
    /// Image pixels per CSS px: the natural size is the image's size divided
    /// by it.
    pub(crate) density: f32,
}

/// One entry of a `srcset` attribute.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Candidate<'a> {
    url: &'a str,
    descriptor: Descriptor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Descriptor {
    /// No descriptor: the same as `1x`.
    None,
    /// `<number>x`
    Density(f32),
    /// `<integer>w`, to be divided by the source size.
    Width(u32),
}

/// The source that the `<img>` element `image` shows in `context`.
///
/// A `<source>` of the `<picture>` that is the image's parent comes first:
/// the first one before the image with candidates, whose `media` matches and
/// whose `type` is one this renderer can show, gives the image its candidates
/// and the image's own attributes are not read. Otherwise the candidates come
/// from the image's `srcset`, with `src` added as `1x` unless a candidate
/// already is `1x` or has a width.
///
/// Of the candidates, the one with the smallest density that is at least the
/// output's one pixel per CSS px is chosen, or the densest when none reaches
/// it, as in Chromium. The first of two with the same density wins. An image
/// with no candidate shows its `src` as written, even an empty one, and one
/// without `src` shows nothing.
pub(crate) fn image_source<'a>(
    document: &'a Document,
    image: usize,
    context: &MediaContext,
) -> Option<ImageSource<'a>> {
    let node = document.get_node(image)?;
    if let Some(candidates) = picture_source_candidates(document, image, context) {
        return choose(candidates);
    }
    let src = node.attribute("src");
    let mut candidates = node
        .attribute("srcset")
        .map(|set| sized_candidates(parse_srcset(set), node.attribute("sizes"), context))
        .unwrap_or_default();
    if let Some(src) = src.filter(|src| !src.is_empty())
        && !candidates.iter().any(|(candidate, density)| {
            *density == 1.0 || matches!(candidate.descriptor, Descriptor::Width(_))
        })
    {
        candidates.push((
            Candidate {
                url: src,
                descriptor: Descriptor::None,
            },
            1.0,
        ));
    }
    choose(candidates).or_else(|| src.map(|url| ImageSource { url, density: 1.0 }))
}

/// The candidates of the first `<source>` before `image` in the `<picture>`
/// that is its parent which applies in `context`, or `None` when the image is
/// not in a picture or no source applies.
fn picture_source_candidates<'a>(
    document: &'a Document,
    image: usize,
    context: &MediaContext,
) -> Option<Vec<(Candidate<'a>, f32)>> {
    let picture = document.parent_of(image)?;
    if document.nodes[picture].tag_name() != Some("picture") {
        return None;
    }
    let children = &document.nodes[picture].children;
    let before = &children[..children.iter().position(|&child| child == image)?];
    before.iter().find_map(|&child| {
        let source = &document.nodes[child];
        if source.tag_name() != Some("source") {
            return None;
        }
        let candidates = parse_srcset(source.attribute("srcset")?);
        let applies = !candidates.is_empty()
            && source
                .attribute("media")
                .is_none_or(|media| raikiri_style::media_list_matches(media, context))
            && source.attribute("type").is_none_or(is_supported_image_type);
        applies.then(|| sized_candidates(candidates, source.attribute("sizes"), context))
    })
}

/// Whether the MIME type `value` names an image format this renderer can
/// show. An empty type does not exclude the source, as in Chromium.
fn is_supported_image_type(value: &str) -> bool {
    let essence = value.split(';').next().unwrap_or_default().trim_ascii();
    essence.is_empty()
        || [
            "image/apng",
            "image/avif",
            "image/bmp",
            "image/gif",
            "image/jpeg",
            "image/jpg",
            "image/pjpeg",
            "image/png",
            "image/svg+xml",
            "image/vnd.microsoft.icon",
            "image/webp",
            "image/x-icon",
            "image/x-png",
        ]
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(essence))
}

/// The candidates with their densities: a width descriptor is divided by the
/// size that `sizes` gives the image.
fn sized_candidates<'a>(
    candidates: Vec<Candidate<'a>>,
    sizes: Option<&str>,
    context: &MediaContext,
) -> Vec<(Candidate<'a>, f32)> {
    let mut source_size = None;
    candidates
        .into_iter()
        .map(|candidate| {
            let density = match candidate.descriptor {
                Descriptor::None => 1.0,
                Descriptor::Density(density) => density,
                Descriptor::Width(width) => {
                    let size = *source_size.get_or_insert_with(|| {
                        raikiri_style::source_size(sizes.unwrap_or_default(), context)
                    });
                    width as f32 / size
                }
            };
            (candidate, density)
        })
        .collect()
}

/// The candidate shown on an output with one device pixel per CSS px. A
/// density of zero is never chosen; an infinite one, from a width divided by
/// a source size of zero, gives the image no size, as in Chromium.
fn choose(candidates: Vec<(Candidate<'_>, f32)>) -> Option<ImageSource<'_>> {
    let usable = || candidates.iter().filter(|(_, density)| *density > 0.0);
    let fits = usable()
        .filter(|(_, density)| *density >= 1.0)
        .reduce(|best, next| if next.1 < best.1 { next } else { best });
    let (candidate, density) =
        fits.or_else(|| usable().reduce(|best, next| if next.1 > best.1 { next } else { best }))?;
    Some(ImageSource {
        url: candidate.url,
        density: *density,
    })
}

/// The candidates of a `srcset` attribute, without those whose descriptors
/// are invalid.
///
/// Spec: <https://html.spec.whatwg.org/multipage/images.html#parse-a-srcset-attribute>
fn parse_srcset(input: &str) -> Vec<Candidate<'_>> {
    let mut candidates = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            return candidates;
        }
        let url_end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let (url, after) = rest.split_at(url_end);
        // A URL that ends in commas ends its candidate: the commas are not
        // part of it, and it has no descriptors.
        let (url, descriptors) = if url.ends_with(',') {
            rest = after;
            (url.trim_end_matches(','), Vec::new())
        } else {
            let descriptors;
            (descriptors, rest) = tokenize_descriptors(after);
            (url, descriptors)
        };
        if let Some(candidate) = candidate(url, &descriptors) {
            candidates.push(candidate);
        }
    }
}

/// The descriptors after a candidate's URL, up to the comma that ends the
/// candidate, and the input after that comma.
fn tokenize_descriptors(input: &str) -> (Vec<&str>, &str) {
    let input = input.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let mut descriptors = Vec::new();
    let mut start = None;
    let mut in_parens = false;
    fn push<'a>(input: &'a str, start: &mut Option<usize>, end: usize, out: &mut Vec<&'a str>) {
        if let Some(begin) = start.take() {
            out.push(&input[begin..end]);
        }
    }
    for (index, c) in input.char_indices() {
        if in_parens {
            in_parens = c != ')';
        } else if c.is_ascii_whitespace() {
            push(input, &mut start, index, &mut descriptors);
        } else if c == ',' {
            push(input, &mut start, index, &mut descriptors);
            return (descriptors, &input[index + 1..]);
        } else {
            in_parens = c == '(';
            start.get_or_insert(index);
        }
    }
    push(input, &mut start, input.len(), &mut descriptors);
    (descriptors, "")
}

/// The candidate for `url` with `descriptors`, or `None` when they are
/// invalid: an unknown descriptor, two of one kind, a density with a width or
/// a height, a height without a width, or a value out of range.
fn candidate<'a>(url: &'a str, descriptors: &[&str]) -> Option<Candidate<'a>> {
    let mut width = None;
    let mut density = None;
    let mut height = None;
    for descriptor in descriptors {
        let (value, unit) =
            descriptor.split_at(descriptor.len() - descriptor.chars().last()?.len_utf8());
        match unit {
            "w" if width.is_none() && density.is_none() => {
                width = Some(parse_positive_integer(value)?);
            }
            "x" if width.is_none() && density.is_none() && height.is_none() => {
                density = Some(parse_float(value).filter(|density| *density >= 0.0)?);
            }
            "h" if height.is_none() && density.is_none() => {
                height = Some(parse_positive_integer(value)?);
            }
            _ => return None,
        }
    }
    if height.is_some() && width.is_none() {
        return None;
    }
    let descriptor = match (width, density) {
        (Some(width), _) => Descriptor::Width(width),
        (None, Some(density)) => Descriptor::Density(density),
        (None, None) => Descriptor::None,
    };
    Some(Candidate { url, descriptor })
}

/// A valid non-negative integer greater than zero: ASCII digits only.
fn parse_positive_integer(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|value| *value > 0)
}

/// A valid floating-point number: an optional `-`, digits with an optional
/// fraction (or a fraction alone), and an optional exponent.
///
/// Spec: <https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#valid-floating-point-number>
fn parse_float(value: &str) -> Option<f32> {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (&unsigned[..index], Some(&unsigned[index + 1..])),
        None => (unsigned, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let mantissa_valid = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole.is_empty() || digits(whole)) && digits(fraction),
        None => digits(mantissa),
    };
    let exponent_valid = exponent
        .is_none_or(|exponent| digits(exponent.strip_prefix(['-', '+']).unwrap_or(exponent)));
    if !(mantissa_valid && exponent_valid) {
        return None;
    }
    value.parse::<f32>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests;
