//! Diagnostic: what does symphonia see inside a container?
//!
//! `cargo run -p oc-audio --example probe_tracks -- <file>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("need a file")?;
    let bytes = std::fs::read(&path)?;
    let mss = symphonia::core::io::MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes)),
        Default::default(),
    );
    match symphonia::default::get_probe().format(
        &symphonia::core::probe::Hint::new(),
        mss,
        &Default::default(),
        &Default::default(),
    ) {
        Ok(p) => {
            for t in p.format.tracks() {
                let c = &t.codec_params;
                println!(
                    "id={} codec={:?} rate={:?} ch={:?}",
                    t.id, c.codec, c.sample_rate, c.channels
                );
                match symphonia::default::get_codecs().make(c, &Default::default()) {
                    Ok(_) => println!("  decoder: OK"),
                    Err(e) => println!("  decoder ERROR: {e}"),
                }
            }
        }
        Err(e) => println!("probe failed: {e}"),
    }
    Ok(())
}
