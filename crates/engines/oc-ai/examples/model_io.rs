//! Diagnostic: print a model's input and output signatures.
//!
//! `cargo run -p oc-ai --example model_io -- <model.onnx>...`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path)?;
        let session = ort::session::Session::builder()?
            .with_intra_threads(1)?
            .commit_from_memory(&bytes)?;
        println!("== {path}");
        for i in session.inputs() {
            println!("  in  {:<24} {:?}", i.name(), i.dtype());
        }
        for o in session.outputs() {
            println!("  out {:<24} {:?}", o.name(), o.dtype());
        }
    }
    Ok(())
}
