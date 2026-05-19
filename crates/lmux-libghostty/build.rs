#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::path::PathBuf;
use std::process::Command;

const REQUIRED_ZIG_VERSION: &str = "0.15.2";

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
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
    println!("cargo:rerun-if-env-changed=ZIG");

    let zig = env::var_os("ZIG").unwrap_or_else(|| "zig".into());
    verify_zig_version(&zig);
    let status = Command::new(&zig)
        .arg("build")
        .arg("--release=fast")
        .current_dir(&vendor_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to run `{} build` for vendor-ghostty: {err}",
                PathBuf::from(&zig).display()
            )
        });
    assert!(status.success(), "zig build failed");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    if target_os != "macos" {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=dl");
    }

    let gcc_inc = Command::new("gcc")
        .args(["-print-file-name=include"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut bindings = bindgen::Builder::default()
        .header(include_dir.join("ghostty/vt.h").to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .clang_arg("-DGHOSTTY_STATIC")
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .derive_default(true);
    if let Some(gcc_inc) = gcc_inc {
        bindings = bindings.clang_arg(format!("-isystem{gcc_inc}"));
    }
    let bindings = bindings.generate().expect("bindgen failed");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out).expect("write bindings.rs");
}

fn verify_zig_version(zig: &std::ffi::OsStr) {
    let output = Command::new(zig).arg("version").output().unwrap_or_else(|err| {
        panic!(
            "failed to run `{} version`: {err}. Run `mise install` or set ZIG=/path/to/zig-{REQUIRED_ZIG_VERSION}",
            PathBuf::from(zig).display()
        )
    });
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.trim();
    assert!(
        output.status.success() && version == REQUIRED_ZIG_VERSION,
        "vendor-ghostty requires Zig {REQUIRED_ZIG_VERSION}, got {} from `{}`. Run `mise install` or set ZIG=/path/to/zig-{REQUIRED_ZIG_VERSION}",
        if version.is_empty() { "unknown" } else { version },
        PathBuf::from(zig).display()
    );
}
