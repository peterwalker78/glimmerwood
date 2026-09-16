//! Carry the chrome's files into the binary.
//!
//! GTK has GResource for this. Windows has nothing like it, so the files are
//! written into a table the binary embeds. The list comes from the same
//! GResource manifest the GTK build uses, so neither can quietly grow a file
//! the other has never heard of.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
    let manifest = ui.join("glimmerwood.gresource.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let xml = fs::read_to_string(&manifest).expect("the chrome's resource list is readable");

    let mut table = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for line in xml.lines() {
        let Some((served, from)) = entry(line) else {
            continue;
        };
        let path = ui.join(&from);
        assert!(
            path.exists(),
            "{} is missing; build the chrome first with scripts/build-ui",
            path.display()
        );
        println!("cargo:rerun-if-changed={}", path.display());
        table.push_str(&format!(
            "    ({served:?}, include_bytes!({:?})),\n",
            path.canonicalize().unwrap_or(path),
        ));
    }
    table.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out_dir.join("files.rs"), table).expect("write the file table");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
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

/// `<file alias="chrome/x.js">dist/chrome/x.js</file>` → served as
/// `chrome/x.js`, read from `dist/chrome/x.js`. Without an alias, both are
/// the same path.
fn entry(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let rest = line.strip_prefix("<file")?;
    let (attributes, rest) = rest.split_once('>')?;
    let from = rest.strip_suffix("</file>")?.trim().to_string();
    let served = match attributes.split_once("alias=\"") {
        Some((_, tail)) => tail.split('"').next()?.to_string(),
        None => from.clone(),
    };
    Some((served, from))
}
