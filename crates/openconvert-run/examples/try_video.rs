//! Manual check: transcode a video through the module.
//!
//! `cargo run -p openconvert-run --example try_video -- <in> <webm|mkv|mp4> <out>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let input = std::fs::read(a.next().ok_or("need <in>")?)?;
    let to = match a.next().ok_or("need a target")?.as_str() {
        "webm" => openconvert_core::format::FormatId::Webm,
        "mkv" => openconvert_core::format::FormatId::Mkv,
        "mp4" => openconvert_core::format::FormatId::Mp4,
        other => return Err(format!("unknown target {other}").into()),
    };
    let out_path = a.next().ok_or("need <out>")?;
    println!("module: {:?}", openconvert_run::video::module_path());
    let limits = openconvert_core::policy::Policy::default().base_limits();
    let t = std::time::Instant::now();
    let out = openconvert_run::video::transcode(&input, to, &limits)?;
    println!("ok: {} bytes in {:?}", out.len(), t.elapsed());
    std::fs::write(out_path, &out)?;
    Ok(())
}
