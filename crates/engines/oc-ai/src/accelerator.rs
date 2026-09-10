//! Which execution provider runs a model, and why it is not always the GPU.
//!
//! # What was measured
//!
//! On a laptop with an RTX 4050 and Intel Iris Xe, Windows 11 26200, ONNX
//! Runtime 1.20.1 with the DirectML 1.15.2 redistributable:
//!
//! | model | CPU | DirectML |
//! |---|---|---|
//! | Real-ESRGAN x4, 256×192 → 1024×768 | 107.3 s | **14.5 s** |
//! | u2netp background removal | ~1 s | 7 s of session build alone |
//! | BiRefNet-lite, 1024² fixed | 46 s | out of video memory |
//!
//! Three different answers, and they are the whole design.
//!
//! **The GPU is a large win on heavy models.** 7.4× on upscaling is not a
//! tuning gain, it is the difference between a feature people use and one they
//! start and abandon.
//!
//! **It is a loss on small ones.** DirectML compiles the graph when the
//! session is built, and that cost — five to nine seconds here — is fixed
//! regardless of how little work follows. For a model that finishes in about a
//! second on the CPU, moving it to the GPU makes it *seven times slower*. A
//! build that used the GPU for everything would be able to advertise GPU
//! support and make background removal worse.
//!
//! **It can fail on the largest model of all.** BiRefNet needs about 6.5 GB;
//! this card has 4 GB. Video memory is not the machine's memory and cannot be
//! raised in Settings, so the fallback is not decorative — it is the only
//! reason the best tier runs at all on hardware like this.
//!
//! # ACG does not block it, which was not the expectation
//!
//! Workers are spawned into an AppContainer with
//! `PROHIBIT_DYNAMIC_CODE_ALWAYS_ON`, and a GPU driver compiles shaders at run
//! time — dynamic code generation, which is precisely what that mitigation
//! forbids. The expectation was that GPU inference and the sandbox were
//! incompatible.
//!
//! Measured instead, with `examples/try_gpu.rs` spawning itself through the
//! same `spawn_piped_in_container` the pool uses: the child reports
//! `ACG: true  AppContainer: true` and builds a DirectML session anyway. So
//! the GPU costs nothing in confinement here. That claim is worth re-testing
//! on other drivers rather than inheriting.
//!
//! # The rule: never silently
//!
//! Whichever provider ran is **returned to the caller**, not logged and
//! dropped. A user whose GPU sat idle through a two-minute conversion is
//! entitled to know the machine chose not to use it.

/// An execution provider this build can attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// DirectML: vendor-neutral on Windows, working on NVIDIA, AMD and Intel
    /// alike, and needing no vendor toolkit beside the binary.
    ///
    /// Chosen over CUDA deliberately. CUDA is faster on the hardware it
    /// supports, and it is NVIDIA-only and needs a multi-hundred-megabyte
    /// redistributable — which would make the shipped artifact depend on a
    /// driver stack that cannot be verified at build time. This product runs
    /// on whatever machine it is put on.
    DirectML,
    /// The portable path, and the only one guaranteed to exist.
    Cpu,
}

impl Provider {
    /// The name as it appears in a receipt.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DirectML => "DirectML",
            Self::Cpu => "CPU",
        }
    }
}

/// How much work a model represents, which decides whether a GPU pays.
///
/// Not a measure of the model's size on disk. It is a claim about the
/// arithmetic per call, which is what the session-build cost has to be
/// amortised against — a 4.5 MB model run over four hundred tiles is heavy,
/// and a 224 MB model run once on a thumbnail is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// Seconds of arithmetic or more: the GPU wins, comfortably.
    Heavy,
    /// About a second on a CPU: the GPU's session build costs more than the
    /// whole job, so do not pay it.
    Light,
}

/// Providers to try for a workload, best first.
///
/// CPU is always last and always present, so this list is never empty and a
/// caller can always finish.
#[must_use]
pub fn candidates_for(workload: Workload) -> Vec<Provider> {
    match workload {
        Workload::Heavy if gpu_allowed() => vec![Provider::DirectML, Provider::Cpu],
        _ => vec![Provider::Cpu],
    }
}

/// Every provider, for the probe example. Not a runtime path.
#[must_use]
pub fn candidates() -> Vec<(&'static str, Provider)> {
    vec![("directml", Provider::DirectML), ("cpu", Provider::Cpu)]
}

/// Whether the GPU may be used at all right now.
///
/// Two sources, and the request wins.
///
/// **`RunLimits::use_gpu`** carries the user's Settings choice, clamped by
/// every policy layer above it. It is a limit like the others: a layer may
/// withdraw the GPU and none may grant it, which is what makes "off" mean off
/// rather than "off unless something downstream disagrees".
///
/// **`OPENCONVERT_GPU=off`** is a developer override for the paths that have no
/// request — `examples/`, and a worker driven by hand. It can only ever take
/// the GPU away, so it cannot be used to re-enable hardware the user refused.
fn gpu_allowed() -> bool {
    if !openconvert_worker::use_gpu() {
        return false;
    }
    !matches!(
        std::env::var("OPENCONVERT_GPU").as_deref(),
        Ok("off" | "0" | "false" | "cpu")
    )
}

/// Build a session on a named provider, refusing rather than falling back.
///
/// # Errors
///
/// When the provider is unavailable in this build or on this machine, or when
/// the model will not load.
pub fn try_session(model: &[u8], provider: Provider) -> Result<(), String> {
    crate::infer::session_on(model, provider).map(|_| ())
}

/// Try each provider in turn: build something on it, then do work with it.
///
/// # Why the policy is here and not beside the sessions
///
/// **So it can be tested at all.** The bug this exists to prevent was a
/// fallback that covered building and not running, and it survived because
/// nothing could exercise it: reproducing it needed a graphics card, a large
/// model, and an image big enough to exhaust the card's memory. None of that
/// belongs in a test suite, and without a test the loop was whatever its
/// comment claimed.
///
/// Generic over both, so a test supplies a fake session and a closure that
/// fails on the first provider and succeeds on the second — which is exactly
/// the shape of a DirectML run that exhausts video memory and then finishes on
/// the processor.
///
/// # The work is retried, not only the build
///
/// That is the entire point. `build` failing is the ordinary case of a
/// provider being absent; `work` failing is the case that reached users, and
/// the one the loop used not to see.
///
/// # Errors
///
/// The last candidate's error. With no candidates at all, a message saying so
/// rather than an empty string — an empty error reads as a success nobody
/// checked.
pub fn first_success<S, T>(
    candidates: &[Provider],
    mut build: impl FnMut(Provider) -> Result<S, String>,
    mut work: impl FnMut(&mut S) -> Result<T, String>,
) -> Result<(T, Provider), String> {
    let mut last: Option<String> = None;
    for &provider in candidates {
        match build(provider) {
            Err(e) => last = Some(e),
            Ok(mut built) => match work(&mut built) {
                Ok(value) => return Ok((value, provider)),
                Err(e) => last = Some(e),
            },
        }
    }
    Err(last.unwrap_or_else(|| "no execution provider was offered at all".to_string()))
}

#[cfg(test)]
mod fallback_tests {
    use super::{first_success, Provider};
    use std::cell::RefCell;

    /// **THE REGRESSION THIS FILE EXISTS FOR: the WORK fails, not the build.**
    ///
    /// DirectML builds a session without complaint and then exhausts video
    /// memory partway through `run()`. The old loop wrapped only the build, so
    /// it saw nothing and the run died on the GPU — on a machine whose
    /// processor could have finished it.
    #[test]
    fn a_run_that_fails_on_the_card_finishes_on_the_processor() {
        let tried = RefCell::new(Vec::new());
        let out = first_success(
            &[Provider::DirectML, Provider::Cpu],
            |p| {
                tried.borrow_mut().push(("build", p));
                // BOTH BUILD FINE. That is the situation.
                Ok(p)
            },
            |on: &mut Provider| {
                tried.borrow_mut().push(("run", *on));
                if *on == Provider::DirectML {
                    Err("out of video memory".to_string())
                } else {
                    Ok("the mask")
                }
            },
        );
        assert_eq!(out, Ok(("the mask", Provider::Cpu)));
        assert_eq!(
            tried.into_inner(),
            vec![
                ("build", Provider::DirectML),
                ("run", Provider::DirectML),
                ("build", Provider::Cpu),
                ("run", Provider::Cpu),
            ],
            "the card is tried first and the processor only after it fails"
        );
    }

    /// The older case still works: a provider that cannot be built is skipped.
    #[test]
    fn a_provider_that_will_not_build_is_skipped() {
        let ran_on = RefCell::new(Vec::new());
        let out = first_success(
            &[Provider::DirectML, Provider::Cpu],
            |p| {
                if p == Provider::DirectML {
                    Err("DirectML is not in this build".to_string())
                } else {
                    Ok(p)
                }
            },
            |on: &mut Provider| {
                ran_on.borrow_mut().push(*on);
                Ok(42)
            },
        );
        assert_eq!(out, Ok((42, Provider::Cpu)));
        assert_eq!(
            ran_on.into_inner(),
            vec![Provider::Cpu],
            "work never runs on a provider that could not be built"
        );
    }

    /// **A run that succeeds on the card does NOT also run on the processor.**
    ///
    /// The fallback costs a whole second attempt, and upscaling on the CPU is
    /// seven times slower than on the card. Doing it when the first attempt
    /// worked would make every success pay for the failure case.
    #[test]
    fn a_run_that_succeeds_is_not_repeated() {
        let runs = RefCell::new(0_usize);
        let out = first_success(
            &[Provider::DirectML, Provider::Cpu],
            Ok,
            |_on: &mut Provider| {
                *runs.borrow_mut() += 1;
                Ok(())
            },
        );
        assert_eq!(out, Ok(((), Provider::DirectML)));
        assert_eq!(runs.into_inner(), 1, "the processor was not asked as well");
    }

    /// When everything fails, the LAST error is the one reported.
    ///
    /// Which means the processor's, whenever the processor was reached — and
    /// that is the message worth showing. A DirectML diagnostic about a shape
    /// it could not fuse tells a user nothing they can act on.
    #[test]
    fn the_reported_failure_is_the_last_one() {
        let out: Result<((), Provider), String> = first_success(
            &[Provider::DirectML, Provider::Cpu],
            Ok,
            |on: &mut Provider| Err(format!("{on:?} could not do it")),
        );
        assert_eq!(out, Err("Cpu could not do it".to_string()));
    }

    /// No candidates is not a silent success.
    #[test]
    fn an_empty_ladder_says_so() {
        let out: Result<((), Provider), String> =
            first_success(&[], |p: Provider| Ok(p), |_on: &mut Provider| Ok(()));
        let Err(why) = out else {
            panic!("an empty ladder must not report success");
        };
        assert!(!why.is_empty(), "an empty error reads as an unchecked one");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A light workload never asks for the GPU.
    ///
    /// Measured: u2netp finishes in about a second on the CPU, and building a
    /// DirectML session for it costs seven. "Use the GPU when there is one" is
    /// the wrong rule; "use it where it pays" is the right one, and this is
    /// where the difference lives.
    #[test]
    fn a_light_workload_stays_on_the_cpu() {
        assert_eq!(candidates_for(Workload::Light), vec![Provider::Cpu]);
    }

    /// CPU is always last and always present, so a caller can always finish.
    ///
    /// The GPU can fail after registering -- BiRefNet exhausts a 4 GB card's
    /// video memory, which is not the machine's memory and cannot be raised in
    /// Settings. An empty or GPU-only list would turn that into a failed
    /// conversion instead of a slower one.
    #[test]
    fn every_workload_can_always_finish() {
        for w in [Workload::Heavy, Workload::Light] {
            let list = candidates_for(w);
            assert!(!list.is_empty(), "{w:?} had no provider at all");
            assert_eq!(
                list.last(),
                Some(&Provider::Cpu),
                "{w:?} did not end at the CPU, so a GPU failure would have nowhere to go"
            );
        }
    }

    /// Providers name themselves for the receipt, and distinctly.
    #[test]
    fn providers_are_named_distinctly() {
        assert_eq!(Provider::Cpu.name(), "CPU");
        assert_eq!(Provider::DirectML.name(), "DirectML");
        assert_ne!(Provider::Cpu.name(), Provider::DirectML.name());
    }
}
