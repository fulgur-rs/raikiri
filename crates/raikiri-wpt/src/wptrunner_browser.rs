//! Command implementation for the upstream wptrunner screenshot process.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use raikiri::Url;
use raikiri_net::SystemHttpProvider;

use crate::reftest::{RenderedDocument, RenderedImage};
use crate::{WptHostResolver, render_print_url, render_screen_url};

enum BrowserCommand {
    Screen(ScreenArgs),
    PrintReftest(PrintReftestArgs),
}

impl BrowserCommand {
    fn parse<I, S>(arguments: I) -> Result<Self, BrowserError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut arguments: Vec<String> = arguments.into_iter().map(Into::into).collect();
        if arguments
            .first()
            .is_some_and(|value| value == "print-reftest")
        {
            arguments.remove(0);
            PrintReftestArgs::parse(arguments).map(Self::PrintReftest)
        } else {
            ScreenArgs::parse(arguments).map(Self::Screen)
        }
    }
}

struct ScreenArgs {
    url: Url,
    output: PathBuf,
    viewport: Viewport,
    host_file: PathBuf,
}

impl ScreenArgs {
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

        Ok(Self {
            url: parse_http_url(&required(url, "--url")?)?,
            output: required(output, "--output")?.into(),
            viewport: Viewport::parse(&required(window_size, "--window-size")?, "--window-size")?,
            host_file: required(host_file, "--host-file")?.into(),
        })
    }
}

struct PrintReftestArgs {
    url: Url,
    output_directory: PathBuf,
    page_size: Viewport,
    host_file: PathBuf,
}

impl PrintReftestArgs {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, BrowserError> {
        let mut url = None;
        let mut output_directory = None;
        let mut page_size = None;
        let mut host_file = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| BrowserError::new(format!("missing value for {argument}")))?;
            let slot = match argument.as_str() {
                "--url" => &mut url,
                "--output-directory" => &mut output_directory,
                "--page-size" => &mut page_size,
                "--host-file" => &mut host_file,
                _ => return Err(BrowserError::new(format!("unknown argument: {argument}"))),
            };
            if slot.replace(value).is_some() {
                return Err(BrowserError::new(format!("duplicate argument: {argument}")));
            }
        }

        Ok(Self {
            url: parse_http_url(&required(url, "--url")?)?,
            output_directory: required(output_directory, "--output-directory")?.into(),
            page_size: Viewport::parse(&required(page_size, "--page-size")?, "--page-size")?,
            host_file: required(host_file, "--host-file")?.into(),
        })
    }
}

struct Viewport {
    width: u32,
    height: u32,
}

impl Viewport {
    fn parse(value: &str, option: &str) -> Result<Self, BrowserError> {
        let (width, height) = value.split_once('x').ok_or_else(|| {
            BrowserError::new(format!("invalid {option} {value:?}; expected WIDTHxHEIGHT"))
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

fn parse_http_url(value: &str) -> Result<Url, BrowserError> {
    let url =
        Url::parse(value).map_err(|error| BrowserError::new(format!("invalid --url: {error}")))?;
    if url.scheme() != "http" {
        return Err(BrowserError::new(format!(
            "only http URLs are supported by this prototype, got {url}"
        )));
    }
    Ok(url)
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
    match BrowserCommand::parse(arguments)? {
        BrowserCommand::Screen(args) => {
            let provider = provider_from_hosts_file(&args.host_file)?;
            let image = render_screen_url(
                &provider,
                args.url,
                args.viewport.width,
                args.viewport.height,
            )
            .map_err(|error| BrowserError::new(error.to_string()))?;
            write_png_atomically(&args.output, &image)
        }
        BrowserCommand::PrintReftest(args) => {
            ensure_empty_output_directory(&args.output_directory)?;
            let provider = provider_from_hosts_file(&args.host_file)?;
            let document = render_print_url(
                &provider,
                args.url,
                args.page_size.width,
                args.page_size.height,
            )
            .map_err(|error| BrowserError::new(error.to_string()))?;
            write_pages_atomically(&args.output_directory, &document)
        }
    }
}

fn provider_from_hosts_file(host_file: &Path) -> Result<SystemHttpProvider, BrowserError> {
    let hosts = WptHostResolver::from_hosts_file(host_file)
        .map_err(|error| BrowserError::new(error.to_string()))?;
    Ok(SystemHttpProvider::with_host_overrides(
        hosts.into_overrides(),
    ))
}

fn required(value: Option<String>, name: &str) -> Result<String, BrowserError> {
    value.ok_or_else(|| BrowserError::new(format!("missing required argument {name}")))
}

fn ensure_empty_output_directory(output: &Path) -> Result<(), BrowserError> {
    if !output.is_dir() {
        return Err(BrowserError::new(format!(
            "print output directory does not exist or is not a directory: {}",
            output.display()
        )));
    }
    let mut entries = fs::read_dir(output)
        .map_err(|error| BrowserError::new(format!("read {}: {error}", output.display())))?;
    if entries.next().is_some() {
        return Err(BrowserError::new(format!(
            "print output directory must be empty: {}",
            output.display()
        )));
    }
    Ok(())
}

fn write_pages_atomically(
    output_directory: &Path,
    document: &RenderedDocument,
) -> Result<(), BrowserError> {
    ensure_empty_output_directory(output_directory)?;
    if document.pages.is_empty() {
        return Err(BrowserError::new("print renderer produced no pages".into()));
    }
    for (index, page) in document.pages.iter().enumerate() {
        let output = output_directory.join(format!("page-{:04}.png", index + 1));
        write_png_atomically(&output, page)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests;
