//! Records the version of the compiler cargo builds this tool with, which
//! `raikiri-layout-diff compiler` prints, and finds out how the DOM crate it is
//! built against lets a reader get at the laid-out box of a node.

use std::path::{Path, PathBuf};
use std::process::Command;

// cov:ignore: build script; cargo runs it while building, outside the coverage run, and the `compiler` test checks what it records
fn main() {
    // Cargo gives build scripts the compiler it resolved, from `RUSTC`,
    // `build.rustc` or the default, as `RUSTC`.
    let rustc = std::env::var_os("RUSTC").expect("cargo sets RUSTC for build scripts");
    let output = Command::new(&rustc)
        .arg("-vV")
        .output()
        .expect("run the compiler for its version");
    assert!(
        output.status.success(),
        "{} -vV failed",
        Path::new(&rustc).display()
    );
    let out_dir = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR for build scripts");
    std::fs::write(Path::new(&out_dir).join("compiler"), output.stdout)
        .expect("write the compiler version");
    // A new compiler rebuilds and reruns this script; nothing else changes
    // what it records.
    println!("cargo::rerun-if-changed=build.rs");

    // `src/node_layout.rs` reads a node's box one way in trees where `Node`
    // has a `layout()` accessor and another way in the trees before it.
    println!("cargo::rustc-check-cfg=cfg(node_layout_accessor)");
    let node_source = dom_source_dir().join("src/node.rs");
    println!("cargo::rerun-if-changed={}", node_source.display());
    let source = std::fs::read_to_string(&node_source)
        .unwrap_or_else(|error| panic!("read {}: {error}", node_source.display()));
    if source.contains("pub fn layout(&self)") {
        println!("cargo::rustc-cfg=node_layout_accessor");
    }
}

/// The directory of the `raikiri-dom` crate this tool is built against.
///
/// `scripts/layout-diff.sh` builds the sources with a generated manifest that
/// names the crate by path, whichever tree it comes from. Inside the
/// workspace the crate is the sibling directory.
// cov:ignore: build script; cargo runs it while building, outside the coverage run
fn dom_source_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    let manifest = std::fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("read this crate's manifest");
    manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("raikiri-dom"))
        .find_map(|line| {
            let path = line.split("path = \"").nth(1)?;
            Some(PathBuf::from(path.split('"').next()?))
        })
        .unwrap_or_else(|| manifest_dir.join("../raikiri-dom"))
}
