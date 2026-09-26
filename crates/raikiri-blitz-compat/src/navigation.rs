//! blitz_traits::navigation compatible shape.

use crate::net::{Body, Request};
use http::Method;
use url::Url;

pub trait NavigationProvider: Send + Sync + 'static {
    fn navigate_to(&self, options: NavigationOptions);
}

pub struct DummyNavigationProvider;

impl NavigationProvider for DummyNavigationProvider {
    fn navigate_to(&self, _options: NavigationOptions) {}
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct NavigationOptions {
    pub url: Url,
    pub content_type: Option<String>,
    pub source_document: usize,
    pub method: Method,
    pub document_resource: Body,
}

impl NavigationOptions {
    pub fn new(url: Url, content_type: Option<String>, source_document: usize) -> Self {
        Self {
            url,
            content_type,
            source_document,
            method: Method::GET,
            document_resource: Body::Empty,
        }
    }
    pub fn set_document_resource(mut self, document_resource: Body) -> Self {
        self.document_resource = document_resource;
        self
    }
    pub fn set_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }
    pub fn into_request(self) -> Request {
        Request {
            url: self.url,
            method: self.method,
            content_type: self.content_type,
            headers: http::HeaderMap::new(),
            body: self.document_resource,
            signal: None,
        }
    }
}

#[cfg(test)]
mod tests;
