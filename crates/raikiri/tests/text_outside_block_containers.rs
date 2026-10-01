//! Text that no block container of the document holds directly is still
//! laid out and drawn: captions, form control labels and text of
//! `display: contents` items. Text no paragraph can hold fails the layout
//! instead of disappearing.

use raikiri::{FontCollectionBuilder, html_to_png_with_render_fonts};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn render(body: &str) -> Result<Vec<u8>, raikiri::RenderError> {
    let html = format!(
        "<html><head><style>*{{font-family:Ahem;font-size:20px;line-height:20px}}</style></head><body style=\"margin:0\">{body}</body></html>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM)
        .build()
        .expect("fonts");
    html_to_png_with_render_fonts(html.as_bytes(), fonts)
}

/// Black pixels of `body` rendered with Ahem at 20px: "Hello" is five 20px
/// squares, 2000 pixels of ink.
fn ink(body: &str) -> usize {
    let png = render(body).expect("render");
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().expect("png header");
    let mut buffer = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buffer).expect("png frame");
    buffer[..info.buffer_size()]
        .chunks(4)
        .filter(|pixel| pixel[0] < 16 && pixel[1] < 16 && pixel[2] < 16)
        .count()
}

#[test]
fn a_block_of_text_is_drawn() {
    assert_eq!(ink("<div>Hello</div>"), 2000);
}

#[test]
fn a_table_caption_is_drawn() {
    assert_eq!(
        ink("<table><caption>Hello</caption><tr><td></td></tr></table>"),
        2000
    );
}

#[test]
fn a_button_label_is_drawn() {
    assert_eq!(ink("<button>Hello</button>"), 2000);
}

#[test]
fn the_option_of_a_select_is_drawn() {
    assert_eq!(ink("<select><option>Hello</option></select>"), 2000);
}

#[test]
fn the_text_of_a_textarea_is_drawn() {
    assert_eq!(ink("<textarea>Hello</textarea>"), 2000);
}

#[test]
fn text_directly_in_a_table_row_fails_the_layout() {
    // CSS 2.1 17.2.1 wraps such text in an anonymous cell; the table
    // algorithm builds no anonymous cells, so no paragraph holds the text.
    for body in [
        r#"<div style="display:table"><div style="display:table-row">Hello</div></div>"#,
        r#"<div style="display:table"><div style="display:table-row-group">Hello</div></div>"#,
    ] {
        assert!(
            matches!(
                render(body),
                Err(raikiri::RenderError::Layout(
                    raikiri::LayoutError::IfcUnsupported { .. }
                ))
            ),
            "{body}"
        );
    }
}

#[test]
fn text_of_a_contents_child_of_a_flex_container_is_drawn() {
    assert!(
        ink(r#"<div style="display:flex"><span style="display:contents">Hello</span></div>"#)
            >= 2000
    );
}
