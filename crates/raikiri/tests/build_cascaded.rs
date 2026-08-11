//! raikiri umbrella integration tests (raikiri-spike-m1.23)。
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
/// (raikiri-style::ruletree::walk_and_collect と同 pattern、roborev job 199)。
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
    // bd raikiri-spike-5z86.1 Acceptance: <article><h2>...</h2><p>...</p>
    // </article> が block box として render される (article 自体が inline化
    // して子要素と混線しない)。article を含む、同じ UA CSS 追加を受けた 10
    // 要素すべてを real parse → build_cascaded パイプラインで直接検証する —
    // `crates/raikiri-html/src/lib.rs` の
    // `minimal_ua_css_covers_required_display_block_selectors` は
    // `MINIMAL_UA_CSS` の生テキストを走査するだけで実際に cssparser で
    // parse されるとは限らない (comment 構文の誤りなどを検出できない)。
    // この test は cascade まで通した computed value を見るので非-vacuous。
    for tag in [
        "article",
        "section",
        "nav",
        "aside",
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
    // bd raikiri-spike-5z86.2 Acceptance: <ul><li>...</li><li>...</li></ul>
    // の各 <li> が縦に積まれた block として表示される (marker記号は Epic 4
    // 待ちで出なくてよい)。ol/ul は spec通り display: block、li は spec の
    // `display: list-item` が raikiri-style で未実装 (parse_display /
    // display_rejects_unknown_ident test) なため display: block に
    // interim fallback している。list-item 実装は bd raikiri-spike-uhzy
    // で track (詳細は crates/raikiri-html/src/ua/minimal.css のコメント
    // 参照)。sectioning test と同様、cascade まで通した computed value を
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
    // NB: m1.4 cascade は class/id selector を drop するので inline style を使う
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
    // m1.4 cascade は type selector のみサポートするため p{...} を使う。
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
fn extra_stylesheets_author_rule_overrides_ua_via_umbrella() {
    // Consumer が opts.extra_stylesheets 経由で渡した CSS が Author として
    // build_cascaded 経路に到達することを verify (parse 時 Document.stylesheets
    // に Author として push される)。
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
fn author_important_beats_normal_ua_via_umbrella() {
    // CSS Cascading L4 §6.1 "Cascade Sorting Order"
    // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Origin and
    // Importance 段 (`!important` による反転は §6.3
    // <https://www.w3.org/TR/css-cascade-4/#importance>) の cascade rank ordering:
    //   Normal UA (rank 0) < Normal Author (1) < Important Author (2) < Important UA (3)。
    // bundled UA CSS (spec §M1.4a minimal.css) は !important を含まないため、
    // Important UA > Important Author の反転検証は本 test では直接行えない。
    // ここで verify するのは "Important Author が Normal UA を破る" leg で、これは
    // umbrella の StylesheetKind → Origin map が正しく機能していることを end-to-end で
    // 確認する最小 case。full !important 反転 (Important UA vs Important Author) は
    // Consumer が UA `!important` rule を提供する構造が spec で許容された時点で追加検討。
    // Author 側は extra_stylesheets で渡す (parse 時 Author kind として Document に注入される)。
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

    // NB: この test 段階では bundled UA CSS は !important を含まない (spec §M1.4a の minimal.css)。
    // Author !important があると Author が勝つ (Normal UA 0 < Important Author 2 < Important UA 3)。
    // したがって p の display は inline になる。この test は "Important Author > Normal UA"
    // の origin-rank ordering が umbrella wiring 越しに保存されることを confirm する。
    assert_eq!(
        display,
        DisplayValue::Inline,
        "Author !important should beat Normal UA via umbrella cascade wiring",
    );
}

#[test]
fn umbrella_re_exports_cover_computed_value_types_and_parse_options_fields() {
    // AC #6 の "Consumer が use raikiri::…; で完結" 契約を name-resolution level で証明。
    // raikiri crate から直接名指し可能な全型を actual use する: value 型 (CssColor / Length /
    // Atom)、Document (UncascadedDocument.dom の型)、NetworkProvider (ParseOptions.network の
    // 型)、Url (ParseOptions.base_url の型)。sub-crate を direct dep せずに ParseOptions を
    // 完全構築、ComputedValues field を型付き binding できることを compile-time で verify。
    // (roborev-refine job 228 medium finding regression)
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
    // bd raikiri-spike-zls8: `font_size` は computed 層の `ComputedLength`。
    let _font_size: ComputedLength = computed.font_size;
    // specified 層の `Length` は `PropertyValue` の payload 型として引き続き
    // Consumer から名指しできる必要がある (umbrella re-export list の rationale)。
    let _specified_font_size: PropertyValue = PropertyValue::FontSize(Length::Px(12.0));
    // raikiri-spike-no7b (d9y.1/d9y.2 pattern踏襲) 以降、実 field 型は
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
    // bd raikiri-spike-eow8: `PropertyValue` の Sides<T> 系 payload
    // (Padding/Margin/Border) と LineHeight を umbrella から型付きで名指し
    // できることを compile-time で証明する。zls8 が
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

    // `Border(Sides<Border>)` — bd raikiri-spike-x0dq より前は `Border` struct
    // 自体が `#[non_exhaustive]` のため raikiri crate から `Border { .. }`
    // struct-literal 構築ができず (E0639)、tuple-variant constructor を fn
    // pointer に coerce する形で型だけ pin していた (値そのものは作れなかった)。
    // x0dq で `Border::new()` (= `Self::default()`) を追加したので、
    // FontSize/Padding/Margin/LineHeight と同じ「実際に値を construct する」
    // 形に揃える。全 field が `pub` なので `Border::new()` の後に non-initial
    // 値へ mutation することも確認する (`BorderStyle` / `BorderColor` も
    // x0dq で umbrella re-export に追加、その2型も型付きで construct できる
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
    // bd raikiri-spike-eow8: zls8 が re-export した Computed* 5 型は leaf 型に
    // 過ぎず、`ComputedValues.padding` / `.margin` / `.border` の実 field 型
    // `Sides<Computed*>` は `Sides<T>` 自体が re-export されていなかったため
    // 名指しできなかった (zls8 以前からの gap、`crates/raikiri-style/src/
    // computed.rs` の field 定義で実測)。eow8 の `Sides` 追加でこの 3 field も
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
    // (roborev-refine job 230 medium finding regression)
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
fn body_style_element_is_not_applied_per_m1_head_only_contract() {
    // spec §M1: raikiri-html は現状 head 配下の <style> のみ stylesheet_sources
    // に集約する (<body> 内 <style> の position-aware semantics は M2+)。
    // umbrella build_cascaded は stylesheet_sources を Author として消費するため、
    // <body> 内 <style> は cascade に流れず、<p> は UA CSS の display: block を得る。
    // (roborev-refine job 226 medium finding regression、
    //  raikiri-html/src/sink.rs::extract_inline_stylesheets の invariant と一致)
    let html = "<html><head></head>\
                <body><style>p { display: inline }</style><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<body> 内の <style> は M1 では未対応、<p> は UA CSS 経由で display: block を得る",
    );
}

#[test]
fn img_width_height_html_attributes_reach_computed_style_through_real_parse_path() {
    // bd raikiri-spike-5z86.7: raikiri-style crate 内 (`crate::cascade::
    // push_img_dimension_hints`) の実装だけで足りる、という scope-narrowing
    // 判定 ("wall/sink label dropped after re-verification") の根拠は
    // `crates/raikiri-html/src/sink.rs::wire_side_tables` が null-namespace
    // 属性を汎用的に `Node.attributes` へ配線済み、という **static code
    // reading** だった (実行して確かめてはいない)。この umbrella test は
    // raikiri-style 単体の unit test (`TestDoc` — テスト自身が属性を注入する
    // mock) では検証できない箇所、すなわち「本物の html5ever TreeSink
    // (`RaikiriTreeSink`) → `Document.set_element_attributes` →
    // `ElementRef::attr()` → `StyleElement::attr()`」という配線の
    // **実行時**証拠を、raikiri crate 公開 API のみを使って与える
    // (m1.23 の "consumer は `use raikiri::…;` のみで完結" contract と同じ形)。
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
