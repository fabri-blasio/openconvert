//! Manual check: does an execution provider engage, and does it survive the
//! confinement the workers actually run under?
//!
//! ```text
//! cargo run --release -p oc-ai --features directml --example try_gpu -- <model.onnx>
//! cargo run --release -p oc-ai --features directml --example try_gpu -- <model.onnx> confined
//! ```
//!
//! # Why this exists before any GPU code is trusted
//!
//! Two claims have to hold before "use the GPU when one is present" is a
//! feature rather than a wish, and the second is the doubtful one.
//!
//! 1. **The runtime can do it.** The pinned `onnxruntime-win-x64` is a
//!    CPU-only build; DirectML lives in a different artifact. Without
//!    `error_on_failure`, asking a CPU-only runtime for DirectML *succeeds*
//!    and runs on the CPU — so a build can "support the GPU" and never use
//!    one, with nothing anywhere saying so.
//!
//! 2. **The confinement permits it.** Workers are spawned into an
//!    AppContainer with `PROHIBIT_DYNAMIC_CODE_ALWAYS_ON` — ACG — and a GPU
//!    driver compiles shaders at run time. Dynamic code generation is exactly
//!    what that mitigation forbids, so this is a direct conflict between two
//!    things the product wants, not a tuning problem.
//!
//! `confined` re-launches this binary through `spawn_piped_in_container`,
//! which is the same call the worker pool makes, so the answer comes from the
//! real policy rather than a guess about it. **Plain `spawn_piped` is not a
//! substitute**: ACG is requested only on the container path, so a child
//! spawned without one reports `ACG: false` and proves nothing.

#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().ok_or("need <model.onnx>")?;
    if args.next().as_deref() == Some("confined") {
        return confined(&model_path);
    }

    let back = openconvert_os::readback::read_back(&["ACG", "CIG", "AppContainer"]);
    println!("runtime : {}", oc_ai::runtime_version()?);
    println!(
        "ACG     : {}   AppContainer: {}",
        back.engaged("ACG"),
        back.engaged("AppContainer")
    );

    let model = std::fs::read(&model_path)?;
    for (name, provider) in oc_ai::accelerator::candidates() {
        let t = std::time::Instant::now();
        match oc_ai::accelerator::try_session(&model, provider) {
            Ok(()) => println!("  {name:<9} OK       session built in {:?}", t.elapsed()),
            Err(e) => println!("  {name:<9} REFUSED  {e}"),
        }
    }
    Ok(())
}

/// Run this same binary the way the pool runs a worker: inside an
/// AppContainer, with ACG requested.
#[cfg(all(windows, has_ort))]
fn confined(model_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use openconvert_os::appcontainer::AppContainer;
    use std::io::Read;

    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().ok_or("the example has no directory")?;

    // The container must be able to read the model, and it cannot see the
    // user's app-data directory. Copy it somewhere we are willing to ACL.
    let staging = std::env::temp_dir().join(format!("tx-gpu-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let staged_model = staging.join("model.onnx");
    std::fs::copy(model_path, &staged_model)?;

    let container = AppContainer::create("openconvert-gpu-probe")?;
    // Without this the spawn fails with ACCESS_DENIED on a binary that is
    // plainly there -- the container cannot read what it was not granted.
    container.grant_execute(&exe)?;
    container.acl_directory(exe_dir)?; // the runtime DLLs beside it
    container.acl_directory(&staging)?;

    let mut child = openconvert_os::spawn::spawn_piped_in_container(
        &exe,
        &[&staged_model.to_string_lossy()],
        &container,
    )?;
    let mut text = String::new();
    if let Some(out) = child.stdout() {
        out.read_to_string(&mut text)?;
    }
    let code = child.wait()?;
    println!("--- child, confined exactly as a worker is (exit {code}) ---");
    print!("{text}");
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
            .into(),
    )
}
