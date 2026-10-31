//! The box of an absolutely positioned table whose `width` is `auto` is drawn
//! as wide as the table's content, not as wide as its containing block.

use raikiri::html_to_png;

struct Decoded {
    width: u32,
    rgba: Vec<u8>,
}

impl Decoded {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

fn render(body: &str) -> Decoded {
    let html = format!(
        "<html><head><style>@page {{ size: 100px 60px }} body {{ margin: 0 }}</style></head><body>{body}</body></html>"
    );
    let png = html_to_png(html.as_bytes()).expect("render");
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().expect("png header");
    let mut buffer = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buffer).expect("png frame");
    buffer.truncate(info.buffer_size());
    Decoded {
        width: info.width,
        rgba: buffer,
    }
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// A 24x24 table (3px border, 4px padding, a 10x10 cell) at the top left of a
/// containing block as wide as the page, with the given extra `style`.
fn table(style: &str) -> String {
    format!(
        "<div style=\"position:relative;height:50px\">\
         <div style=\"display:table;position:absolute;top:0;{style};border:3px solid blue;\
         padding:4px;background:red\">\
         <div style=\"display:table-row\"><div style=\"display:table-cell;background:green\">\
         <div style=\"width:10px;height:10px\"></div></div></div></div></div>"
    )
}

#[test]
fn the_background_and_border_end_at_the_content() {
    let image = render(&table("left:0"));
    // Border, padding and cell across the middle of the table.
    assert_eq!(image.pixel(1, 12), BLUE);
    assert_eq!(image.pixel(5, 12), RED);
    assert_eq!(image.pixel(12, 12), GREEN);
    assert_eq!(image.pixel(19, 12), RED);
    assert_eq!(image.pixel(22, 12), BLUE);
    // Nothing is drawn to the right of the table's 24px.
    assert_eq!(image.pixel(24, 12), WHITE);
    assert_eq!(image.pixel(60, 12), WHITE);
    assert_eq!(image.pixel(99, 12), WHITE);
}

#[test]
fn both_offsets_leave_the_table_at_its_content_width() {
    let image = render(&table("left:10px;right:10px"));
    assert_eq!(image.pixel(11, 12), BLUE);
    assert_eq!(image.pixel(32, 12), BLUE);
    assert_eq!(image.pixel(34, 12), WHITE);
    assert_eq!(image.pixel(60, 12), WHITE);
}
