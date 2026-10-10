//! Records the version of the compiler cargo builds this tool with, which
//! `raikiri-cascade-diff compiler` prints.

use std::path::Path;
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
}
