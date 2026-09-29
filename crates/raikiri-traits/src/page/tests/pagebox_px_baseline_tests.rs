use super::super::*;

#[test]
fn a4_dimensions_match_css_px_conversion() {
    // 210mm × 297mm を CSS px (1/96 in) 換算:
    //   width  = 210mm × 96/25.4 ≈ 793.7008
    //   height = 297mm × 96/25.4 ≈ 1122.5197
    assert!(
        (PageBox::A4.width - 793.7008).abs() < 0.001,
        "A4.width should be ~793.7008 px, got {}",
        PageBox::A4.width
    );
    assert!(
        (PageBox::A4.height - 1122.5197).abs() < 0.001,
        "A4.height should be ~1122.5197 px, got {}",
        PageBox::A4.height
    );
}

#[test]
fn us_letter_dimensions_match_exact_integers() {
    // 8.5in × 11in @ 96 DPI = 816 × 1056 px exactly
    assert_eq!(PageBox::US_LETTER.width, 816.0);
    assert_eq!(PageBox::US_LETTER.height, 1056.0);
}

#[test]
fn pagebox_default_is_a4() {
    assert_eq!(PageBox::default(), PageBox::A4);
}

#[test]
fn pagebox_from_page_size_resolves_lengths_and_orientation() {
    let box_size = PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
        width: raikiri_style::Length::Px(300.0),
        height: raikiri_style::Length::Px(50.0),
    }));
    assert_eq!(
        box_size,
        PageBox {
            width: 300.0,
            height: 50.0
        }
    );

    let landscape = PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
        keyword: Some(raikiri_style::PageSizeKeyword::A5),
        orientation: Some(raikiri_style::PageOrientation::Landscape),
    }));
    assert!(landscape.width > landscape.height);
}

#[test]
fn pagebox_from_page_size_covers_orientation_swaps() {
    assert_eq!(
        apply_page_orientation(50.0, 300.0, Some(PageOrientation::Landscape)),
        (300.0, 50.0)
    );
    assert_eq!(
        apply_page_orientation(300.0, 50.0, Some(PageOrientation::Portrait)),
        (50.0, 300.0)
    );
    assert_eq!(apply_page_orientation(300.0, 50.0, None), (300.0, 50.0));
}

#[test]
fn pagebox_from_page_size_covers_length_units() {
    let cases = [
        (raikiri_style::Length::Px(1.0), 1.0),
        (raikiri_style::Length::Pt(1.0), 96.0 / 72.0),
        (raikiri_style::Length::Cm(1.0), 96.0 / 2.54),
        (raikiri_style::Length::Mm(1.0), 96.0 / 25.4),
        (raikiri_style::Length::Q(1.0), 96.0 / 101.6),
        (raikiri_style::Length::In(1.0), 96.0),
        (raikiri_style::Length::Pc(1.0), 16.0),
        (raikiri_style::Length::Em(1.0), 16.0),
        (raikiri_style::Length::Rem(1.0), 16.0),
        (raikiri_style::Length::Ex(1.0), 8.0),
        (raikiri_style::Length::Ch(1.0), 8.0),
        (raikiri_style::Length::Ic(1.0), 16.0),
        (raikiri_style::Length::Rex(1.0), 8.0),
        (raikiri_style::Length::Rch(1.0), 8.0),
        (raikiri_style::Length::Ric(1.0), 16.0),
        (raikiri_style::Length::Lh(1.0), 16.0),
        (raikiri_style::Length::Rlh(1.0), 16.0),
    ];
    for (length, expected) in cases {
        let page = PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
            width: length,
            height: raikiri_style::Length::Px(1.0),
        }));
        assert!((page.width - expected).abs() < 0.001, "got {}", page.width);
    }
}

#[test]
fn pagebox_from_page_size_covers_named_keywords_and_invalid_fallbacks() {
    let keywords = [
        raikiri_style::PageSizeKeyword::A5,
        raikiri_style::PageSizeKeyword::A4,
        raikiri_style::PageSizeKeyword::A3,
        raikiri_style::PageSizeKeyword::B5,
        raikiri_style::PageSizeKeyword::B4,
        raikiri_style::PageSizeKeyword::JisB5,
        raikiri_style::PageSizeKeyword::JisB4,
        raikiri_style::PageSizeKeyword::Letter,
        raikiri_style::PageSizeKeyword::Legal,
        raikiri_style::PageSizeKeyword::Ledger,
    ];
    for keyword in keywords {
        let page = PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
            keyword: Some(keyword),
            orientation: None,
        }));
        assert!(page.width > 0.0 && page.height > page.width);
    }

    assert_eq!(
        PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
            keyword: None,
            orientation: None,
        })),
        PageBox::A4
    );
    assert_eq!(
        PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
            width: raikiri_style::Length::Px(10.0),
            height: raikiri_style::Length::Percent(50.0),
        })),
        PageBox::A4
    );
    assert_eq!(
        PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
            width: raikiri_style::Length::Px(0.0),
            height: raikiri_style::Length::Px(10.0),
        })),
        PageBox::A4
    );
}

#[test]
fn pagebox_from_page_size_uses_a4_for_auto_or_invalid_relative_input() {
    assert_eq!(
        PageBox::from_page_size(Some(raikiri_style::PageSize::Auto)),
        PageBox::A4
    );
    assert_eq!(
        PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
            width: raikiri_style::Length::Percent(50.0),
            height: raikiri_style::Length::Px(50.0),
        })),
        PageBox::A4
    );
}
