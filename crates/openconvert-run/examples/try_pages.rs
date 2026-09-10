//! Manual check: how many pages does the worker say a PDF has?
//!
//! `cargo run --release -p openconvert-run --example try_pages -- <file.pdf>`
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(std::env::args().nth(1).ok_or("need <file.pdf>")?);
    let policy = openconvert_core::policy::Policy::default();
    let mut table = openconvert_run::handles::HandleTable::new();
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());

    let facts = openconvert_run::detect::detect(&path, &mut table)?;
    let bytes = openconvert_run::detect::bytes_of(&facts, &mut table)?;
    let limits = policy.base_limits();

    let (out, _, _) = pool.run(
        openconvert_sandbox::argv::EngineBin::Pdf,
        facts.provenance(),
        &limits,
        bytes,
        "txt",
        &[("op".to_string(), "pages".to_string())],
    )?;
    println!("pages: {}", String::from_utf8_lossy(&out).trim());
    Ok(())
}
