//! Screenshot process used by the external upstream wptrunner product.

use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use raikiri::Url;
use raikiri_net::SystemHttpProvider;
use raikiri_wpt::{WptHostResolver, render_screen_url};

struct Args {
    url: Url,
    output: PathBuf,
    width: u32,
    height: u32,
    host_file: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("raikiri-wpt-browser: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    let hosts = WptHostResolver::from_hosts_file(&args.host_file).map_err(|e| e.to_string())?;
    let provider = SystemHttpProvider::with_host_overrides(hosts.into_overrides());
    let image = render_screen_url(&provider, args.url, args.width, args.height)
        .map_err(|e| e.to_string())?;
    write_png_atomically(&args.output, &image)
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut url = None;
    let mut output = None;
    let mut window_size = None;
    let mut host_file = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {argument}"))?;
        let slot = match argument.as_str() {
            "--url" => &mut url,
            "--output" => &mut output,
            "--window-size" => &mut window_size,
            "--host-file" => &mut host_file,
            _ => return Err(format!("unknown argument: {argument}")),
        };
        if slot.replace(value).is_some() {
            return Err(format!("duplicate argument: {argument}"));
        }
    }

    let url_text = required(url, "--url")?;
    let url = Url::parse(&url_text).map_err(|error| format!("invalid --url: {error}"))?;
    if url.scheme() != "http" {
        return Err(format!(
            "only http URLs are supported by this prototype, got {url}"
        ));
    }
    let (width, height) = parse_window_size(&required(window_size, "--window-size")?)?;
    Ok(Args {
        url,
        output: required(output, "--output")?.into(),
        width,
        height,
        host_file: required(host_file, "--host-file")?.into(),
    })
}

fn required(value: Option<String>, name: &str) -> Result<String, String> {
    value.ok_or_else(|| format!("missing required argument {name}"))
}

fn parse_window_size(value: &str) -> Result<(u32, u32), String> {
    let (width, height) = value
        .split_once('x')
        .ok_or_else(|| format!("invalid --window-size {value:?}; expected WIDTHxHEIGHT"))?;
    let width = width
        .parse::<u32>()
        .map_err(|error| format!("invalid viewport width {width:?}: {error}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|error| format!("invalid viewport height {height:?}: {error}"))?;
    if width == 0 || height == 0 {
        return Err("viewport dimensions must be positive".into());
    }
    Ok((width, height))
}

fn write_png_atomically(
    output: &Path,
    image: &raikiri_wpt::reftest::RenderedImage,
) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| format!("output path has no file name: {}", output.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let result = write_png(&temporary, image).and_then(|()| {
        fs::rename(&temporary, output).map_err(|error| {
            format!(
                "rename {} to {}: {error}",
                temporary.display(),
                output.display()
            )
        })
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_png(path: &Path, image: &raikiri_wpt::reftest::RenderedImage) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut output = io::BufWriter::new(file);
    {
        let mut encoder = png::Encoder::new(&mut output, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("PNG header: {error}"))?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|error| format!("PNG data: {error}"))?;
        writer
            .finish()
            .map_err(|error| format!("PNG finish: {error}"))?;
    }
    output
        .flush()
        .map_err(|error| format!("flush {}: {error}", path.display()))?;
    Ok(())
}
