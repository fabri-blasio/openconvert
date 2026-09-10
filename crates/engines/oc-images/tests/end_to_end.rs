//! Drive the real worker binary as a real subprocess.
//!
//! Everything else in this project tests the worker runtime **in-process**,
//! through in-memory pipes. This does not: it starts `oc-images` as a separate
//! program and talks to it over the protocol.
//!
//! That distinction is the whole point of the design. `EngineBin` names
//! *programs*, and a closed enum of programs is only meaningful if the programs
//! exist and can be talked to. Until this test existed, that was a claim.

use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};
use std::io::Write;
use std::process::{Command, Stdio};

/// A real PNG, built here so the test needs no fixture file.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode");
    out.into_inner()
}

/// The built `oc-images` binary, next to this test's own executable.
fn worker_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current exe");
    p.pop(); // deps/
    p.pop(); // target/<profile>/
    p.join(format!("oc-images{}", std::env::consts::EXE_SUFFIX))
}

/// Run one session against the real binary and collect every answer with the
/// content that followed it.
fn talk(requests: &[(Request, Vec<u8>)]) -> Vec<(Response, Vec<u8>)> {
    let exe = worker_path();
    assert!(
        exe.exists(),
        "oc-images is not built at {} -- this test needs the binary, not the library",
        exe.display()
    );

    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn oc-images");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        for (r, content) in requests {
            let body = serde_json::to_vec(r).expect("encode");
            write_frame(&mut stdin, &body).expect("write frame");
            write_content(&mut stdin, content).expect("write content");
        }
        stdin.flush().expect("flush");
        // Closing stdin is how a session ends. The worker returns cleanly.
    }

    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "oc-images exited with {:?}",
        out.status.code()
    );

    let mut cursor = std::io::Cursor::new(out.stdout);
    let mut answers: Vec<(Response, Vec<u8>)> = Vec::new();
    while let Ok(frame) = read_frame(&mut cursor) {
        let response: Response = serde_json::from_slice(&frame).expect("decode response");
        // Only `Done` is followed by bytes. Reading content after any other
        // variant would swallow the next answer's frame.
        let content = match response {
            Response::Done { output_len, .. } => read_content(&mut cursor, output_len, u64::MAX)
                .expect("the worker declared more content than it sent"),
            _ => Vec::new(),
        };
        answers.push((response, content));
    }
    answers
}

/// Test limits, with the two the image engine cares about varied.
///
/// Named rather than inline so a ceiling added to `RunLimits` shows up in one
/// place here instead of at every call site.
fn lim(decode_pixels: u64, memory_bytes: u64) -> RunLimits {
    RunLimits {
        decode_pixels,
        memory_bytes,
        archive_depth: 32,
        archive_entries: 100_000,
        archive_total_bytes: 8 << 30,
        use_gpu: true,
    }
}

/// One request with its content.
fn req(r: Request, content: &[u8]) -> (Request, Vec<u8>) {
    (r, content.to_vec())
}

/// **The architecture, proven rather than described.**
///
/// A separate program starts, applies its own confinement, reports what
/// actually engaged, converts an image it was handed as bytes, and answers â€”
/// without ever seeing a path.
#[test]
fn the_worker_runs_as_a_real_program_and_converts() {
    let src = png(32, 32);
    let answers = talk(&[
        req(Request::Hello, &[]),
        req(
            Request::Probe {
                input_len: src.len() as u64,
            },
            &src,
        ),
        req(
            Request::Run {
                input_len: src.len() as u64,
                to: "jpeg".into(),
                limits: lim(1 << 20, 1 << 28),
                params: Vec::new(),
                model_lens: Vec::new(),
                extra_lens: Vec::new(),
            },
            &src,
        ),
    ]);

    assert_eq!(answers.len(), 3, "one answer per request");

    let Response::Ready {
        engine,
        version,
        mitigations,
    } = &answers[0].0
    else {
        panic!("expected Ready, got {:?}", answers[0].0);
    };
    assert_eq!(engine, "oc-images");
    assert!(version.contains("image-rs"), "version was {version}");
    if cfg!(windows) {
        // Read back from the OS in the CHILD, which is the only place the
        // answer means anything (03 Â§9.5).
        assert_eq!(
            mitigations.len(),
            3,
            "ACG, CIG and AppContainer should all be reported"
        );

        // **THE CONTROL for the whole confinement story.**
        //
        // This test starts the binary with plain `Command` and bare pipes --
        // no AppContainer, no mitigation policy. `host_drives_worker.rs` starts
        // the *same binary* through the real host and asserts both are ON.
        //
        // Same program, two spawn paths, opposite answers. Without this half,
        // a `TokenIsAppContainer` query that returned `true` unconditionally
        // would satisfy the other test completely -- and "the confinement check
        // always says yes" is the one bug that would make every receipt this
        // product writes a lie.
        let engaged = |name: &str| {
            mitigations
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, on)| *on)
        };
        assert_eq!(
            engaged("AppContainer"),
            Some(false),
            "spawned WITHOUT a container, the worker claimed one: {mitigations:?}"
        );
        assert_eq!(
            engaged("ACG"),
            Some(false),
            "ACG is requested at spawn, not inherent -- a plain spawn must not have it"
        );
    }
    #[cfg(target_os = "linux")]
    {
        // **THE LINUX CONTROL, and it reads the opposite way.**
        //
        // This binary was started with plain `Command` -- no host-side
        // confinement exists on this platform. It confined ITSELF, before
        // reading any byte we sent, and the read-back attempts prove both
        // mechanisms engaged: a filesystem create was refused and an outbound
        // IPv4 socket was refused, inside that child.
        //
        // The Windows half of this test asserts the plain spawn is UNconfined;
        // here the plain spawn is the ONLY kind there is, so the assertion is
        // that self-confinement happened. Same program, same protocol, two
        // platforms, two correct answers.
        let engaged = |name: &str| {
            mitigations
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, on)| *on)
        };
        assert_eq!(
            engaged("Landlock"),
            Some(true),
            "the worker did not confine its own filesystem access: {mitigations:?}"
        );
        assert_eq!(
            engaged("Seccomp"),
            Some(true),
            "the worker did not deny its own network access: {mitigations:?}"
        );
        // Two (Reduced) or three (Full: + NetNs) mechanisms depending on
        // whether the kernel granted unprivileged user namespaces — both are
        // honest answers, and the probe decides which this machine gets.
        assert!(
            mitigations.len() == 2 || mitigations.len() == 3,
            "Linux reports Reduced (2) or Full (3) mechanisms: {mitigations:?}"
        );
        if let Some(netns) = engaged("NetNs") {
            assert!(
                netns,
                "a NetNs entry reading false would be an escalation claimed but absent"
            );
        }
    }

    // The Image variant specifically: an image engine that answered with an
    // archive shape would be answering a question nobody asked.
    let Response::Properties(openconvert_worker::Probed::Image {
        width,
        height,
        has_alpha,
        frames,
    }) = &answers[1].0
    else {
        panic!("expected image Properties, got {:?}", answers[1].0);
    };
    assert_eq!((*width, *height), (32, 32));
    // Read from the header, not decoded and not guessed: the fixture is an
    // RGBA PNG, which is a single-frame format with alpha.
    assert!(*has_alpha, "an RGBA PNG declares alpha");
    assert_eq!(*frames, 1, "PNG is single-frame and known from the header");

    let (Response::Done { removed, .. }, bytes) = &answers[2] else {
        panic!("expected Done, got {:?}", answers[2].0);
    };
    assert!(
        bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "the worker did not return a JPEG"
    );
    assert!(
        !removed.is_empty(),
        "a re-encode drops metadata and must say so"
    );
}

/// The pixel limit is enforced **in the worker**, not only by the host.
///
/// The host clamps (I14), and the worker checks again â€” because this process is
/// the one that would do the allocating, and a worker that trusts its caller is
/// a worker that cannot be reused safely.
#[test]
fn the_worker_enforces_its_own_pixel_limit() {
    let src = png(64, 64);
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "jpeg".into(),
            limits: lim(100, 1 << 28),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);

    let Response::Failed {
        engine,
        message,
        retryable,
    } = &answers[0].0
    else {
        panic!("expected Failed, got {:?}", answers[0].0);
    };
    assert_eq!(engine, "oc-images");
    assert!(
        message.contains("4096") && message.contains("100"),
        "the message should name both the actual and the limit: {message}"
    );
    assert!(
        *retryable,
        "a limit is not a crash; the host may offer to raise it"
    );
}

/// The control: the same image passes under a sane limit.
///
/// Without it, the test above is satisfied by a worker that refuses everything.
#[test]
fn the_same_image_converts_under_a_sane_limit() {
    let src = png(64, 64);
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "png".into(),
            limits: lim(1 << 20, 1 << 28),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    assert!(
        matches!(answers[0].0, Response::Done { .. }),
        "expected Done, got {:?}",
        answers[0].0
    );
}

/// A format the worker cannot write is refused by name, and not retryable.
#[test]
fn an_unsupported_output_format_is_refused_clearly() {
    let src = png(8, 8);
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "heic".into(),
            limits: lim(1 << 20, 1 << 28),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    let Response::Failed {
        message, retryable, ..
    } = &answers[0].0
    else {
        panic!("expected Failed");
    };
    assert!(
        message.contains("heic"),
        "the message should name the format: {message}"
    );
    assert!(!retryable, "retrying will not add HEIC support");
}

/// Garbage in, structured failure out â€” the worker does not crash.
///
/// It is handed attacker-supplied bytes by definition. A panic here takes down
/// a process the host is waiting on, and the host sees an exit code instead of
/// a reason.
#[test]
fn malformed_image_bytes_produce_a_failure_not_a_crash() {
    let junk = b"\x89PNG\r\n\x1a\nTHIS IS NOT AN IMAGE";
    let answers = talk(&[req(
        Request::Run {
            input_len: junk.len() as u64,
            to: "jpeg".into(),
            limits: lim(1 << 20, 1 << 28),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        junk,
    )]);
    assert!(
        matches!(answers[0].0, Response::Failed { .. }),
        "expected a structured failure, got {:?}",
        answers[0].0
    );
}

/// **A file far larger than one frame.**
///
/// `MAX_FRAME_BYTES` is 1 MiB and `Limits::defaults` permits a 4 GiB output. For
/// as long as `Run` carried a `Vec<u8>`, those two facts contradicted each
/// other: serde encodes a byte vector as an array of decimal numbers, so a 4 MB
/// PNG became a 13 MB frame and **every image over roughly 300 KB was refused**
/// by the sandboxed path.
///
/// Content now arrives as raw chunked frames, so the allocation bound is
/// untouched and the ceiling is gone.
#[test]
fn a_file_much_larger_than_one_frame_converts() {
    // Pseudo-random pixels so PNG cannot compress the fixture away.
    let img = image::RgbaImage::from_fn(1024, 1024, |x, y| {
        let n = x
            .wrapping_mul(2_654_435_761)
            .wrapping_add(y.wrapping_mul(40_503));
        image::Rgba([(n >> 16) as u8, (n >> 8) as u8, n as u8, 255])
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode");
    let src = buf.into_inner();
    // Several frames, not merely two: a chunking bug that drops or duplicates
    // the tail chunk survives a two-chunk test surprisingly often.
    assert!(
        src.len() > 3 * (openconvert_sandbox::protocol::MAX_FRAME_BYTES as usize),
        "the fixture must span several frames; it is {} bytes",
        src.len()
    );

    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "jpeg".into(),
            limits: lim(1 << 22, 1 << 30),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);

    let (Response::Done { output_len, .. }, bytes) = &answers[0] else {
        panic!("expected Done, got {:?}", answers[0].0);
    };
    assert_eq!(
        *output_len as usize,
        bytes.len(),
        "the declared length must match the bytes that followed"
    );
    assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
}
