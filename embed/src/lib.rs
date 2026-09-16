//! Carrying the chrome's files into a binary.
//!
//! GTK has GResource for this and the other platforms have nothing like it,
//! so their builds write the files into a table the binary embeds. The list
//! comes from the same GResource manifest the GTK build uses, so no platform
//! can quietly grow or lose a file the others have never heard of.
//!
//! This is a build-time helper: shells call it from their `build.rs`.

use std::path::{Path, PathBuf};
use std::{env, fs};

/// Write `files.rs` into `OUT_DIR`, declaring every chrome file as
/// `pub static FILES: &[(&str, &[u8])]`, keyed by the address it is served at.
pub fn write_file_table(ui: &Path) {
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
/// `chrome/x.js`, read from `dist/chrome/x.js`. Without an alias, both are the
/// same path.
fn entry(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("<file")?;
    let (attributes, rest) = rest.split_once('>')?;
    let from = rest.strip_suffix("</file>")?.trim().to_string();
    let served = match attributes.split_once("alias=\"") {
        Some((_, tail)) => tail.split('"').next()?.to_string(),
        None => from.clone(),
    };
    Some((served, from))
}

#[cfg(test)]
mod tests {
    use super::entry;

    #[test]
    fn an_alias_says_where_a_file_is_served() {
        assert_eq!(
            entry(r#"    <file alias="home/home.js">dist/home/home.js</file>"#),
            Some(("home/home.js".into(), "dist/home/home.js".into()))
        );
        assert_eq!(
            entry("    <file>home/index.html</file>"),
            Some(("home/index.html".into(), "home/index.html".into()))
        );
        assert_eq!(entry("  <gresource prefix=\"/x\">"), None);
    }
}
