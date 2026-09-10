//! Manual check: a model tool through `run_tool`, the way the desktop calls it.
//!
//! `cargo run --release -p openconvert-run --example try_tool -- <tool-id> <file>`
//!
//! This is the layer nothing exercised. The adapter examples in `oc-ai` call
//! the inference functions directly and skip detection, routing and the auto
//! class ceiling entirely -- so they passed while every one of these tools
//! failed in the product.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(profile) = std::env::current_exe()?
        .parent()
        .and_then(std::path::Path::parent)
    {
        openconvert_run::engine_dir::set_resource_dir(profile.to_path_buf());
    }
    let mut a = std::env::args().skip(1);
    let id = a.next().ok_or("need <tool-id>")?;
    let paths: Vec<std::path::PathBuf> = a.map(std::path::PathBuf::from).collect();
    if paths.is_empty() {
        return Err("need at least one <file>".into());
    }

    // Exactly what the desktop builds: no arming, straight from the config.
    let policy = openconvert_core::policy::Policy::default();

    let t = std::time::Instant::now();
    // `key=value` among the arguments becomes a parameter, so composites can be
    // driven from here too. Bound rather than inlined into the match: a block
    // as a scrutinee reads as though the match is over the block.
    let outcome = {
        // `ID=key=value` on the front of the path list becomes a parameter,
        // so composites can be driven from here too.
        let mut params: Vec<(String, String)> = Vec::new();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for p in &paths {
            let text = p.to_string_lossy().into_owned();
            match text.split_once('=') {
                Some((k, v)) if !std::path::Path::new(&text).exists() => {
                    params.push((k.to_string(), v.to_string()));
                }
                _ => files.push(p.clone()),
            }
        }
        openconvert_run::worker_client::with_progress(
            |done, total| eprintln!("progress: {done}/{total}"),
            || openconvert_run::tools::run_tool(&id, &files, &params, &policy),
        )
    };
    match outcome {
        Ok(run) => {
            let size = std::fs::metadata(&run.output).map(|m| m.len()).unwrap_or(0);
            println!(
                "ok: {} ({size} bytes) in {:?}",
                run.output.display(),
                t.elapsed()
            );
        }
        Err(e) => println!("FAILED: {e}"),
    }
    Ok(())
}
