//! The chrome's files, carried along with the binary.

use std::path::PathBuf;

fn main() {
    let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
    glimmerwood_embed::write_file_table(&ui);
}
