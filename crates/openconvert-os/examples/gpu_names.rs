//! Manual check: what this machine's graphics card is called.
//!
//! The unit tests can only assert that whatever is found is *renderable* --
//! they run on machines with no card at all, so they cannot assert a name.
//! This prints the actual answer, which is the only way to see that the
//! registry parsing produced the string a user would recognise rather than a
//! plausible-looking one.
//!
//! ```text
//! cargo run -p openconvert-os --example gpu_names
//! ```
//!
//! On a machine with both a discrete and an integrated adapter, the discrete
//! one must come first: that ordering is what the settings screen names.

fn main() {
    let all = openconvert_os::gpu::adapters();
    if all.is_empty() {
        println!("no display adapters found (expected off Windows)");
        return;
    }
    for a in &all {
        let gib = a.video_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        println!("{:<40} {gib:>6.2} GiB dedicated", a.name);
    }
    println!();
    println!(
        "the one the settings screen names: {}",
        openconvert_os::gpu::primary().map_or_else(|| "none".into(), |a| a.name)
    );
}
