// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Build glue for Tauri.
//
// `webview2-com-sys` picks its WebView2 loader with
//
//     #[cfg_attr(target_env = "msvc", link(name = "WebView2LoaderStatic", kind = "static"))]
//     #[cfg_attr(not(target_env = "msvc"), link(name = "WebView2Loader.dll"))]
//
// An MSVC build therefore statically links the loader, while a **mingw** build
// imports `WebView2Loader.dll` at run time. WebView2's own runtime installation does
// not put that DLL on the search path, so a mingw-built exe dies during load with
// STATUS_DLL_NOT_FOUND (0xC0000135) — before `main`, with no window and no log. That
// failure is silent when launched from Explorer, which makes it look like the app
// "does nothing".
//
// Static linking is not an option here: the vendor's `WebView2LoaderStatic.lib` is a
// `ar` archive of **MSVC C++ objects** (`??0SimpleString@...` mangling), which the
// GNU toolchain cannot link.
//
// So the loader must ship beside the exe. This script copies it from the vendored
// crate into the output directory, which is where cargo puts the binary.

use std::path::PathBuf;

fn main() {
    tauri_build::build();
    stage_webview2_loader();
}

/// Copy `WebView2Loader.dll` next to the built binary for non-MSVC targets.
fn stage_webview2_loader() {
    if std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() != "gnu" {
        return;
    }

    let Some(dll) = find_loader_dll() else {
        println!(
            "cargo:warning=WebView2Loader.dll not found in the registry cache; \
             copy it beside the exe manually or the app will not start"
        );
        return;
    };

    // OUT_DIR is target/<triple>/<profile>/build/<pkg>/out; the binary lands three
    // levels up, in target/<triple>/<profile>/.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        println!("cargo:warning=could not resolve the profile directory from OUT_DIR");
        return;
    };

    let dest = profile_dir.join("WebView2Loader.dll");
    match std::fs::copy(&dll, &dest) {
        Ok(_) => {
            println!("cargo:warning=staged WebView2Loader.dll next to the binary");
            println!("cargo:rerun-if-changed={}", dll.display());
        }
        Err(e) => println!("cargo:warning=could not copy WebView2Loader.dll: {e}"),
    }
}

/// Find `WebView2Loader.dll` inside the cargo registry copy of `webview2-com-sys`.
///
/// It ships under a per-architecture directory (`x64`, `x86`, `arm64`), so the search
/// is by directory name rather than by a hard-coded path: the registry hash changes
/// with every mirror and toolchain, which makes any absolute path stale on the next
/// machine.
fn find_loader_dll() -> Option<PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok().map(|h| PathBuf::from(h).join(".cargo")))
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".cargo")))?;

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };

    for registry in std::fs::read_dir(home.join("registry").join("src")).ok()?.flatten() {
        let Ok(entries) = std::fs::read_dir(registry.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("webview2-com-sys-") {
                continue;
            }
            let candidate = entry.path().join(arch).join("WebView2Loader.dll");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}
