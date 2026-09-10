//! **Every tool the interface offers can actually save.**
//!
//! # The defect this exists for
//!
//! The PDF workspace sent **every** save as `pdf-compose`, whatever tool was
//! lit. Compose performs three things — merge, reorder, and stamp a signature
//! — so Split, Password and Compress each arrived with none of its parameters
//! set and compose, asked to do nothing it recognises, wrote nothing.
//!
//! That was survivable while the rail held only the three tools compose knows.
//! Advertising Split and Password put two more through the same funnel, and
//! Compress had been going through it all along.
//!
//! # Why the existing tests did not catch it
//!
//! `every_tool_writes_a_file` calls each tool **by its own id** with the right
//! parameters, and every one of them passes — the engines were never broken.
//! `advertised_options_run` checks that an offered option is one the tool will
//! take. Neither asks the question that mattered: *for each tool the interface
//! OFFERS, is there a request that produces a file?*
//!
//! A tool that cannot be made to write anything is a tool whose rail entry is
//! decoration, whatever the engine underneath can do.
//!
//! # What it does, and what it deliberately does not
//!
//! For every advertised tool, it builds the least request that tool describes
//! — its declared defaults, plus a plausible value for anything required —
//! runs it against a fixture of the right kind, and requires a file on disk
//! that is not empty.
//!
//! **It does not check the content.** That a merge has the right pages and a
//! compression is smaller belongs to the engines' own tests, which are
//! thorough. This is the seam: the catalogue says a tool exists, and this says
//! a file comes out of it.
//!
//! Model-backed tools skip loudly when their weights are absent. A machine
//! without the models is a legitimate machine; a silent skip is not.

use std::path::{Path, PathBuf};

use openconvert_core::policy::Policy;
use openconvert_run::tools::{self, ToolDescriptor, ToolRunOptions};

/// A value for a parameter the tool requires but the interface would supply.
///
/// Only the required ones: anything optional is left to its default, because
/// the default is what the interface sends when a user touches nothing, and
/// that is the path being tested.
fn value_for(tool: &str, param: &str) -> Option<String> {
    Some(
        match (tool, param) {
            ("pdf-reorder", "order") => "2,1",
            ("pdf-extract", "pages") => "1",
            ("pdf-remove", "pages") => "2",
            ("pdf-protect", "password") => "correct horse",
            ("pdf-unlock", "password") => "correct horse",
            ("image-compose", "ops") => "image-invert",
            // A signature needs an image to stamp, and building one here would
            // make this a test of PNG encoding. `pdf-stamp` is covered by
            // `every_tool_writes_a_file`, which has the fixture for it.
            ("pdf-stamp", _) => return None,
            _ => return None,
        }
        .to_string(),
    )
}

/// The request the interface would send for a tool nobody has configured.
fn request(tool: &ToolDescriptor) -> Option<Vec<(String, String)>> {
    let mut args = Vec::new();
    for p in &tool.params {
        if let Some(d) = &p.default {
            args.push((p.id.clone(), d.clone()));
        } else if p.required {
            args.push((p.id.clone(), value_for(&tool.id, &p.id)?));
        }
    }
    Some(args)
}

fn engines_ready() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(profile) = exe.parent().and_then(Path::parent) {
            openconvert_run::engine_dir::set_resource_dir(profile.to_path_buf());
        }
    }
    let present = ["oc-images", "oc-pdf", "oc-audio", "oc-ai"]
        .iter()
        .all(|name| {
            openconvert_run::engine_dir::locate(&format!("{name}{}", std::env::consts::EXE_SUFFIX))
                .is_some()
        });
    if !present {
        eprintln!(
            "SKIPPED: the full engine set is not built. Run `cargo build --workspace` first."
        );
    }
    present
}

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openconvert-saves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace");
    dir
}

#[test]
fn every_advertised_tool_can_be_made_to_save() {
    if !engines_ready() {
        return;
    }
    let dir = workspace();
    let a_pdf = dir.join("doc.pdf");
    std::fs::write(&a_pdf, pdf(4)).expect("pdf fixture");
    let an_image = dir.join("shape.png");
    std::fs::write(&an_image, png()).expect("png fixture");
    let second = dir.join("other.pdf");
    std::fs::write(&second, pdf(2)).expect("second pdf");
    let a_sound = dir.join("tone.wav");
    std::fs::write(&a_sound, wav()).expect("wav fixture");
    // The committed scan, for the one tool whose output depends on the input
    // having words in it.
    let a_page = dir.join("page.png");
    std::fs::write(&a_page, include_bytes!("fixtures/ocr-page.png").as_slice())
        .expect("ocr fixture");

    let mut silent: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut checked = 0_usize;

    for tool in tools::list_tools() {
        // A TOOL THAT WRITES NOTHING BY DESIGN IS NOT A TOOL THAT CANNOT
        // SAVE. The colour picker reads a pixel out of the preview and says
        // so in its descriptor; `run_tool` does not even dispatch it.
        if tool.preview_only {
            continue;
        }
        let Some(args) = request(&tool) else {
            skipped.push(format!("  {} — needs a fixture of its own", tool.id));
            continue;
        };
        let one = match tool.category {
            tools::ToolCategory::Pdf => &a_pdf,
            tools::ToolCategory::Audio | tools::ToolCategory::Video => &a_sound,
            // READ TEXT GETS A PAGE WITH TEXT ON IT. The generic fixture is a
            // gradient, and OCR finding no words in it is OCR working -- but
            // it writes an empty file, which is indistinguishable here from
            // the tool being broken. The committed page is the only way to
            // tell those apart.
            _ if tool.id == "image-ocr" => &a_page,
            _ => &an_image,
        };
        // A MERGE OF ONE DOCUMENT IS NOT A MERGE. `multi_input` is the
        // descriptor saying so, and handing it a single file tests the
        // refusal rather than the tool.
        let inputs: Vec<PathBuf> = if tool.multi_input {
            vec![one.clone(), second.clone()]
        } else {
            vec![one.clone()]
        };
        checked += 1;

        let outputs: Vec<PathBuf> = if tools::is_multi_output(&tool.id) {
            match tools::run_multi_tool(
                &tool.id,
                &inputs,
                &args,
                &Policy::default(),
                ToolRunOptions {
                    write_sidecar: false,
                },
            ) {
                Ok(parts) => parts.into_iter().map(|p| p.output).collect(),
                Err(e) => {
                    silent.push(format!("  {} refused: {e}", tool.id));
                    continue;
                }
            }
        } else {
            match tools::run_tool_with(
                &tool.id,
                &inputs,
                &args,
                &Policy::default(),
                ToolRunOptions {
                    write_sidecar: false,
                },
            ) {
                Ok(run) => vec![run.output],
                Err(e) => {
                    // A MODEL THAT IS NOT INSTALLED IS NOT A BROKEN TOOL. The
                    // message names the artifact; the machine is simply
                    // without it, and that is a legitimate machine.
                    let why = format!("{e:?}");
                    if why.contains("not downloaded")
                        || why.contains("NeedsModel")
                        || why.contains("no model")
                        || why.contains("NoRoute")
                    {
                        skipped.push(format!("  {} — model absent: {e}", tool.id));
                        continue;
                    }
                    silent.push(format!("  {} refused: {e}", tool.id));
                    continue;
                }
            }
        };

        if outputs.is_empty() {
            silent.push(format!("  {} reported success and named no file", tool.id));
            continue;
        }
        for out in outputs {
            match std::fs::metadata(&out) {
                Err(e) => silent.push(format!(
                    "  {} named {} and it is not there: {e}",
                    tool.id,
                    out.display()
                )),
                // AN EMPTY FILE IS A FAILURE, EXCEPT WHERE THE FIXTURE
                // CANNOT CARRY WHAT THE TOOL LOOKS FOR.
                //
                // Transcription over a 440 Hz tone finds no speech, and the
                // voice-activity check in front of it is built to skip
                // exactly that -- so an empty transcript is the tool working.
                // Synthesising speech to prove otherwise is not something a
                // test suite should be doing.
                //
                // The file still has to EXIST, which is the failure that
                // actually reaches users, and the exemption is printed rather
                // than silent so nobody reads a pass as full coverage.
                Ok(m) if m.len() == 0 => {
                    if tool.id == "audio-transcribe" {
                        skipped.push(format!(
                            "  {} — wrote an empty transcript; the fixture is a tone, not speech",
                            tool.id
                        ));
                    } else {
                        silent.push(format!(
                            "  {} wrote {} and it is empty",
                            tool.id,
                            out.display()
                        ));
                    }
                }
                Ok(_) => {}
            }
        }
    }

    for line in &skipped {
        eprintln!("SKIPPED:{line}");
    }
    assert!(
        checked > 0,
        "no advertised tool was exercised at all; the catalogue shape changed \
         and this test is checking nothing"
    );
    assert!(
        silent.is_empty(),
        "these tools are offered in the interface and cannot be made to save:\n{}",
        silent.join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Fixtures, built here rather than committed.
//
// A committed PDF is repaired quietly by `lopdf` when its xref is wrong, so it
// would prove nothing about honest files; a committed PNG is a binary in a
// diff. Both are cheap to write.
// ---------------------------------------------------------------------------

/// A PDF with `pages` real pages, a real content stream and a real xref.
fn pdf(pages: usize) -> Vec<u8> {
    let kids: String = (0..pages)
        .map(|i| format!("{} 0 R", 3 + 2 * i))
        .collect::<Vec<_>>()
        .join(" ");
    let font = 3 + 2 * pages;
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    ];
    for i in 0..pages {
        let content = format!(
            "BT\n/F1 24 Tf\n1 0 0 1 72 700 Tm\n(Page {}) Tj\nET\n",
            i + 1
        );
        bodies.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {} 0 R >>",
            4 + 2 * i
        ));
        bodies.push(format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ));
    }
    bodies.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    let mut out = Vec::from(b"%PDF-1.4\n");
    let mut offsets = vec![0_u32; bodies.len() + 1];
    for (i, body) in bodies.iter().enumerate() {
        let num = i + 1;
        offsets[num] = u32::try_from(out.len()).unwrap_or(0);
        out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for slot in &offsets[1..] {
        out.extend_from_slice(format!("{slot:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out.extend_from_slice(format!("{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// A 96x96 RGBA PNG with structure, so a compressor has something to do.
fn png() -> Vec<u8> {
    let (w, h) = (96_u32, 96_u32);
    let mut rows = Vec::new();
    for y in 0..h {
        rows.push(0_u8); // filter: none
        for x in 0..w {
            rows.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, 140, 255]);
        }
    }
    let chunk = |tag: &[u8], data: &[u8]| {
        let mut c = Vec::new();
        c.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
        c.extend_from_slice(tag);
        c.extend_from_slice(data);
        c.extend_from_slice(&crc32(tag, data).to_be_bytes());
        c
    };
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    // Bytes, not a string literal: the signature is not text, and writing it
    // as one puts a raw 0x89 and a raw 0x1A into this file.
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&chunk(b"IDAT", &deflate(&rows)));
    out.extend_from_slice(&chunk(b"IEND", b""));
    out
}

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

/// Half a second of a 440 Hz tone, 16-bit mono at 16 kHz.
///
/// A real signal rather than silence: the voice-activity check in front of
/// transcription is built to skip silence, so a silent fixture would exercise
/// the skip rather than the tool.
fn wav() -> Vec<u8> {
    const RATE: u32 = 16_000;
    let samples: Vec<i16> = (0..RATE / 2)
        .map(|n| {
            let t = f32::from(u16::try_from(n).unwrap_or(0)) / RATE as f32;
            ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
        })
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = Vec::from(*b"RIFF");
    out.extend_from_slice(&(36 + u32::try_from(data.len()).unwrap_or(0)).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM header length
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2_u16.to_le_bytes()); // block align
    out.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&data);
    out
}
