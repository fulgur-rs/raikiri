//! Command implementation for the upstream wptrunner screenshot process.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use raikiri::Url;
use raikiri_net::SystemHttpProvider;

use crate::reftest::RenderedImage;
use crate::{WptHostResolver, render_screen_url};

struct BrowserArgs {
    url: Url,
    output: PathBuf,
    viewport: Viewport,
    host_file: PathBuf,
}

impl BrowserArgs {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, BrowserError> {
        let mut url = None;
        let mut output = None;
        let mut window_size = None;
        let mut host_file = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| BrowserError::new(format!("missing value for {argument}")))?;
            let slot = match argument.as_str() {
                "--url" => &mut url,
                "--output" => &mut output,
                "--window-size" => &mut window_size,
                "--host-file" => &mut host_file,
                _ => return Err(BrowserError::new(format!("unknown argument: {argument}"))),
            };
            if slot.replace(value).is_some() {
                return Err(BrowserError::new(format!("duplicate argument: {argument}")));
            }
        }

        let url_text = required(url, "--url")?;
        let url = Url::parse(&url_text)
            .map_err(|error| BrowserError::new(format!("invalid --url: {error}")))?;
        if url.scheme() != "http" {
            return Err(BrowserError::new(format!(
                "only http URLs are supported by this prototype, got {url}"
            )));
        }

        Ok(Self {
            url,
            output: required(output, "--output")?.into(),
            viewport: Viewport::parse(&required(window_size, "--window-size")?)?,
            host_file: required(host_file, "--host-file")?.into(),
        })
    }
}

struct Viewport {
    width: u32,
    height: u32,
}

impl Viewport {
    fn parse(value: &str) -> Result<Self, BrowserError> {
        let (width, height) = value.split_once('x').ok_or_else(|| {
            BrowserError::new(format!(
                "invalid --window-size {value:?}; expected WIDTHxHEIGHT"
            ))
        })?;
        let width = width.parse::<u32>().map_err(|error| {
            BrowserError::new(format!("invalid viewport width {width:?}: {error}"))
        })?;
        let height = height.parse::<u32>().map_err(|error| {
            BrowserError::new(format!("invalid viewport height {height:?}: {error}"))
        })?;
        if width == 0 || height == 0 {
            return Err(BrowserError::new(
                "viewport dimensions must be positive".into(),
            ));
        }
        Ok(Self { width, height })
    }
}

/// Runs the command and translates its result to process output and an exit code.
pub fn entrypoint(arguments: impl IntoIterator<Item = String>) -> ExitCode {
    match run(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("raikiri-wpt-browser: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: impl IntoIterator<Item = String>) -> Result<(), BrowserError> {
    let args = BrowserArgs::parse(arguments)?;
    let hosts = WptHostResolver::from_hosts_file(&args.host_file)
        .map_err(|error| BrowserError::new(error.to_string()))?;
    let provider = SystemHttpProvider::with_host_overrides(hosts.into_overrides());
    let image = render_screen_url(
        &provider,
        args.url,
        args.viewport.width,
        args.viewport.height,
    )
    .map_err(|error| BrowserError::new(error.to_string()))?;
    write_png_atomically(&args.output, &image)
}

fn required(value: Option<String>, name: &str) -> Result<String, BrowserError> {
    value.ok_or_else(|| BrowserError::new(format!("missing required argument {name}")))
}

fn write_png_atomically(output: &Path, image: &RenderedImage) -> Result<(), BrowserError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = output.file_name().ok_or_else(|| {
        BrowserError::new(format!(
            "output path has no file name: {}",
            output.display()
        ))
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let result = write_png(&temporary, image).and_then(|()| {
        fs::rename(&temporary, output).map_err(|error| {
            BrowserError::new(format!(
                "rename {} to {}: {error}",
                temporary.display(),
                output.display()
            ))
        })
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_png(path: &Path, image: &RenderedImage) -> Result<(), BrowserError> {
    let file = fs::File::create(path)
        .map_err(|error| BrowserError::new(format!("{}: {error}", path.display())))?;
    let mut output = io::BufWriter::new(file);
    {
        let mut encoder = png::Encoder::new(&mut output, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| BrowserError::new(format!("PNG header: {error}")))?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|error| BrowserError::new(format!("PNG data: {error}")))?;
        writer
            .finish()
            .map_err(|error| BrowserError::new(format!("PNG finish: {error}")))?;
    }
    output
        .flush()
        .map_err(|error| BrowserError::new(format!("flush {}: {error}", path.display())))?;
    Ok(())
}

#[derive(Debug)]
struct BrowserError(String);

impl BrowserError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BrowserError {}
