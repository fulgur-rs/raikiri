//! raikiri umbrella integration tests。
//!
//! Consumer が `use raikiri::…;` のみで parse → build_cascaded → display 判定を
//! 完結できることを verify する。

use raikiri::{
    DisplayValue, Dom, Element, Node, NodeId, NodeKind, ParseOptions, build_cascaded, parse,
};

fn parse_html(source: &str) -> raikiri::UncascadedDocument {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    parse(source.as_bytes(), &opts).expect("parse")
}

/// DOM を root から iterative DFS walk して最初に見つかった tag 一致の
/// Element の NodeId を返す。
///
/// 深いネストで stack overflow しないよう explicit `Vec` stack で iterative
/// (raikiri-style::ruletree::walk_and_collect と同 pattern)。
/// stack は LIFO なので、pre-order (sibling 間 document order) を保つため
/// children を reverse push する。
fn find_by_tag<D: Dom>(dom: &D, tag: &str) -> Option<NodeId> {
    let mut stack: Vec<NodeId> = vec![dom.root_id()];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            if node.kind() == NodeKind::Element
                && let Some(elem) = node.as_element()
                && elem.tag_name().eq_ignore_ascii_case(tag)
            {
                return Some(id);
            }
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
    None
}

#[test]
fn p_without_author_style_is_display_block_via_ua_css() {
    let doc = parse_html("<html><body><p>Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<p> should inherit display: block from bundled UA CSS via build_cascaded",
    );
}

#[test]
fn sectioning_and_grouping_elements_are_display_block_via_ua_css() {
    // Acceptance: <article><h2>...</h2><p>...</p>
    // </article> が block box として render される (article 自体が inline化
    // して子要素と混線しない)。article を含む、同じ UA CSS 追加を受けた 11
    // 要素すべてを real parse → build_cascaded パイプラインで直接検証する —
    // `crates/raikiri-html/src/lib.rs` の
    // `minimal_ua_css_covers_required_display_block_selectors` は
    // `MINIMAL_UA_CSS` の生テキストを走査するだけで実際に cssparser で
    // parse されるとは限らない (comment 構文の誤りなどを検出できない)。
    // この test は cascade まで通した computed value を見るので非-vacuous。
    // hgroup 追加 (article/aside/nav/section と同じ
    // §sections-and-headings (15.3.6) selector group の一員、当初の
    // scope からは漏れていた)。
    for tag in [
        "article",
        "section",
        "nav",
        "aside",
        "hgroup",
        "header",
        "footer",
        "main",
        "figure",
        "figcaption",
        "blockquote",
    ] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "<{tag}> should be display: block from bundled UA CSS via build_cascaded",
        );
    }
}

#[test]
fn list_elements_are_display_block_via_ua_css() {
    // Acceptance: <ul><li>...</li><li>...</li></ul>
    // の各 <li> が縦に積まれた block として表示される (marker記号は将来
    // 対応待ちで出なくてよい)。ol/ul は spec通り display: block、li は spec の
    // `display: list-item` が raikiri-style で未実装 (parse_display /
    // display_rejects_unknown_ident test) なため display: block に
    // interim fallback している (詳細は crates/raikiri-html/src/ua/minimal.css の
    // コメント参照)。sectioning test と同様、cascade まで通した computed value を
    // 見るので非-vacuous。
    for tag in ["ol", "ul", "li"] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "<{tag}> should be display: block from bundled UA CSS via build_cascaded",
        );
    }
}

#[test]
fn li_block_fallback_stacks_siblings_as_boxes() {
    // Acceptance の literal fixture を直接再現: <ul><li>A</li><li>B</li></ul>
    // の2つの <li> が両方とも display: block であることを、tag 走査ではなく
    // 実際の親子構造 (ul の child_ids) 経由で確認する — 1つ目の <li> だけを
    // 見る find_by_tag では2つ目の <li> の取りこぼしを検出できないため。
    let doc = parse_html("<html><body><ul><li>A</li><li>B</li></ul></body></html>");
    let result = build_cascaded(&doc);

    let ul_id = find_by_tag(&doc.dom, "ul").expect("<ul> exists");
    let li_ids: Vec<NodeId> = doc.dom.child_ids(ul_id).collect();
    assert_eq!(li_ids.len(), 2, "expected two <li> children under <ul>");
    for li_id in li_ids {
        let node = doc.dom.node(li_id).expect("child node exists");
        assert_eq!(
            node.kind(),
            NodeKind::Element,
            "expected an Element child under <ul>",
        );
        let tag = node
            .as_element()
            .expect("Element node has as_element()")
            .tag_name()
            .to_owned();
        assert_eq!(
            tag, "li",
            "expected <ul> children to be <li>, found <{tag}>"
        );

        let display = result.computed[li_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "each <li> under <ul> should be display: block from bundled UA CSS",
        );
    }
}

#[test]
fn author_inline_style_overrides_ua_display_block() {
    // NB: cascade は class/id selector を drop するので inline style を使う
    let doc = parse_html("<html><body><p style=\"display:inline\">Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "author inline style (Normal Author) should override UA (Normal UA) per cascade rank",
    );
}

#[test]
fn dom_style_element_author_rule_overrides_ua() {
    // 明示的な <style> Author rule が UA を上回ることを verify。
    // cascade は type selector のみサポートするため p{...} を使う。
    let html = "<html><head><style>p { display: inline }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "DOM <style> Author rule should override UA (both via umbrella wiring)",
    );
}

#[test]
fn lang_pseudo_class_inherits_from_html_lang_attribute_through_real_parse_pipeline() {
    // Acceptance test, exercised through the *real*
    // html5ever parse -> raikiri-dom -> build_cascaded pipeline (not the
    // raikiri-style-internal `TestDoc` mock other coverage for this feature
    // uses):
    // `<html lang="ja">` with a `<p>` descendant that carries no `lang`
    // attribute of its own must still match `:lang(ja)`. This also pins
    // that the `lang` attribute wiring
    // (`raikiri-html`'s sink -> `raikiri-dom::Node.attributes` ->
    // `ElementRef::attr`) actually surfaces `lang` where
    // `raikiri-style::StyleElement::attr("lang")` reads it end to end.
    let html = "<html lang=\"ja\"><head>\
                <style>:lang(ja) { font-family: serif-ja }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let font_family = &result.computed[p_id.0 as usize].font_family;
    assert_eq!(
        font_family[0].to_string(),
        "serif-ja",
        ":lang(ja) must match <p> via the ancestor <html lang=\"ja\">, \
         through the real parse pipeline",
    );
}

#[test]
fn extra_stylesheets_user_rule_overrides_ua_via_umbrella() {
    // Consumer が opts.extra_stylesheets 経由で渡した CSS が User origin として
    // build_cascaded 経路に到達することを verify (parse 時 Document.stylesheets
    // に User kind として push される、Author retag から分離済み)。normal User
    // (rank 1) は normal UserAgent (rank 0) より強いため UA CSS を上書きする。
    let extra: &[&str] = &["p { display: inline }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(display, DisplayValue::Inline);
}

#[test]
fn style_inside_template_element_does_not_affect_cascade() {
    // <template> は spec 上 inert。内部の <style> は cascade に流れず、
    // <p> は UA CSS のみで `display: block` を取る。
    // (raikiri-html/src/sink.rs:315-318 の invariant を umbrella surface で検証)
    let html = "<html><head><template><style>p { display: inline }</style></template></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<template> 内の <style> は inert として無視され、<p> は UA CSS の display: block を得る",
    );
}

#[test]
fn user_important_beats_normal_ua_via_umbrella() {
    // CSS Cascading L4 §6.1 "Cascade Sorting Order"
    // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Origin and
    // Importance 段 (`!important` による反転は §6.3
    // <https://www.w3.org/TR/css-cascade-4/#importance>)。raikiri-style の full
    // cascade_rank ordering (4-tier 化、8-arm total):
    //   Normal UA(0) < Normal User(1) < Normal Hint(2) < Normal Author(3) <
    //   Important Author(4) < Important Hint(5) < Important User(6) < Important UA(7)。
    // bundled UA CSS (minimal.css) は !important を含まないため、
    // Important UA との反転検証は本 test では直接行えない。
    // ここで verify するのは "Important User が Normal UA を破る" leg で、これは
    // umbrella の StylesheetKind → Origin map (extra_stylesheets → `Origin::User`) が
    // 正しく機能していることを end-to-end で確認する最小
    // case。extra_stylesheets で渡す (parse 時 User kind として Document に注入
    // される、Author retag から分離済み)。
    let extra: &[&str] = &["p { display: inline !important }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;

    // NB: この test 段階では bundled UA CSS は !important を含まない (minimal.css)。
    // User !important があると User が勝つ (Normal UA 0 < ... < Important User 6)。
    // したがって p の display は inline になる。この test は "Important User > Normal UA"
    // の origin-rank ordering が umbrella wiring 越しに保存されることを confirm する。
    assert_eq!(
        display,
        DisplayValue::Inline,
        "Important User (extra_stylesheets) should beat Normal UA via umbrella cascade wiring",
    );
}

#[test]
fn umbrella_re_exports_cover_computed_value_types_and_parse_options_fields() {
    // AC #6 の "Consumer が use raikiri::…; で完結" 契約を name-resolution level で証明。
    // raikiri crate から直接名指し可能な全型を actual use する: value 型 (CssColor / Length /
    // Atom)、Document (UncascadedDocument.dom の型)、NetworkProvider (ParseOptions.network の
    // 型)、Url (ParseOptions.base_url の型)。sub-crate を direct dep せずに ParseOptions を
    // 完全構築、ComputedValues field を型付き binding できることを compile-time で verify。
    use raikiri::{
        Atom, ComputedLength, CssColor, Document, Length, NetworkProvider, ParseOptions,
        PropertyValue, Url, build_cascaded, parse,
    };

    // NetworkProvider trait を dyn 経由で名指し可能なことを compile-time で確認。
    let _network: Option<&dyn NetworkProvider> = None;
    // Url を parse できることを確認 (base_url に渡す想定)。
    let base = Url::parse("https://example.com/").expect("url parse");
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: _network,
        base_url: Some(base),
    };

    let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
    // UncascadedDocument.dom: Document を明示 type annotation で受ける (Document re-export 確認)。
    let _dom: &Document = &doc.dom;

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let computed = &result.computed[p_id.0 as usize];

    // ComputedValues field を型付き binding で受け、value 型が名指しできることを verify。
    let _color: CssColor = computed.color;
    // `font_size` は computed 層の `ComputedLength`。
    let _font_size: ComputedLength = computed.font_size;
    // specified 層の `Length` は `PropertyValue` の payload 型として引き続き
    // Consumer から名指しできる必要がある (umbrella re-export list の rationale)。
    let _specified_font_size: PropertyValue = PropertyValue::FontSize(Length::Px(12.0));
    // 実 field 型は
    // `Arc<Vec<Atom>>` — 下記 binding は `Arc<Vec<T>>: Deref<Target = Vec<T>>`
    // による deref coercion 経由で通る (Content/StringSet 等 sibling field と
    // 同じ「read-side consumer は無改修で継続動作」設計、
    // `PropertyValue::FontFamily` doc 参照)。
    let font_family: &Vec<Atom> = &computed.font_family;
    // 実 assertion — initial font-family は Atom("serif") (raikiri-style::ComputedValues::initial)。
    assert!(
        !font_family.is_empty(),
        "font_family should have at least initial serif atom"
    );
}

#[test]
fn umbrella_re_exports_cover_sides_and_specified_payload_types() {
    // `PropertyValue` の Sides<T> 系 payload
    // (Padding/Margin/Border) と LineHeight を umbrella から型付きで名指し
    // できることを compile-time で証明する。既存の
    // `PropertyValue::FontSize(Length::Px(12.0))` で入れた「実際に construct
    // して確認する」形と同じ shape を、直接 construct できる 3 variant
    // (Padding/Margin/LineHeight) には踏襲する。
    use raikiri::{
        Border, BorderColor, BorderStyle, CssColor, Length, LengthOrAuto, LineHeight,
        PropertyValue, Sides,
    };

    // `Padding(Sides<Length>)` — `Sides<T>` / `Length` はどちらも re-export 済みで
    // struct-literal 制約なく直接 construct できる。
    let _specified_padding: PropertyValue = PropertyValue::Padding(Sides::all(Length::Px(4.0)));

    // `Margin(Sides<LengthOrAuto>)` — `LengthOrAuto` も同様に直接 construct 可能。
    let _specified_margin: PropertyValue =
        PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(8.0))));

    // `LineHeight(LineHeight)` — `LineHeight` enum は `#[non_exhaustive]` だが、
    // 既存 variant の construct 自体は (`Length` 同様) 外部 crate から可能。
    let _specified_line_height: PropertyValue = PropertyValue::LineHeight(LineHeight::Normal);

    // `Border(Sides<Border>)` — 以前は `Border` struct
    // 自体が `#[non_exhaustive]` のため raikiri crate から `Border { .. }`
    // struct-literal 構築ができず (E0639)、tuple-variant constructor を fn
    // pointer に coerce する形で型だけ pin していた (値そのものは作れなかった)。
    // `Border::new()` (= `Self::default()`) の追加で、
    // FontSize/Padding/Margin/LineHeight と同じ「実際に値を construct する」
    // 形に揃った。全 field が `pub` なので `Border::new()` の後に non-initial
    // 値へ mutation することも確認する (`BorderStyle` / `BorderColor` も
    // 同時期に umbrella re-export に追加、その2型も型付きで construct できる
    // ことを合わせて pin する)。
    let mut border = Border::new();
    assert_eq!(
        border.width,
        Length::Px(3.0),
        "Border::new() は CSS 初期値 (medium=3px)"
    );
    assert_eq!(border.style, BorderStyle::None);
    assert_eq!(border.color, BorderColor::CurrentColor);
    border.width = Length::Px(2.0);
    border.style = BorderStyle::Solid;
    border.color = BorderColor::Resolved(CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    });
    let _specified_border: PropertyValue = PropertyValue::Border(Sides::all(border));
}

#[test]
fn umbrella_re_exports_cover_computed_sides_container_fields() {
    // 最初に re-export した Computed* 5 型は leaf 型に
    // 過ぎず、`ComputedValues.padding` / `.margin` / `.border` の実 field 型
    // `Sides<Computed*>` は `Sides<T>` 自体が re-export されていなかったため
    // 名指しできなかった (`crates/raikiri-style/src/
    // computed.rs` の field 定義で実測)。後続の `Sides` 追加でこの 3 field も
    // 型付きで受けられることを、実際の cascade 出力を使って verify する。
    use raikiri::{
        ComputedBorder, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ParseOptions,
        Sides, build_cascaded, parse,
    };

    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let computed = &result.computed[p_id.0 as usize];

    let _padding: Sides<ComputedLengthPercentage> = computed.padding;
    let _margin: Sides<ComputedLengthPercentageOrAuto> = computed.margin;
    let _border: Sides<ComputedBorder> = computed.border;
}

#[test]
fn concrete_network_provider_impl_via_raikiri_only_re_exports() {
    // NetworkProvider trait を implement する downstream consumer が
    // sub-crate direct dep なしで完結できることを compile-time で verify。
    // fetch() の param / return / error 3 型 + Bytes + Url + auxiliary
    // (Method / Body / HeaderMap / ResourceKind) を全て raikiri から import して
    // struct 実装。round-trip 動作までは要求しない (fetch 内で NetworkError::Aborted 即返却)
    // — 目的は trait impl の name resolution 完結性の証明。
    use raikiri::{Bytes, FetchedResource, NetworkError, NetworkProvider, Request, Url};

    struct DummyProvider;

    impl NetworkProvider for DummyProvider {
        fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
            // Request field を全て read できることを compile-time で確認 (unused でも OK)。
            let _url: &Url = &request.url;
            let _kind = request.kind;

            // FetchedResource を Bytes / Url ベースで construct できることを confirm。
            Ok(FetchedResource {
                bytes: Bytes::from_static(b""),
                content_type: None,
                final_url: Url::parse("about:blank").unwrap(),
                encoding: None,
            })
        }
    }

    // dyn dispatch で trait object 化 (`ParseOptions.network` の型と互換性を verify)。
    let provider: &dyn NetworkProvider = &DummyProvider;
    let _network: Option<&dyn NetworkProvider> = Some(provider);
}

#[test]
fn link_rel_stylesheet_fetched_css_reaches_computed_style_through_real_cascade() {
    // <link rel="stylesheet" href="..."> の
    // 検出→fetch (NetworkProvider::fetch, ResourceKind::ExternalStylesheet)
    // →CSS text 化までは raikiri-html 単体 unit test 済み。この umbrella
    // test はその先 — raikiri-html が `UncascadedDocument.stylesheet_sources`
    // に積んだ fetch 結果を、raikiri crate 側の `build_cascaded` が本当に
    // Author stylesheet として RuleTree に統合し、computed style まで届く
    // ことを、本物の parse → build_cascaded pipeline で end-to-end 検証する
    // (img_width_height_html_attributes_reach_computed_style_through_real_parse_path
    // と同じ理由: static reading だけでは「本当に繋がっているか」は確認
    // できない)。
    use raikiri::{
        Bytes, DisplayValue, FetchedResource, NetworkError, NetworkProvider, ParseOptions, Request,
        Url, build_cascaded, parse,
    };

    struct StylesheetProvider;

    impl NetworkProvider for StylesheetProvider {
        fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
            Ok(FetchedResource {
                bytes: Bytes::from_static(b"div { display: none }"),
                content_type: Some("text/css".to_string()),
                final_url: request.url,
                encoding: None,
            })
        }
    }

    let provider = StylesheetProvider;
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&provider as &dyn NetworkProvider),
        base_url: Some(Url::parse("https://example.test/").expect("valid base url")),
    };
    let html = br#"<html><head><link rel="stylesheet" href="a.css"></head><body><div>Hi</div></body></html>"#;
    let doc = parse(&html[..], &opts).expect("parse");

    // Fetch した CSS text が Author stylesheet として届いていることを、
    // raikiri-html 側の契約 (stylesheet_sources) でも確認する。
    assert_eq!(
        doc.stylesheet_sources,
        vec![String::from("div { display: none }")],
        "fetched external stylesheet CSS text must land in stylesheet_sources"
    );

    let result = build_cascaded(&doc);
    let div_id = find_by_tag(&doc.dom, "div").expect("<div> exists");
    let display = result.computed[div_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::None,
        "div {{ display: none }} fetched via <link rel=stylesheet> must beat the UA CSS \
         display:block default through the real build_cascaded pipeline"
    );
}

#[test]
fn body_style_element_is_not_applied_per_m1_head_only_contract() {
    // raikiri-html は現状 head 配下の <style> のみ stylesheet_sources
    // に集約する (<body> 内 <style> の position-aware semantics は将来対応)。
    // umbrella build_cascaded は stylesheet_sources を Author として消費するため、
    // <body> 内 <style> は cascade に流れず、<p> は UA CSS の display: block を得る。
    // (raikiri-html/src/sink.rs::extract_inline_stylesheets の invariant と一致)
    let html = "<html><head></head>\
                <body><style>p { display: inline }</style><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<body> 内の <style> は現状未対応、<p> は UA CSS 経由で display: block を得る",
    );
}

#[test]
fn img_width_height_html_attributes_reach_computed_style_through_real_parse_path() {
    // raikiri-style crate 内 (`crate::cascade::
    // push_img_dimension_hints`) の実装だけで足りる、という scope-narrowing
    // 判定の根拠は
    // `crates/raikiri-html/src/sink.rs::wire_side_tables` が null-namespace
    // 属性を汎用的に `Node.attributes` へ配線済み、という **static code
    // reading** だった (実行して確かめてはいない)。この umbrella test は
    // raikiri-style 単体の unit test (`TestDoc` — テスト自身が属性を注入する
    // mock) では検証できない箇所、すなわち「本物の html5ever TreeSink
    // (`RaikiriTreeSink`) → `Document.set_element_attributes` →
    // `ElementRef::attr()` → `StyleElement::attr()`」という配線の
    // **実行時**証拠を、raikiri crate 公開 API のみを使って与える
    // ("consumer は `use raikiri::…;` のみで完結" contract と同じ形)。
    let doc = parse_html(r#"<html><body><img src="x.png" width="100" height="50"></body></html>"#);
    let result = build_cascaded(&doc);

    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(100.0),
        "width attribute must reach computed style via the real sink → StyleElement::attr() path"
    );
    assert_eq!(
        computed.height,
        raikiri::ComputedLengthPercentageOrAuto::Px(50.0),
        "height attribute must reach computed style via the real sink → StyleElement::attr() path"
    );
}

#[test]
fn img_width_html_attribute_overridable_by_real_author_stylesheet_through_real_parse_path() {
    // 上と同じ real-path 証拠を、cascade-origin の主張 (「Author CSS で
    // 上書き可能」) 側でも取る — `<style>` 由来の Author-origin 宣言が
    // `raikiri::build_cascaded` の source_order 解決 (`stylesheet_kind_to_origin`
    // 含む実 origin 配線) を経由してもなお hint に勝つことを確認する。
    let html = r#"<html><head><style>img { width: 30px }</style></head>
                  <body><img src="x.png" width="100"></body></html>"#;
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(30.0),
        "real <style> Author rule must outrank the author-origin presentational hint \
         (same origin, real rule's non-zero specificity wins) end-to-end"
    );
}

#[test]
fn img_width_presentational_hint_beats_extra_stylesheets_user_origin_via_umbrella() {
    // Consumer-visible behavior change: `extra_stylesheets`
    // is now tagged `StylesheetKind::User` (→ `Origin::User`, normal rank 1), which
    // CSS Cascading L5 §6.5 places *below* `Origin::AuthorPresentationalHint` (normal
    // rank 2) — so `<img width>`'s presentational hint now beats an
    // `extra_stylesheets` rule regardless of specificity. Previously, `extra_stylesheets`
    // was tagged `Author` (rank 3), which beat the hint — contrast with
    // `img_width_html_attribute_overridable_by_real_author_stylesheet_through_real_parse_path`
    // above, where a *real* Author-origin rule (in-document `<style>`) still beats the
    // hint today.
    //
    // The `extra_stylesheets` rule below sets both `width` (contested by the hint,
    // since the `<img>` has a `width` attribute) and `height` (uncontested — no
    // `height` attribute, so no height hint is pushed). Asserting both distinguishes
    // "the hint outranked the width declaration" from "the stylesheet never reached
    // the RuleTree at all" (which would leave *both* properties at their initial
    // value, not just width) — `extra_stylesheets` reachability on its own is already
    // covered by `extra_stylesheets_user_rule_overrides_ua_via_umbrella` above, but this
    // test is the one cited by name from crates/raikiri-style/src/cascade.rs's
    // `push_img_dimension_hints` doc as *the* end-to-end pin for the flipped ranking, so it
    // should stand alone.
    let extra: &[&str] = &["img { width: 30px; height: 7px }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&br#"<img src="x.png" width="100">"#[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(100.0),
        "img width presentational hint (Origin::AuthorPresentationalHint, rank 2) must \
         beat extra_stylesheets (Origin::User, rank 1) — this ranking flipped \
         (extra_stylesheets used to be tagged Author, rank 3)"
    );
    assert_eq!(
        computed.height,
        raikiri::ComputedLengthPercentageOrAuto::Px(7.0),
        "extra_stylesheets' uncontested height declaration must still land — proves the \
         stylesheet did reach the RuleTree and it's specifically the width property that \
         lost to the hint, not the whole stylesheet being absent"
    );
}

#[test]
fn hr_is_display_block_border_inset_and_margin_via_ua_css() {
    // Acceptance: <hr> が水平線として render される。
    // HTML Living Standard §the-hr-element-2 (15.3.11) は `hr { color: gray;
    // border-style: inset; border-width: 1px;
    // margin-block: 0.5em; margin-inline: auto; overflow: hidden; }` を
    // 規定し、`display: block` は別の §flow-content-3 (15.3.3) flow-content
    // group 側から来る。raikiri-style には border-style / border-width の
    // 独立 multi-side property、margin-block / margin-inline logical
    // property のいずれも実装がない (overflow property 自体は
    // 実装済み — 詳細は minimal.css のコメント参照)
    // — 本 test は「minimal.css が実際に宣言
    // している *置換後* の rule」の cascade 出力を pin する (border
    // shorthand + margin shorthand + color)。spec 原文
    // そのものを pin しているわけではない点に注意。
    //
    // NB: `computed.overflow` (raikiri-style `OverflowValue`/`OverflowXY`)
    // is deliberately **not** asserted here — this
    // module's own doc states its purpose is verifying `use raikiri::…;`
    // alone suffices, and `OverflowValue`/`OverflowXY` are not (yet)
    // re-exported at the umbrella crate root (`crates/raikiri/src/lib.rs`
    // re-exports `Border`/`BorderColor`/`BorderStyle`/`LineHeight` from
    // `raikiri_style::property` for the same "consumer needs the payload
    // type to match on this `PropertyValue` variant" reason those were
    // added; `OverflowValue`/`OverflowXY` would be
    // the same shape of follow-up, but deciding the umbrella's public
    // surface is out of scope here).
    // `sectioning_and_grouping_elements_are_display_block_via_ua_css` /
    // `list_elements_are_display_block_via_ua_css` と同様、実 parse ->
    // build_cascaded を通すので非-vacuous (raikiri-html::lib の textual
    // scan は rule 文字列の存在しか確認せず、cssparser が実際に accept
    // するかどうかは見ていない)。
    let doc = parse_html("<html><body><hr></body></html>");
    let result = build_cascaded(&doc);

    let hr_id = find_by_tag(&doc.dom, "hr").expect("<hr> exists");
    let computed = &result.computed[hr_id.0 as usize];

    assert_eq!(
        computed.display,
        DisplayValue::Block,
        "<hr> should be display: block from the flow-content UA CSS group"
    );

    // color: gray は decorative ではなく load-bearing — 下の `border`
    // shorthand は color component を省略しており currentcolor に
    // default するため、border を spec 通り gray に塗らせているのは
    // 実質この color 宣言。省略すると、この hr が本来 inherit するはずの
    // 別の `color` で border が塗られてしまう。
    assert_eq!(
        computed.color,
        raikiri::CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        },
        "<hr> color should resolve to CSS named color `gray`"
    );

    // border-style: inset + border-width: 1px を 4 side 全てに — `border:
    // 1px inset` shorthand 経由の置換 (raikiri-style には独立の
    // border-style/border-width property がない)。top だけでなく 4 side
    // 全てを検査することで、shorthand の `Sides::all` fan-out が実際に
    // 起きたことを確認する。
    for (side_name, side) in [
        ("top", &computed.border.top),
        ("right", &computed.border.right),
        ("bottom", &computed.border.bottom),
        ("left", &computed.border.left),
    ] {
        assert_eq!(
            side.width(),
            raikiri::ComputedLength(1.0),
            "<hr> border-{side_name}-width should be 1px"
        );
        assert_eq!(
            side.style(),
            raikiri::BorderStyle::Inset,
            "<hr> border-{side_name}-style should be inset"
        );
        assert_eq!(
            side.color,
            raikiri::BorderColor::CurrentColor,
            "<hr> border-{side_name}-color should be the shorthand's omitted-color default \
             (currentcolor), not an explicit color"
        );
    }

    // margin-block: 0.5em の fallback (margin shorthand 経由で
    // margin-top/margin-bottom 物理 longhand に展開される — raikiri-style
    // には margin-block logical property がない)。0.5em は inherit
    // された (UA-default の) 16px font-size 基準で解決される。
    assert_eq!(
        computed.margin.top,
        raikiri::ComputedLengthPercentageOrAuto::Px(8.0),
        "<hr> margin-top should be 0.5em (8px at default 16px font-size), the margin-block \
         fallback"
    );
    assert_eq!(
        computed.margin.bottom,
        raikiri::ComputedLengthPercentageOrAuto::Px(8.0),
        "<hr> margin-bottom should be 0.5em (8px at default 16px font-size), the margin-block \
         fallback"
    );

    // margin-inline: auto の fallback (margin shorthand 経由で
    // margin-left/margin-right 物理 longhand に展開される —
    // raikiri-style には margin-inline logical property がない)。
    assert_eq!(
        computed.margin.left,
        raikiri::ComputedLengthPercentageOrAuto::Auto,
        "<hr> margin-left should be auto, the margin-inline fallback"
    );
    assert_eq!(
        computed.margin.right,
        raikiri::ComputedLengthPercentageOrAuto::Auto,
        "<hr> margin-right should be auto, the margin-inline fallback"
    );
}

#[test]
fn flow_content_3_residue_elements_are_display_block_via_ua_css() {
    // Acceptance: the 6 §flow-content-3 (15.3.3)
    // display:block selector members that were previously untracked
    // now resolve to display: block end-to-end.
    // center/listing/plaintext/xmp are HTML LS §16.2 "entirely obsolete"
    // elements, but that classification governs authoring conformance,
    // not UA rendering — §flow-content-3 itself still lists them in the
    // same display:block selector as address/search (full reasoning in
    // minimal.css's comment). dialog, the 7th residue element, is
    // deliberately NOT in this list — its display resolves conditionally
    // on the `open` attribute rather than unconditionally to
    // `display: block`, so it needs its own attribute-driven test rather
    // than fitting this unconditional loop (see
    // `dialog_display_reflects_open_attribute_via_ua_css` below). Same
    // non-vacuous real parse -> build_cascaded pattern as
    // `sectioning_and_grouping_elements_are_display_block_via_ua_css` /
    // `list_elements_are_display_block_via_ua_css` /
    // `hr_is_display_block_border_inset_and_margin_via_ua_css` (the
    // raikiri-html::lib textual scan only confirms the rule text exists,
    // not that cssparser actually accepts it end-to-end).
    for tag in ["address", "center", "listing", "plaintext", "search", "xmp"] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "<{tag}> should be display: block from the flow-content-3 UA CSS group"
        );
    }
}

#[test]
fn dialog_display_reflects_open_attribute_via_ua_css() {
    // HTML LS §flow-content-3 (15.3.3) specifies
    // `dialog:not([open]) { display: none; }` alongside dialog's
    // membership in the section's unconditional `display: block`
    // group selector. minimal.css reproduces that pair without `:not()`
    // support via specificity instead (`dialog { display: none; }`
    // overridden by the higher-specificity `dialog[open] { display:
    // block; }` — see that file's comment for the full rationale).
    //
    // This exercises the real parse -> build_cascaded path (not a
    // raikiri-style unit test with a mock attribute), so that
    // `raikiri_dom::ElementRef::attr()`'s presence-vs-value handling of
    // the bare HTML5 boolean-attribute form (`<dialog open>`, value `""`)
    // is proven end-to-end rather than merely assumed, alongside the
    // non-empty `open="open"` form.
    let cases = [
        ("<dialog>Hi</dialog>", DisplayValue::None),
        ("<dialog open>Hi</dialog>", DisplayValue::Block),
        ("<dialog open=\"open\">Hi</dialog>", DisplayValue::Block),
    ];
    for (fragment, expected) in cases {
        let html = format!("<html><body>{fragment}</body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let dialog_id = find_by_tag(&doc.dom, "dialog").expect("<dialog> exists");
        let display = result.computed[dialog_id.0 as usize].display;
        assert_eq!(
            display, expected,
            "dialog display should reflect the open attribute per minimal.css's \
             dialog/dialog[open] specificity pair (fragment: {fragment:?})"
        );
    }
}
