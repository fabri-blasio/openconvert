fn main() {
    let url = std::env::args().nth(1).expect("need <url>");
    let mut seen = 0u64;
    let mut ticks = 0u32;
    match openconvert_worker::net::fetch_with_progress(&url, 200 << 20, &mut |got, total| {
        seen = got;
        ticks += 1;
        if ticks.is_multiple_of(40) {
            println!("  {got} / {total}");
        }
    }) {
        Ok(b) => println!("ok: {} bytes, {ticks} progress ticks", b.len()),
        Err(e) => println!("FAILED: {e}"),
    }
}
