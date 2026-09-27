use super::*;

fn waiting(document: &raikiri_dom::Document) -> bool {
    let mut stack = vec![document.root_index()];
    while let Some(id) = stack.pop() {
        let Some(node) = document.get_node(id) else {
            continue;
        };
        if node.tag_name() == Some("html") {
            return node.attribute("class").is_some_and(|classes| {
                classes
                    .split_ascii_whitespace()
                    .any(|class| class == "reftest-wait")
            });
        }
        stack.extend(node.children.iter().rev().copied());
    }
    false
}

fn serialize(
    document: &raikiri_dom::Document,
    report: &raikiri_js::runtime::RunReport,
) -> Result<String, ReftestError> {
    if report.aborted.is_some()
        || !report.host_failures.is_empty()
        || !report.fetch_errors.is_empty()
        || !report.uncaught_errors.is_empty()
    {
        return Err(ReftestError::RaikiriRender(format!(
            "dynamic reftest script failed: {report:?}"
        )));
    }
    if waiting(document) {
        return Err(ReftestError::RaikiriRender(
            "dynamic reftest still has reftest-wait after event loop drained".into(),
        ));
    }
    let markup = document
        .serialize_inner_html(document.root_index())
        .map_err(ReftestError::RaikiriRender)?;
    let doctype = match document.quirks_mode() {
        raikiri_dom::QuirksMode::NoQuirks => "<!DOCTYPE html>",
        raikiri_dom::QuirksMode::LimitedQuirks => {
            "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\" \"http://www.w3.org/TR/html4/loose.dtd\">"
        }
        raikiri_dom::QuirksMode::Quirks => "",
        _ => {
            return Err(ReftestError::RaikiriRender(
                "unsupported dynamic document mode".into(),
            ));
        }
    };
    Ok(format!("{doctype}{markup}"))
}

pub(super) fn prepare(
    html: &str,
    path: &Path,
    suffix: &str,
    config: ReftestConfig,
) -> Result<String, ReftestError> {
    let parsed = raikiri_html::parse(
        html.as_bytes(),
        &raikiri::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .map_err(|e| ReftestError::RaikiriRender(format!("parse readiness: {e:?}")))?;
    if !waiting(&parsed.dom) {
        return Ok(html.to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| ReftestError::RaikiriRender("dynamic reftest has no directory".into()))?;
    let wpt_root = parent
        .ancestors()
        .find(|p| p.join("resources/testharness.js").is_file())
        .unwrap_or(parent);
    let setup = prepare_wpt_live_document(html, config.width, config.height, parent, wpt_root)
        .map_err(ReftestError::RaikiriRender)?;
    let url = raikiri::Url::from_file_path(path).map_err(|_| {
        ReftestError::RaikiriRender("dynamic reftest needs an absolute path".into())
    })?;
    let url = url
        .join(suffix)
        .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
    let host = crate::wpt_host::WptDocumentHost::new(setup, wpt_root).with_page_url(url);
    // spec: https://web-platform-tests.org/writing-tests/reftests.html#controlling-when-the-screenshot-is-taken
    let preamble = "window.addEventListener('load', function() { requestAnimationFrame(function() { requestAnimationFrame(function() { if (document.documentElement.classList.contains('reftest-wait')) document.documentElement.dispatchEvent(new Event('TestRendered', {bubbles:true})); }); }); });";
    #[cfg(feature = "js-native")]
    {
        let mut runtime = raikiri_js::runtime::DomRuntime::new(host)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        raikiri_js_wasmtime_harness::install_page_support(&mut runtime)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        runtime
            .evaluate(preamble)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        let report = runtime.run_document();
        let host = runtime.into_host();
        serialize(host.document(), &report)
    }
    #[cfg(feature = "js-wasmtime")]
    {
        let mut runtime = raikiri_js_wasmtime_host::WasmtimePage::new(
            Box::new(host),
            Default::default(),
            Default::default(),
        )
        .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        runtime
            .evaluate_preamble(preamble)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        let report = runtime
            .run_document()
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?
            .into_native();
        runtime
            .synchronize_final_document()
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        serialize(runtime.document(), &report)
    }
}
