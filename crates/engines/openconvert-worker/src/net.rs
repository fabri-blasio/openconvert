//! THE one outbound network call site.
//!
//! SR-9: zero telemetry, and exactly one fetch path — the weekly
//! update-and-revocation manifest and the model downloads the user asks for
//! by id share it, and `cargo xtask lint` fails any other socket, TLS or
//! HTTP call site in the boundary crates. This module lives in the WORKER
//! crate rather than `openconvert-run` for a hard reason: every pure-Rust TLS
//! stack bottoms out in a natively-linked crypto crate (`ring`,
//! aws-lc-rs), and the depcheck gate rightly refuses native linkage inside
//! `-run`. The worker crate is where native linkage is permitted — the same
//! argument that puts the engines here.
//!
//! # What a fetch owes its caller
//!
//! - a **hard byte cap**, enforced while reading (a hostile or misconfigured
//!   server cannot OOM the host by declaring a small file and sending big);
//! - a **timeout**, so a stalled mirror cannot park a conversion job;
//! - an initial attempt and **three retries** for transient transport failures;
//!   callers must not multiply this budget with another retry loop;
//! - **progress**, for the same reason a 90 MB download needs it at all.

use std::io::Read;
use std::time::Duration;

/// Why a fetch failed.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// The response exceeded the caller's cap.
    #[error("the download exceeded the {limit}-byte limit")]
    TooLarge {
        /// The cap that was refused.
        limit: u64,
    },
    /// Transport, DNS, TLS or status failure.
    #[error("{0}")]
    Transport(String),
}

/// Fetch `url`, capped at `max_bytes`, with fixed connect+read timeouts.
///
/// Returns the exact bytes received. Anything larger than the cap is an
/// error naming the cap, not a truncated success.
///
/// # Errors
///
/// See [`NetError`].
pub fn fetch(url: &str, max_bytes: u64) -> Result<Vec<u8>, NetError> {
    fetch_with_progress(url, max_bytes, &mut |_, _| {})
}

/// Fetch `url`, reporting bytes as they arrive.
///
/// `on_progress` receives `(received_so_far, total_expected)`, where the total
/// is the server's `Content-Length` or `0` when it did not send one -- **never
/// a guess**, because a progress bar that invents its denominator lies twice.
///
/// # Why this exists rather than `read_to_end`
///
/// The previous version read the whole body in one call. Nothing was wrong with
/// it except what a user saw: the desktop app's download bar sat at zero for the
/// length of a 78 MB model and then vanished. The frontend contract
/// (`ModelProgress { receivedBytes, totalBytes, phase }`) was fully specified,
/// the preview harness emitted it, and the product had no producer at all --
/// so the bar animated in the browser and was dead in the shipped app.
///
/// The chunk is 64 KiB: small enough that a slow link moves the bar several
/// times a second, large enough that the callback is not the bottleneck.
///
/// # Errors
///
/// See [`NetError`]. The cap is enforced *while reading*, so an oversized body
/// is refused without ever being held in full.
pub fn fetch_with_progress(
    url: &str,
    max_bytes: u64,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<Vec<u8>, NetError> {
    /// How many times to pick the download back up after the connection drops.
    ///
    /// **A 64 MB model over one uninterrupted TLS connection is optimistic.**
    /// The reported failure was `os error 10054` — the peer reset the
    /// connection — partway through the Real-ESRGAN artifact, and with no retry
    /// that lost the whole download and reported the feature as failed.
    const ATTEMPTS: u32 = 4;

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build();

    let mut body: Vec<u8> = Vec::with_capacity(64 << 10);
    let mut total: u64 = 0;
    let mut last: Option<NetError> = None;

    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            // Linear, short, and bounded. Long enough to outlast a blip, short
            // enough that four of them do not look like a hang.
            std::thread::sleep(Duration::from_millis(500 * u64::from(attempt)));
        }

        // RESUME WHERE IT STOPPED. A reset at 60 MB should cost the last chunk,
        // not the first sixty. `Range` is a request; a server that ignores it
        // answers 200 with the whole body, which is handled below by starting
        // the buffer over rather than appending to it.
        let mut request = agent
            .get(url)
            .set("User-Agent", "OpenConvert model downloader");
        if !body.is_empty() {
            request = request.set("Range", &format!("bytes={}-", body.len()));
        }

        let response = match request.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                if matches!(code, 408 | 429 | 500 | 502 | 503 | 504) {
                    last = Some(NetError::Transport(format!(
                        "the model server temporarily answered {code}"
                    )));
                    continue;
                }
                // A status error is the server's considered answer, not a
                // broken pipe. Retrying a 404 or a 403 just asks again.
                return Err(NetError::Transport(format!(
                    "the server answered {code} for {url} ({})",
                    r.status_text()
                )));
            }
            Err(other) => {
                last = Some(NetError::Transport(other.to_string()));
                continue;
            }
        };

        let resumed = response.status() == 206;
        if !resumed {
            // Not a partial answer: whatever we had is not a prefix of this
            // body, so it is discarded rather than concatenated into a file
            // that would fail its hash in a way nobody could explain.
            body.clear();
        }

        let declared: u64 = response
            .header("Content-Length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // The server's own number, or zero. Zero means "show an indeterminate
        // bar", which is honest; a number means a real one can be drawn. On a
        // resumed response it counts only what is left, so the total is the
        // part already held plus the remainder.
        if declared > 0 {
            total = body.len() as u64 + declared;
        }

        let remaining = max_bytes
            .saturating_sub(body.len() as u64)
            .saturating_add(1);
        let mut reader = response.into_reader().take(remaining);
        let mut chunk = vec![0_u8; 64 << 10];
        on_progress(body.len() as u64, total);

        let mut failed: Option<NetError> = None;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    body.extend_from_slice(&chunk[..n]);
                    if body.len() as u64 > max_bytes {
                        return Err(NetError::TooLarge { limit: max_bytes });
                    }
                    on_progress(body.len() as u64, total);
                }
                Err(e) => {
                    failed = Some(NetError::Transport(e.to_string()));
                    break;
                }
            }
        }

        match failed {
            // Read to the end without an error: this is the whole file.
            None => return Ok(body),
            Some(e) => last = Some(e),
        }
    }

    Err(last.unwrap_or_else(|| {
        NetError::Transport(format!(
            "could not download {url} after {ATTEMPTS} attempts"
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bogus_host_is_a_transport_error_not_a_panic() {
        // No network in unit tests' contract; an unresolvable host answers
        // fast and shapes the error without touching a socket that matters.
        let err = fetch("https://bogus.invalid/openconvert-probe", 1024);
        assert!(matches!(err, Err(NetError::Transport(_))));
    }
}
