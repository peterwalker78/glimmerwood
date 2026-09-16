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

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("files.rs");
    fs::write(&out, table).expect("write the file table");
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
    // The app icon rides along beside Home; everything else is under ui/.
    let _ = Path::new(&from);
    Some((served, from))
}
