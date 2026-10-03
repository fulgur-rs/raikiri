//! HTTP-backed single-viewport screen rendering for upstream wptrunner.

use std::fmt;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use raikiri::{
    Body, MediaContext, Method, NetworkProvider, PageBox, PageContextQuery, ParseOptions,
    RasterBufferBudget, Request, ResourceKind, Url, build_cascaded_with_media_context_for_page,
};
use raikiri_html::effective_document_base_url;
use raikiri_net::{ImageResolver, SystemHttpProvider};

use crate::http_resources::{
    NetworkFontLoader, StylesheetUrlProvider, absolutize_img_sources, prepare_cascade_images,
};
use crate::reftest::{RenderedImage, wpt_document_fonts};

/// Fetches and renders one HTTP document at the requested screen viewport.
pub fn render_screen_url(
    provider: &SystemHttpProvider,
    url: Url,
    width: u32,
    height: u32,
) -> Result<RenderedImage, ScreenRenderError> {
    let size = RasterBufferBudget::new()
        .reserve_pixels(width, height)
        .map_err(ScreenRenderError::from_raster_error)?;
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
    let (fonts, bundled_only) = wpt_document_fonts(
        font_face_tree.font_faces(),
        Some(&NetworkFontLoader {
            provider,
            base_url: &base_url,
        }),
        false,
    )
    .map_err(ScreenRenderError::new)?;
    uncascaded.dom.set_font_collection(fonts);
    uncascaded.dom.set_ifc_parallel_build(bundled_only);
    let mut cascade =
        build_cascaded_with_media_context_for_page(&uncascaded, &media_context, &page_query);
    let mut page_box = PageBox::new();
    page_box.width = size.width() as f32;
    page_box.height = size.height() as f32;
    let image_resolver = ImageResolver::new(provider.clone());
    prepare_cascade_images(&mut cascade, &base_url, &image_resolver);
    raikiri_dom::layout_single_page_with_resolver_and_base_url(
        &mut uncascaded.dom,
        &cascade,
        page_box,
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
        size.width(),
        size.height(),
    );
    Ok(RenderedImage {
        width: size.width(),
        height: size.height(),
        rgba,
    })
}

/// Failure while fetching, parsing, laying out, or painting a screen document.
#[derive(Debug)]
pub struct ScreenRenderError {
    message: String,
    raster_error: Option<raikiri::RenderError>,
}

impl ScreenRenderError {
    fn new(message: String) -> Self {
        Self {
            message,
            raster_error: None,
        }
    }

    fn from_raster_error(error: raikiri::RenderError) -> Self {
        Self {
            message: error.to_string(),
            raster_error: Some(error),
        }
    }
}

impl fmt::Display for ScreenRenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScreenRenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.raster_error
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests;
