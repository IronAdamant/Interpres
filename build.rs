//! Compile native macOS AppKit UI with system clang — no crates.io build deps.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("apple-darwin") && !target.contains("apple-ios") {
        // Still watch native sources so edits re-trigger on Mac builds.
        println!("cargo:rerun-if-changed=native/macos/interpres_gui.m");
        println!("cargo:rerun-if-changed=native/macos/interpres_gui.h");
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = manifest.join("native/macos/interpres_gui.m");
    let obj = out_dir.join("interpres_gui.o");

    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed=native/macos/interpres_gui.h");

    let status = Command::new("clang")
        .args([
            "-fobjc-arc",
            "-fmodules",
            "-O2",
            "-c",
            "-o",
        ])
        .arg(&obj)
        .arg(&src)
        .arg("-I")
        .arg(manifest.join("native/macos"))
        .status()
        .expect("failed to spawn clang — install Xcode Command Line Tools");

    if !status.success() {
        panic!("clang failed to compile native/macos/interpres_gui.m");
    }

    // Link the object into the final binary.
    println!("cargo:rustc-link-arg={}", obj.display());
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Cocoa");
}
