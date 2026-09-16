//! The chrome's files, and WebView2's loader, carried along with the binary.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
    glimmerwood_embed::write_file_table(&ui);

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
        put_the_loader_beside_the_binary(&out_dir);
    }
}

/// WebView2's loader is a small redistributable DLL that has to sit next to
/// the program: it is what finds the Edge runtime on the machine. It comes
/// with the bindings, so it is copied out rather than asked of anyone.
fn put_the_loader_beside_the_binary(out_dir: &Path) {
    const LOADER: &str = "WebView2Loader.dll";
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => return,
    };
    // OUT_DIR is <target>/<profile>/build/<crate>-<hash>/out, and the binary
    // is three levels above it. The bindings' own build script leaves the
    // loaders in its OUT_DIR, which is a sibling of ours.
    let Some(profile) = out_dir.ancestors().nth(3) else {
        return;
    };
    let Ok(builds) = fs::read_dir(profile.join("build")) else {
        return;
    };
    for entry in builds.flatten() {
        let candidate = entry.path().join("out").join(architecture).join(LOADER);
        if candidate.exists() {
            let _ = fs::copy(&candidate, profile.join(LOADER));
            return;
        }
    }
    println!("cargo:warning={LOADER} was not found; WebView2 will not start without it");
}
