#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::path::PathBuf;
use std::process::Command;

const REQUIRED_ZIG_VERSION: &str = "0.15.2";
const MACOS_DEPLOYMENT_TARGET: &str = "14.0";

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
    println!("cargo:rerun-if-env-changed=SDKROOT");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let zig = env::var_os("ZIG").unwrap_or_else(|| "zig".into());
    verify_zig_version(&zig);

    let macos_sdk = (target_os == "macos").then(macos_sdk_path).flatten();
    let mut zig_build = Command::new(&zig);
    zig_build
        .arg("build")
        .arg("--release=fast")
        .current_dir(&vendor_dir);

    if let Some(sdk) = &macos_sdk {
        // GitHub's macOS runners may not expose enough SDK information for Zig to
        // link the build runner against LibSystem automatically. Passing the SDK
        // and deployment target explicitly avoids undefined symbols such as
        // _abort, _dispatch_* and __availability_version_check during `zig build`.
        zig_build
            .env("SDKROOT", sdk)
            .env("MACOSX_DEPLOYMENT_TARGET", deployment_target())
            .env("LIBRARY_PATH", sdk.join("usr/lib"))
            .arg("--sysroot")
            .arg(sdk);
    }

    let status = zig_build.status().unwrap_or_else(|err| {
        panic!(
            "failed to run `{} build` for vendor-ghostty: {err}",
            PathBuf::from(&zig).display()
        )
    });
    assert!(status.success(), "zig build failed");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    if target_os == "macos" {
        println!("cargo:rustc-link-search=native={}", macos_sdk.as_ref().map(|p| p.join("usr/lib")).unwrap_or_default().display());
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=System");
    } else {
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
    if let Some(sdk) = &macos_sdk {
        bindings = bindings
            .clang_arg(format!("-isysroot{}", sdk.display()))
            .clang_arg(format!("-mmacosx-version-min={}", deployment_target()));
    }
    if let Some(gcc_inc) = gcc_inc {
        bindings = bindings.clang_arg(format!("-isystem{gcc_inc}"));
    }
    let bindings = bindings.generate().expect("bindgen failed");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out).expect("write bindings.rs");
}

fn deployment_target() -> String {
    env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| MACOS_DEPLOYMENT_TARGET.to_string())
}

fn macos_sdk_path() -> Option<PathBuf> {
    env::var_os("SDKROOT")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .or_else(|| {
            Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|s| PathBuf::from(s.trim()))
                .filter(|path| path.exists())
        })
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
        "vendor-ghostty requires Zig {REQUIRED_ZIG_VERSION}, got {} from `{}`. Run `mise install` or set ZIG=/path/to/zig-{REQUIRED_ZIG_VERSION}`",
        if version.is_empty() { "unknown" } else { version },
        PathBuf::from(zig).display()
    );
}