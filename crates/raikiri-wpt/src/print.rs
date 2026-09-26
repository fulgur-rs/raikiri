//! HTTP-backed paginated rendering for upstream print reftests.

use std::fmt;

use raikiri::{Body, Method, NetworkProvider, ParseOptions, Request, ResourceKind, Url};
use raikiri_html::effective_document_base_url;
use raikiri_net::{ImageResolver, SystemHttpProvider};

use crate::http_resources::{NetworkFontLoader, StylesheetUrlProvider, prepare_cascade_images};
use crate::reftest::{PrintRenderResources, RenderedDocument, render_raikiri_pages_with_resources};

/// Fetches and renders one HTTP document into ordered print pages.
pub fn render_print_url(
    provider: &SystemHttpProvider,
    url: Url,
    width: u32,
    height: u32,
) -> Result<RenderedDocument, PrintRenderError> {
    if width == 0 || height == 0 {
        return Err(PrintRenderError::new(
            "fallback page dimensions must be positive".into(),
        ));
    }
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
        .map_err(|error| PrintRenderError::new(format!("document fetch failed: {error:?}")))?;

    let fallback_base_url = resource.final_url;
    let probe_options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: Some(fallback_base_url.clone()),
    };
    let base_probe = raikiri::parse(resource.bytes.as_ref(), &probe_options).map_err(|error| {
        PrintRenderError::new(format!(
            "HTML base URL parse failed for {fallback_base_url}: {error:?}"
        ))
    })?;
    let base_url = effective_document_base_url(&base_probe, Some(&fallback_base_url))
        .unwrap_or_else(|| fallback_base_url.clone());
    let html = String::from_utf8_lossy(resource.bytes.as_ref());
    let stylesheet_provider = StylesheetUrlProvider { provider };
    let image_resolver = ImageResolver::new(provider.clone());
    let font_loader = NetworkFontLoader {
        provider,
        base_url: &base_url,
    };
    let prepare_images = |cascade: &mut raikiri_style::CascadeResult| {
        prepare_cascade_images(cascade, &base_url, &image_resolver);
    };

    render_raikiri_pages_with_resources(
        &html,
        width,
        height,
        PrintRenderResources {
            network: Some(&stylesheet_provider),
            parse_base_url: Some(&fallback_base_url),
            base_url: Some(&base_url),
            replaced_resolver: Some(&image_resolver),
            image_pixel_source: Some(&image_resolver),
            font_loader: Some(&font_loader),
            prepare_cascade_images: Some(&prepare_images),
        },
    )
    .map_err(|error| PrintRenderError::new(error.to_string()))
}

/// Failure while fetching, parsing, laying out, or painting a print document.
#[derive(Debug)]
pub struct PrintRenderError(String);

impl PrintRenderError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for PrintRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PrintRenderError {}

#[cfg(test)]
mod tests;
