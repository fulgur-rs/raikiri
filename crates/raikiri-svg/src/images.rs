//! Pictures for SVG `<image>` elements.
//!
//! This crate decodes no raster format and opens no file or connection. The
//! parser asks an [`SvgImageSource`] for the picture an `<image>` names, and
//! the source is the page: it resolves the URL against the page's base, runs
//! it through the network provider and the resource policy, and decodes the
//! response under the page's byte and pixel limits. The scene then carries
//! the decoded pixels, so a document's pictures are fetched and budgeted by
//! the same code as the pictures of the HTML around it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use raikiri_traits::DecodedImage;

/// Most distinct pictures one scene may ask its source for, each `data:` URL
/// and each URL counting once however often the document repeats it. A
/// document that asks for more is an error, not a picture with holes.
pub(crate) const MAX_SCENE_IMAGES: usize = 256;

/// Which `<image>` references a document may carry.
///
/// A reference the access does not cover is rejected when the document is
/// parsed, so a source is never asked for anything the host did not allow.
/// Fragment references (`#id`) name no resource and are always allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub enum SvgImageAccess {
    /// Nothing but fragment references. [`SvgDocument::parse`] uses this.
    ///
    /// [`SvgDocument::parse`]: crate::SvgDocument::parse
    None,
    /// Also `data:` URLs, which hold their bytes in themselves. This is an SVG
    /// used as an image, which loads nothing by itself.
    Data,
    /// Also any URL, which the source fetches. This is an inline `<svg>` of a
    /// page. A document with an `xml:base` is rejected, because the source
    /// resolves every URL against the page and not against that base.
    Any,
}

/// Where SVG `<image>` elements get their pictures.
///
/// An implementation answers from the page's resources: it checks the URL
/// against the resource policy, fetches it through the network provider and
/// decodes it under the page's limits, and answers `None` for anything it
/// denied, could not fetch or could not decode as a raster picture. An
/// `<image>` that gets `None` draws nothing, like a broken picture in HTML.
///
/// A picture that is itself an SVG document is not a raster picture: the
/// scene draws no nested SVG, so a source answers `None` for one, and an SVG
/// can never lead back to itself.
///
/// A source should hand out the same [`Arc`] every time it answers with the
/// same picture, so the scene holds it once.
pub trait SvgImageSource: Send + Sync {
    /// The picture that `href` names, exactly as the document wrote it, so
    /// the source resolves it. Fragment references and empty references never
    /// reach the source.
    fn url(&self, href: &str) -> Option<Arc<DecodedImage>>;

    /// The picture inside a `data:` URL, which the parser has already
    /// decoded: its MIME type and its bytes. A `data:` URL of an SVG document
    /// never reaches the source.
    fn data(&self, mime: &str, bytes: &[u8]) -> Option<Arc<DecodedImage>>;
}

/// The pictures a parse has asked for.
///
/// The parser knows a picture by the bytes of a raster file, which it only
/// measures. Every picture the source hands out gets 24 bytes that say how
/// big it is, in the shape of the start of a PNG file, and the parser's node
/// keeps those. The map recognizes them by identity and gives back the pixels.
///
/// The pixels of the pictures held are bounded while the parser asks, not
/// after it: a document of many pictures never holds more than the bound, and
/// stops asking once it has gone over.
pub(crate) struct Images<'a> {
    source: &'a dyn SvgImageSource,
    /// Most pixel bytes the distinct pictures may hold together.
    max_bytes: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Everything the source was asked for, in order, so that the parser's
    /// repeated references (`use` instances) cost one question.
    asked: Vec<Asked>,
    pictures: Vec<Picture>,
    /// The pixel bytes of `pictures`.
    bytes: usize,
    overflowed: bool,
    bytes_exceeded: bool,
}

struct Asked {
    key: Key,
    picture: Option<usize>,
}

enum Key {
    Url(String),
    Data {
        mime: String,
        hash: u64,
        bytes: Arc<Vec<u8>>,
    },
}

impl Key {
    fn data(mime: &str, bytes: Arc<Vec<u8>>) -> Self {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self::Data {
            mime: mime.to_owned(),
            hash: hasher.finish(),
            bytes,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(a), Self::Url(b)) => a == b,
            (
                Self::Data { mime, hash, bytes },
                Self::Data {
                    mime: other_mime,
                    hash: other_hash,
                    bytes: other_bytes,
                },
            ) => {
                mime == other_mime
                    && hash == other_hash
                    && (Arc::ptr_eq(bytes, other_bytes) || bytes == other_bytes)
            }
            _ => false,
        }
    }
}

#[derive(Clone)]
struct Picture {
    /// What the parser's node keeps in place of the file.
    header: Arc<Vec<u8>>,
    image: Arc<DecodedImage>,
}

/// The pictures behind the nodes of a parsed tree.
#[derive(Default)]
pub(crate) struct ImageMap {
    pictures: Vec<Picture>,
    overflowed: bool,
    bytes_exceeded: bool,
}

impl ImageMap {
    /// The pixels of the picture `header` stands for.
    pub(crate) fn get(&self, header: &Arc<Vec<u8>>) -> Option<&Arc<DecodedImage>> {
        self.pictures
            .iter()
            .find(|picture| Arc::ptr_eq(&picture.header, header))
            .map(|picture| &picture.image)
    }

    /// Whether the document asked for more pictures than a scene may hold.
    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Whether the pictures of the document hold more pixels than a scene
    /// may carry.
    pub(crate) fn bytes_exceeded(&self) -> bool {
        self.bytes_exceeded
    }
}

impl<'a> Images<'a> {
    pub(crate) fn new(source: &'a dyn SvgImageSource, max_bytes: usize) -> Self {
        Self {
            source,
            max_bytes,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic in a source leaves the lists consistent: entries are only
        // ever appended whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The parser's image resolver: it asks the source for each reference.
    pub(crate) fn resolver(&self) -> usvg::ImageHrefResolver<'_> {
        usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime, data, _| {
                // Raster pictures only: a nested SVG document stays out.
                if mime.eq_ignore_ascii_case("image/svg+xml") {
                    return None;
                }
                let bytes = Arc::clone(&data);
                self.resolve(Key::data(mime, data), || self.source.data(mime, &bytes))
            }),
            resolve_string: Box::new(move |href, _| {
                let href = href.trim();
                if href.is_empty() || href.starts_with('#') {
                    return None;
                }
                self.resolve(Key::Url(href.to_owned()), || self.source.url(href))
            }),
        }
    }

    /// The parser's picture for `key`, asking the source the first time.
    fn resolve(
        &self,
        key: Key,
        ask: impl FnOnce() -> Option<Arc<DecodedImage>>,
    ) -> Option<usvg::ImageKind> {
        let mut state = self.state();
        let picture = match state.asked.iter().find(|asked| asked.key.matches(&key)) {
            Some(asked) => asked.picture,
            // The scene is an error already: asking on would only fetch and
            // decode what nobody draws.
            None if state.overflowed || state.bytes_exceeded => return None,
            None if state.asked.len() >= MAX_SCENE_IMAGES => {
                state.overflowed = true;
                return None;
            }
            None => {
                let max_bytes = self.max_bytes;
                let picture = ask()
                    .filter(|image| is_well_formed(image))
                    .and_then(|image| state.add(image, max_bytes));
                state.asked.push(Asked { key, picture });
                picture
            }
        };
        picture.map(|index| usvg::ImageKind::PNG(Arc::clone(&state.pictures[index].header)))
    }

    /// The pictures that were handed out.
    pub(crate) fn into_map(self) -> ImageMap {
        let state = self
            .state
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        ImageMap {
            pictures: state.pictures,
            overflowed: state.overflowed,
            bytes_exceeded: state.bytes_exceeded,
        }
    }
}

impl State {
    /// The index of `image`, adding it when this is the first time the source
    /// hands it out. A picture that would take the pictures over `max_bytes`
    /// is dropped, and the scene fails.
    fn add(&mut self, image: Arc<DecodedImage>, max_bytes: usize) -> Option<usize> {
        if let Some(index) = self
            .pictures
            .iter()
            .position(|picture| Arc::ptr_eq(&picture.image, &image))
        {
            return Some(index);
        }
        let bytes = self.bytes.saturating_add(image.rgba.len());
        if bytes > max_bytes {
            self.bytes_exceeded = true;
            return None;
        }
        self.bytes = bytes;
        self.pictures.push(Picture {
            header: header(image.width, image.height),
            image,
        });
        Some(self.pictures.len() - 1)
    }
}

/// Whether `image` has pixels for the size it states. A source that hands out
/// anything else is wrong, and the picture is left out rather than drawn.
fn is_well_formed(image: &DecodedImage) -> bool {
    u64::from(image.width) * u64::from(image.height) * 4 == image.rgba.len() as u64
        && !image.rgba.is_empty()
}

/// The start of a PNG file for a picture of `width` × `height` pixels: its
/// signature and the header chunk, which is all that the parser measures.
fn header(width: u32, height: u32) -> Arc<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    Arc::new(bytes)
}
