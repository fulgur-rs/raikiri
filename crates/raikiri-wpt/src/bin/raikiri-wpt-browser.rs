//! Screenshot process used by the external upstream wptrunner product.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    raikiri_wpt::wptrunner_browser::entrypoint(env::args().skip(1))
}
