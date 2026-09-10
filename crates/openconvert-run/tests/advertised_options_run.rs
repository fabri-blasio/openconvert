//! **Every option a tool advertises must actually run.**
//!
//! # The defect this exists for
//!
//! `image-upscale` offered a **2×** and a **4×** in its parameter spec, and
//! `run_upscale` refused everything except 4:
//!
//! ```text
//! if scale != 4 {
//!     return Err(ToolError::BadParam("this build enlarges by 4x only ..."))
//! }
//! ```
//!
//! So the interface rendered a two-item selector, the user picked the first
//! one, and the tool answered that it does not do that — after the file had
//! been chosen and the button pressed. The refusal is honest and the offer was
//! not.
//!
//! This is the same shape as a route the table declares and the engine refuses,
//! which `every_declared_document_route_runs` holds one layer down. The
//! parameter spec is a second place where the product describes itself, and
//! nothing was checking it against the code that has to honour it.
//!
//! # What it does, and what it deliberately does not
//!
//! For every advertised tool, for every `choice` parameter, for every option:
//! run the tool with that option set and require that it does not come back
//! `BadParam`. **`BadParam` is the only failure this treats as a bug** —
//! everything else is a legitimate answer to a fixture that is not the right
//! kind of file, or to a model that is not installed on this machine.
//!
//! That narrowness is the point. A gate that demanded every tool succeed on a
//! generic PNG would fail for a dozen honest reasons and be switched off within
//! a week.

use std::path::{Path, PathBuf};

use openconvert_core::policy::Policy;
use openconvert_run::tools::{self, ToolError, ToolRunOptions};

/// A 64x64 RGBA PNG, built here so the test carries no fixture.
fn png() -> Vec<u8> {
    let (w, h) = (64_u32, 64_u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0_u8); // filter: none
        for x in 0..w {
            raw.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, 128, 255]);
        }
    }
    let chunk = |tag: &[u8], data: &[u8]| {
        let mut c = Vec::new();
        c.extend_from_slice(&(u32::try_from(data.len()).unwrap_or(0)).to_be_bytes());
        c.extend_from_slice(tag);
        c.extend_from_slice(data);
        c.extend_from_slice(&crc32(tag, data).to_be_bytes());
        c
    };
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    // The signature as bytes, not as a string literal: it is not text, and
    // writing it as one puts a raw 0x89 and a raw 0x1A into this file.
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&chunk(b"IDAT", &deflate(&raw)));
    out.extend_from_slice(&chunk(b"IEND", b""));
    out
}

/// CRC-32 over a chunk's tag and body, computed here so the test needs no
/// dependency of its own.
fn crc32(tag: &[u8], body: &[u8]) -> u32 {
    let mut table = [0_u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFF_u32;
    for b in tag.iter().chain(body) {
        crc = table[((crc ^ u32::from(*b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).expect("deflate");
    e.finish().expect("deflate")
}

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openconvert-options-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    dir
}

fn engines_ready(profile_set: &mut bool) -> bool {
    if !*profile_set {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(profile) = exe.parent().and_then(Path::parent) {
                openconvert_run::engine_dir::set_resource_dir(profile.to_path_buf());
            }
        }
        *profile_set = true;
    }
    let present =
        openconvert_run::engine_dir::locate(&format!("oc-images{}", std::env::consts::EXE_SUFFIX))
            .is_some();
    if !present {
        eprintln!("SKIPPED: the engines are not built. Run `cargo build --workspace` first.");
    }
    present
}

#[test]
fn every_advertised_choice_is_one_the_tool_accepts() {
    let mut profile_set = false;
    if !engines_ready(&mut profile_set) {
        return;
    }
    let dir = workspace();
    let input = dir.join("shape.png");
    std::fs::write(&input, png()).expect("fixture");

    let mut refused: Vec<String> = Vec::new();
    let mut checked = 0_usize;

    for tool in tools::list_tools().iter().filter(|t| t.advertised) {
        for param in &tool.params {
            if param.kind != "choice" {
                continue;
            }
            for option in &param.options {
                checked += 1;
                // THE TOOL'S OTHER REQUIRED PARAMETERS GO IN TOO.
                //
                // This sent the one choice and nothing else, which is fine for
                // a tool whose only parameter is that choice and wrong for any
                // tool that has a required sibling: `pdf-protect` gained a
                // `direction` choice beside its required `password`, and every
                // option of it "failed" with `missing required parameter
                // "password"` — a true statement about the request this test
                // built, and nothing at all about the option.
                //
                // A stand-in value rather than a realistic one: the run is
                // expected to fail somewhere later (wrong file type, absent
                // model), and the only failure this reads is `BadParam`.
                let mut args = vec![(param.id.clone(), option.value.clone())];
                for other in &tool.params {
                    if other.id == param.id || !other.required {
                        continue;
                    }
                    let stand_in = match other.kind.as_str() {
                        "number" | "range" => "1",
                        _ => "x",
                    };
                    args.push((other.id.clone(), stand_in.to_string()));
                }
                let run = tools::run_tool_with(
                    &tool.id,
                    std::slice::from_ref(&input),
                    &args,
                    &Policy::default(),
                    ToolRunOptions {
                        write_sidecar: false,
                    },
                );
                // ONLY `BadParam` IS A BUG HERE. A tool refusing a PNG because
                // it wants a PDF, or because its model is not installed, is
                // answering correctly — the question this asks is whether the
                // option it OFFERS is one it will take.
                //
                // And only a `BadParam` ABOUT THIS PARAMETER. A stand-in of
                // "x" where a tool wanted a page range is a complaint about
                // the fixture, not about the option under test; the message
                // names the parameter it is about, so that is what is read.
                if let Err(ToolError::BadParam(why)) = run {
                    let about_another = tool
                        .params
                        .iter()
                        .any(|p| p.id != param.id && why.contains(&p.id));
                    if !about_another {
                        refused.push(format!(
                            "  {} advertises {}={:?} ({:?}) and refuses it: {why}",
                            tool.id, param.id, option.value, option.label
                        ));
                    }
                }
            }
        }
    }

    assert!(
        checked > 0,
        "no advertised choice options were found at all; the catalogue shape changed \
         and this test is checking nothing"
    );
    assert!(
        refused.is_empty(),
        "these tools offer options they will not accept:\n{}",
        refused.join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
