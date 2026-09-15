//! Bundles the chrome (HTML, CSS and the JavaScript `tsc` produced) into a
//! GResource that the binary embeds.

use std::{env, path::PathBuf, process::Command};

fn main() {
    let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let out = out_dir.join("glimmerwood.gresource");

    // Regenerating ui/protocol.gen.ts runs the tests, which needs the crate
    // to build before the UI can compile against the new types. This builds
    // with an empty bundle for exactly that; scripts/protocol sets it.
    println!("cargo:rerun-if-env-changed=GLIMMERWOOD_WITHOUT_UI");
    if env::var_os("GLIMMERWOOD_WITHOUT_UI").is_some() {
        let empty = out_dir.join("empty.gresource.xml");
        std::fs::write(&empty, "<gresources/>").expect("write empty manifest");
        let status = Command::new("glib-compile-resources")
            .arg(format!("--target={}", out.display()))
            .arg(&empty)
            .status()
            .expect("glib-compile-resources runs");
        assert!(status.success(), "glib-compile-resources failed");
        return;
    }

    let manifest = ui.join("glimmerwood.gresource.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let deps = Command::new("glib-compile-resources")
        .arg(format!("--sourcedir={}", ui.display()))
        .arg("--generate-dependencies")
        .arg(&manifest)
        .output()
        .expect("glib-compile-resources is installed (glib2-devel)");
    for dep in String::from_utf8_lossy(&deps.stdout).lines() {
        let path = ui.join(dep);
        assert!(
            path.exists(),
            "{} is missing; build the UI first with scripts/build-ui",
            path.display()
        );
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let status = Command::new("glib-compile-resources")
        .arg(format!("--sourcedir={}", ui.display()))
        .arg(format!("--target={}", out.display()))
        .arg(&manifest)
        .status()
        .expect("glib-compile-resources runs");
    assert!(status.success(), "glib-compile-resources failed");
}
