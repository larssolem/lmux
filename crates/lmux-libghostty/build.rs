#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor_dir = manifest_dir.join("vendor-ghostty");
    let zig_out = vendor_dir.join("zig-out");
    let lib_dir = zig_out.join("lib");
    let include_dir = zig_out.join("include");

    println!(
        "cargo:rerun-if-changed={}",
        vendor_dir.join("build.zig").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor_dir.join("build.zig.zon").display()
    );

    let status = Command::new("zig")
        .arg("build")
        .arg("--release=fast")
        .current_dir(&vendor_dir)
        .status()
        .expect("failed to run `zig build` for vendor-ghostty");
    assert!(status.success(), "zig build failed");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=dl");

    let gcc_inc = String::from_utf8(
        Command::new("gcc")
            .args(["-print-file-name=include"])
            .output()
            .expect("gcc -print-file-name=include")
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    let bindings = bindgen::Builder::default()
        .header(include_dir.join("ghostty/vt.h").to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .clang_arg(format!("-isystem{}", gcc_inc))
        .clang_arg("-DGHOSTTY_STATIC")
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .derive_default(true)
        .generate()
        .expect("bindgen failed");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out).expect("write bindings.rs");
}
