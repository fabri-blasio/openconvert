//! Negative fixture: a path printed straight to the terminal.
//!
//! The gate is "no untrusted path printed raw". This is what it must object
//! to — and the reason it must is that `path` here can hold
//! `invoice\u{202E}fdp.exe`, which the user reads as `invoiceexe.pdf`.
//!
//! Outside the workspace, so an ordinary build never compiles it.

use std::path::Path;

fn wrong(path: &Path) {
    println!("{}", path.display());
}

/// The control, on the line below the violation: this is what the JSON path
/// does, and the gate must **not** fire on it. A serde encoder escapes control
/// characters itself, and handing a machine a sanitised name would be handing
/// it a different name than the one on disk.
fn allowed(path: &Path) -> String {
    path.display().to_string()
}

fn main() {
    wrong(Path::new("x"));
    let _ = allowed(Path::new("x"));
}
