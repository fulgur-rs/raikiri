//! Shared HTTP document resource preparation for screen and print rendering.

use raikiri::{
    Body, FetchOutcome, Method, NetworkError, NetworkProvider, Request, ResourceKind, Url,
};
use raikiri_net::{ImageResolver, SystemHttpProvider};
use raikiri_style::property::BackgroundImage;
use raikiri_traits::{ReplacedResolver, ResolverRequest};

mod css_urls;

pub(crate) struct StylesheetUrlProvider<'a> {
    pub(crate) provider: &'a SystemHttpProvider,
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

pub(crate) struct NetworkFontLoader<'a> {
    pub(crate) provider: &'a SystemHttpProvider,
    pub(crate) base_url: &'a Url,
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

pub(crate) fn absolutize_img_sources(document: &mut raikiri_dom::Document, base_url: &Url) {
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

pub(crate) fn prepare_cascade_images(
    cascade: &mut raikiri_style::CascadeResult,
    base_url: &Url,
    resolver: &ImageResolver<SystemHttpProvider>,
) {
    for computed in &mut cascade.computed {
        prepare_background_image(&mut computed.background_image, base_url, resolver);
        prepare_background_image(&mut computed.list_style_image, base_url, resolver);
    }
    cascade
        .page
        .for_each_background_image_mut(|image| prepare_background_image(image, base_url, resolver));
}
