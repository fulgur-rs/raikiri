use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Fuzzy {
    difference: (u64, u64),
    pixels: (u64, u64),
}

fn invalid() -> ReftestError {
    ReftestError::RaikiriRender("invalid WPT fuzzy metadata".into())
}

fn range(value: &str) -> Result<(u64, u64), ReftestError> {
    let (lo, hi) = value
        .trim()
        .split_once('-')
        .unwrap_or((value.trim(), value.trim()));
    let lo = lo.trim().parse().map_err(|_| invalid())?;
    let hi = hi.trim().parse().map_err(|_| invalid())?;
    Ok((lo, hi))
}

pub(super) fn metadata(html: &str, pair: &ReftestPair) -> Result<Option<Fuzzy>, ReftestError> {
    let parsed = raikiri_html::parse(
        html.as_bytes(),
        &raikiri::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .map_err(|e| ReftestError::RaikiriRender(format!("parse fuzzy: {e:?}")))?;
    let mut stack = vec![parsed.dom.root_index()];
    let mut references = Vec::new();
    let mut contents = Vec::new();
    while let Some(id) = stack.pop() {
        let Some(node) = parsed.dom.get_node(id) else {
            continue;
        };
        stack.extend(node.children.iter().rev().copied());
        let html_namespace =
            parsed.dom.element_namespace_uri(id) == Some("http://www.w3.org/1999/xhtml");
        if html_namespace
            && node.tag_name() == Some("link")
            && matches!(node.attribute("rel"), Some("match" | "mismatch"))
        {
            let href = node.attribute("href").ok_or_else(invalid)?;
            references.push((href, node.attribute("rel") == Some("match")));
        }
        if html_namespace
            && node.tag_name() == Some("meta")
            && node.attribute("name") == Some("fuzzy")
        {
            contents.push(node.attribute("content").ok_or_else(invalid)?);
        }
    }
    if contents.is_empty() {
        return Ok(None);
    }
    let canonical_test = std::fs::canonicalize(&pair.test).unwrap_or_else(|_| pair.test.clone());
    let root = canonical_test
        .ancestors()
        .find(|p| p.join("resources/testharness.js").is_file());
    let url = |path: &Path| -> Result<raikiri::Url, ReftestError> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(root) = root {
            let relative = path.strip_prefix(root).map_err(|_| invalid())?;
            raikiri::Url::parse("https://wpt.invalid/")
                .map_err(|_| invalid())?
                .join(&relative.to_string_lossy())
                .map_err(|_| invalid())
        } else {
            raikiri::Url::from_file_path(&path).map_err(|_| invalid())
        }
    };
    let base = url(&pair.test)?;
    let reference = url(&pair.reference)?;
    // The WPT manifest enumerates match links before mismatch links.
    references.sort_by_key(|(_, is_match)| !is_match);
    let mut relations = std::collections::HashMap::new();
    for (href, is_match) in references {
        let target = base.join(href).map_err(|_| invalid())?;
        relations.entry(target).or_insert(if is_match {
            ReftestKind::Match
        } else {
            ReftestKind::Mismatch
        });
    }
    let mut entries = std::collections::HashMap::new();
    for content in contents {
        let (key, value) = content
            .rsplit_once(':')
            .map_or((None, content), |(k, v)| (Some(k.trim()), v));
        let key = key
            .map(|k| base.join(k).map_err(|_| invalid()))
            .transpose()?;
        if key.as_ref().is_some_and(|k| !relations.contains_key(k)) {
            return Err(invalid());
        }
        let fields: Vec<_> = value.split(';').collect();
        if fields.len() != 2 {
            return Err(invalid());
        }
        let mut named = std::collections::HashMap::new();
        let mut positional = std::collections::VecDeque::new();
        for field in fields {
            if let Some((name, value)) = field.split_once('=') {
                let name = name.trim();
                if !matches!(name, "maxDifference" | "totalPixels")
                    || named.insert(name, range(value)?).is_some()
                {
                    return Err(invalid());
                }
            } else {
                positional.push_back(range(field)?);
            }
        }
        let mut take = |name| {
            named
                .remove(name)
                .or_else(|| positional.pop_front())
                .ok_or_else(invalid)
        };
        let fuzzy = Fuzzy {
            difference: take("maxDifference")?,
            pixels: take("totalPixels")?,
        };
        if entries.insert(key, fuzzy).is_some() {
            return Err(invalid());
        }
    }
    // spec: https://web-platform-tests.org/writing-tests/reftests.html#fuzzy-matching
    let selected = reference
        .join(&pair.reference_suffix)
        .map_err(|_| invalid())?;
    let keyed = if relations.get(&selected) == Some(&pair.kind) {
        entries.get(&Some(selected))
    } else {
        None
    };
    Ok(keyed.or_else(|| entries.get(&None)).copied())
}

pub(super) fn compare(left: &RenderedImage, right: &RenderedImage, fuzzy: Fuzzy) -> ImageDiff {
    let total_pixels = u64::from(left.width) * u64::from(left.height);
    if left.width != right.width
        || left.height != right.height
        || left.rgba.len() != right.rgba.len()
        || left.rgba.len() as u64 != total_pixels * 4
    {
        return ImageDiff {
            mismatched_pixels: total_pixels,
            total_pixels,
            first_mismatch: None,
            matched: false,
        };
    }
    let mut count = 0;
    let mut maximum = 0;
    for (a, b) in left.rgba.chunks_exact(4).zip(right.rgba.chunks_exact(4)) {
        let delta = (0..3)
            .map(|i| a[i].abs_diff(b[i]) as u64)
            .max()
            .unwrap_or(0);
        count += u64::from(delta != 0);
        maximum = maximum.max(delta);
    }
    let matched = (count == 0 && fuzzy.pixels.0 == 0)
        || (maximum == 0 && fuzzy.difference.0 == 0)
        || (fuzzy.difference.0 <= maximum
            && maximum <= fuzzy.difference.1
            && fuzzy.pixels.0 <= count
            && count <= fuzzy.pixels.1);
    ImageDiff {
        mismatched_pixels: count,
        total_pixels,
        first_mismatch: None,
        matched,
    }
}
