//! Explicit document syntax at file and HTTP entry points.

use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DocumentSyntax {
    #[default]
    Html,
    Xhtml,
}

impl DocumentSyntax {
    pub(crate) fn for_path(path: &Path) -> Self {
        // Limit automatic XML parsing to the runner's XHTML discovery extension.
        // Direct .xht/.xml fixtures retain their legacy HTML-compatible path until
        // XHTML DTD catalogs and their named character references are supported.
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("xhtml") => Self::Xhtml,
            _ => Self::Html,
        }
    }

    pub(crate) fn for_content_type(content_type: Option<&str>) -> Self {
        match content_type
            .and_then(|value| value.split(';').next())
            .map(str::trim)
        {
            Some(value) if value.eq_ignore_ascii_case("application/xhtml+xml") => Self::Xhtml,
            _ => Self::Html,
        }
    }

    pub(crate) fn parse(
        self,
        source: &[u8],
        options: &raikiri_html::ParseOptions<'_>,
    ) -> Result<raikiri_html::UncascadedDocument, String> {
        match self {
            Self::Html => raikiri_html::parse(source, options).map_err(|error| error.to_string()),
            Self::Xhtml => {
                raikiri_html::parse_xhtml(source, options).map_err(|error| error.to_string())
            }
        }
    }
}
