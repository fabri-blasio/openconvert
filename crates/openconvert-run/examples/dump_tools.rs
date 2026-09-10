//! What the Tools menu is actually offered.
//!
//! `cargo run -p openconvert-run --example dump_tools`

fn main() {
    for t in openconvert_run::tools::list_tools() {
        println!(
            "{:<26} {:<8} available={:<5} preview_only={:<5} params={} {}",
            t.id,
            format!("{:?}", t.category),
            t.available,
            t.preview_only,
            t.params.len(),
            t.unavailable_reason.unwrap_or_default()
        );
    }
}
