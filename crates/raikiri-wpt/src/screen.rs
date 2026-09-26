//! HTTP-backed single-viewport screen rendering for upstream wptrunner.

use std::fmt;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use raikiri::{
    Body, FetchOutcome, MediaContext, Method, NetworkError, NetworkProvider, PageBox,
    PageContextQuery, ParseOptions, Request, ResourceKind, Url,
    build_cascaded_with_media_context_for_page,
};
use raikiri_html::effective_document_base_url;
use raikiri_net::{ImageResolver, SystemHttpProvider};
use raikiri_style::property::BackgroundImage;
use raikiri_traits::{ReplacedResolver, ResolverRequest};

use crate::reftest::{RenderedImage, resolve_font_ctx};

mod css_urls;

struct StylesheetUrlProvider<'a> {
    provider: &'a SystemHttpProvider,
}

impl NetworkProvider for StylesheetUrlProvider<'_> {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let rewrite_urls = matches!(
            request.kind,
            ResourceKind::ExternalStylesheet | ResourceKind::StylesheetImport
        );
        let outcome = self.provider.fetch_one_hop(request)?;
        match outcome {
            FetchOutcome::Body(mut resource) if rewrite_urls => {
                let source = String::from_utf8_lossy(&resource.bytes);
                let rewritten = css_urls::absolutize_stylesheet_urls(&source, &resource.final_url);
                resource.bytes = rewritten.into();
                Ok(FetchOutcome::Body(resource))
            }
            other => Ok(other),
        }
    }

    fn max_import_depth(&self) -> Option<u32> {
        self.provider.max_import_depth()
    }
}

struct NetworkFontLoader<'a> {
    provider: &'a SystemHttpProvider,
    base_url: &'a Url,
}

impl raikiri_dom::FontFaceLoader for NetworkFontLoader<'_> {
    fn load(&self, source: &str) -> Option<Vec<u8>> {
        let url = Url::parse(source)
            .ok()
            .or_else(|| self.base_url.join(source).ok())?;
        self.provider
            .fetch(Request {
                url,
                method: Method::Get,
                content_type: None,
                headers: Vec::new(),
                body: Body::Empty,
                signal: None,
                kind: ResourceKind::Font,
            })
            .ok()
            .map(|resource| resource.bytes.to_vec())
    }
}

fn absolutize_img_sources(document: &mut raikiri_dom::Document, base_url: &Url) {
    let updates: Vec<_> = (0..document.node_count())
        .filter_map(|node_id| {
            let node = document.get_node(node_id)?;
            if node.tag_name() != Some("img") {
                return None;
            }
            let source = document.element_attribute(node_id, "src")?;
            if Url::parse(source).is_ok() {
                return None;
            }
            base_url
                .join(source)
                .ok()
                .map(|absolute| (node_id, absolute.to_string()))
        })
        .collect();
    for (node_id, source) in updates {
        document
            .set_element_attribute(node_id, "src", source)
            .expect("src is a valid HTML attribute name");
    }
}

fn prepare_background_image(
    image: &mut BackgroundImage,
    base_url: &Url,
    resolver: &ImageResolver<SystemHttpProvider>,
) {
    let BackgroundImage::Url(source) = image else {
        return;
    };
    let Some(absolute) = Url::parse(source)
        .ok()
        .or_else(|| base_url.join(source).ok())
    else {
        return;
    };
    *source = absolute.to_string();
    let _ = resolver.resolve(ResolverRequest::new(&absolute));
}

fn prepare_cascade_images(
    cascade: &mut raikiri_style::CascadeResult,
    base_url: &Url,
    resolver: &ImageResolver<SystemHttpProvider>,
) {
    for computed in &mut cascade.computed {
        prepare_background_image(&mut computed.background_image, base_url, resolver);
        prepare_background_image(&mut computed.list_style_image, base_url, resolver);
    }
}

/// Fetches and renders one HTTP document at the requested screen viewport.
pub fn render_screen_url(
    provider: &SystemHttpProvider,
    url: Url,
    width: u32,
    height: u32,
) -> Result<RenderedImage, ScreenRenderError> {
    let resource = provider
        .fetch(Request {
            url,
            method: Method::Get,
            content_type: None,
            headers: Vec::new(),
            body: Body::Empty,
            signal: None,
            kind: ResourceKind::Other,
        })
        .map_err(|error| ScreenRenderError::new(format!("document fetch failed: {error:?}")))?;

    let fallback_base_url = resource.final_url;
    let encoding = resource.encoding.as_deref().unwrap_or("unspecified");
    let stylesheet_provider = StylesheetUrlProvider { provider };
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&stylesheet_provider),
        base_url: Some(fallback_base_url.clone()),
    };
    let mut uncascaded = raikiri::parse(resource.bytes.as_ref(), &options).map_err(|error| {
        ScreenRenderError::new(format!(
            "HTML parse failed for {fallback_base_url} (declared encoding: {encoding}): {error:?}"
        ))
    })?;
    let base_url = effective_document_base_url(&uncascaded, Some(&fallback_base_url))
        .unwrap_or(fallback_base_url);
    absolutize_img_sources(&mut uncascaded.dom, &base_url);

    let media_context = MediaContext::screen();
    let page_query = PageContextQuery::default();
    let font_face_tree = raikiri::build_rule_tree(&uncascaded);
    let mut font_context = resolve_font_ctx();
    raikiri_dom::register_font_face_sources(
        &mut font_context,
        font_face_tree.font_faces(),
        &NetworkFontLoader {
            provider,
            base_url: &base_url,
        },
    );
    let mut cascade =
        build_cascaded_with_media_context_for_page(&uncascaded, &media_context, &page_query);
    raikiri_dom::expand_font_face_aliases(
        &mut cascade.computed,
        font_face_tree.font_faces(),
        &mut font_context,
    );
    let mut page_box = PageBox::new();
    page_box.width = width as f32;
    page_box.height = height as f32;
    let image_resolver = ImageResolver::new(provider.clone());
    prepare_cascade_images(&mut cascade, &base_url, &image_resolver);
    raikiri_dom::layout_single_page_with_resolver_and_base_url(
        &mut uncascaded.dom,
        &cascade,
        page_box,
        font_context,
        &image_resolver,
        Some(&base_url),
    )
    .map_err(|error| ScreenRenderError::new(format!("layout failed: {error:?}")))?;

    let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
        |painter| {
            raikiri_paint::paint_single_page_with_images(
                painter,
                &uncascaded.dom,
                &cascade,
                page_box,
                &image_resolver,
            );
        },
        width,
        height,
    );
    Ok(RenderedImage {
        width,
        height,
        rgba,
    })
}

/// Failure while fetching, parsing, laying out, or painting a screen document.
#[derive(Debug)]
pub struct ScreenRenderError(String);

impl ScreenRenderError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for ScreenRenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScreenRenderError {}

#[cfg(test)]
mod tests;
