#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// Minimum acceptable Zig version. Ghostty's `minimum_zig_version` is 0.15.2
/// and Ghostty has not yet migrated to 0.16 (upstream issue #12228).
const MINIMUM_ZIG_VERSION: (u32, u32, u32) = (0, 15, 2);
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
        // Zig 0.15.2 resolves the native build runner SDK through xcrun before
        // build.zig arguments are evaluated. Put a local xcrun shim first on PATH
        // so the runner and the actual Ghostty target use the same compatible SDK.
        if let Some(shim_dir) = write_xcrun_sdk_shim(sdk) {
            prepend_path(&mut zig_build, &shim_dir);
        }
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
        println!(
            "cargo:rustc-link-search=native={}",
            macos_sdk
                .as_ref()
                .map(|p| p.join("usr/lib"))
                .unwrap_or_default()
                .display()
        );
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
    let explicit = env::var_os("SDKROOT")
        .map(PathBuf::from)
        .filter(|path| path.exists());
    if let Some(sdk) = explicit {
        if sdk_is_zig_compatible(&sdk) {
            return Some(sdk);
        }
        println!(
            "cargo:warning=lmux-libghostty: ignoring SDKROOT={} because Zig 0.15.2 cannot link against its macOS TBD slices",
            sdk.display()
        );
    }

    let xcrun_sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|path| path.exists());
    if let Some(sdk) = xcrun_sdk {
        if sdk_is_zig_compatible(&sdk) {
            return Some(sdk);
        }
        println!(
            "cargo:warning=lmux-libghostty: xcrun selected {}, but Zig 0.15.2 cannot link against its macOS TBD slices",
            sdk.display()
        );
    }

    let fallback = find_compatible_command_line_tools_sdk();
    if let Some(sdk) = &fallback {
        println!(
            "cargo:warning=lmux-libghostty: using compatible CommandLineTools SDK {}",
            sdk.display()
        );
    }
    fallback
}

fn sdk_is_zig_compatible(sdk: &Path) -> bool {
    if !sdk.join("usr/lib").is_dir() {
        return false;
    }

    for rel in [
        "usr/lib/libSystem.tbd",
        "usr/lib/system/libdispatch.tbd",
        "usr/lib/system/libsystem_c.tbd",
    ] {
        let tbd = sdk.join(rel);
        let Ok(contents) = fs::read_to_string(&tbd) else {
            continue;
        };
        if contents.contains("arm64e-macos") && !contents.contains("arm64-macos") {
            return false;
        }
    }
    true
}

fn find_compatible_command_line_tools_sdk() -> Option<PathBuf> {
    let sdk_root = Path::new("/Library/Developer/CommandLineTools/SDKs");
    let mut candidates: Vec<PathBuf> = fs::read_dir(sdk_root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("MacOSX") && name.ends_with(".sdk"))
                && sdk_is_zig_compatible(path)
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

fn write_xcrun_sdk_shim(sdk: &Path) -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR")?);
    let shim_dir = out_dir.join("xcrun-sdk-shim");
    fs::create_dir_all(&shim_dir).ok()?;
    let shim = shim_dir.join("xcrun");
    fs::write(
        &shim,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${{1:-}}" == "--sdk" && "${{2:-}}" == "macosx" && "${{3:-}}" == "--show-sdk-path" ]]; then
  printf '%s\n' "{}"
  exit 0
fi

exec /usr/bin/xcrun "$@"
"#,
            sdk.display()
        ),
    )
    .ok()?;
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).ok()?;
    Some(shim_dir)
}

fn prepend_path(command: &mut Command, dir: &Path) {
    let old_path = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(env::split_paths(&old_path));
    if let Ok(joined) = env::join_paths(paths) {
        command.env("PATH", joined);
    }
}

fn verify_zig_version(zig: &std::ffi::OsStr) {
    let (min_major, min_minor, min_patch) = MINIMUM_ZIG_VERSION;
    let min = format!("{min_major}.{min_minor}.{min_patch}");
    let output = Command::new(zig).arg("version").output().unwrap_or_else(|err| {
        panic!(
            "failed to run `{} version`: {err}. Run `mise install` or set ZIG=/path/to/zig (>= {min})",
            PathBuf::from(zig).display()
        )
    });
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.trim();
    let meets_minimum =
        output.status.success() && parse_version(version).is_some_and(|v| v >= MINIMUM_ZIG_VERSION);
    assert!(
        meets_minimum,
        "vendor-ghostty requires Zig >= {min}, got {} from `{}`. Run `mise install` or set ZIG=/path/to/zig (>= {min}).",
        if version.is_empty() { "unknown" } else { version },
        PathBuf::from(zig).display()
    );
}

/// Parse the leading `MAJOR.MINOR.PATCH` of a `zig version` string. Zig dev
/// builds suffix `-dev.NNNN+sha`, which we ignore here.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').filter_map(|n| n.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);
    Some((major, minor, patch))
}
