//! Scratch render two files to PPM. Args: <test> <ref> <out_prefix>
use std::path::PathBuf;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args[3].clone();
    for (i, suffix) in [(1usize, "test"), (2usize, "ref")] {
        let html = std::fs::read_to_string(PathBuf::from(&args[i])).unwrap();
        let img = raikiri_wpt::reftest::render_raikiri(&html, 800, 600).unwrap();
        let mut buf = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
        for px in img.rgba.chunks(4) {
            buf.extend_from_slice(&[px[0], px[1], px[2]]);
        }
        std::fs::write(format!("{}_{}.ppm", out, suffix), &buf).unwrap();
    }
    eprintln!("done {}", out);
}
