//! `oc-ai` — the model worker: ONNX Runtime inference behind the same
//! protocol, confinement and receipt path as every other engine.
//!
//! # The weights arrive on the pipe
//!
//! Every other worker reads one thing: the file being converted. This one
//! reads two, because inference needs weights, and a worker has no filesystem
//! to load them from — on Linux Landlock denies all of it, which is stated in
//! `openconvert-worker` as the reason the protocol needs none.
//!
//! Handing this worker a path into the model store would have punched a hole
//! in exactly the confinement the receipts are about, and this is the last
//! worker that should get one: its input is a file the user was told we do not
//! trust. So the host reads the sha256-verified artifact from the
//! content-addressed store and writes it down the pipe after the input.
//!
//! # The runtime is loaded, never linked
//!
//! `ort`'s `load-dynamic` feature resolves `onnxruntime.dll` at run time from
//! beside this binary. An absent runtime is therefore a REFUSAL naming what is
//! missing, not a process that fails to start — the same rule `oc-pdf` follows
//! for pdfium, and the reason `available` can mean "a plan routed here
//! finishes".
//!
//! # Platform scope
//!
//! Windows only today; `build.rs` has no POSIX linkage story yet, so `has_ort`
//! stays unset elsewhere and every request refuses by name.

use openconvert_worker::{run_real, Converted, Engine, Probed, RunLimits};

/// The model engine.
struct Ai;

impl Engine for Ai {
    #[allow(clippy::too_many_arguments)]
    fn run_progress(
        &self,
        bytes: &[u8],
        extras: &[Vec<u8>],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<Converted, String> {
        #[cfg(all(windows, has_ort))]
        if extras.is_empty() && params.iter().any(|(k, v)| k == "task" && v == "transcribe") {
            return oc_ai::transcribe_to_format_progress(
                bytes,
                models,
                limits,
                params.iter().any(|(k, v)| k == "format" && v == "vtt"),
                progress,
            );
        }
        let _ = progress;
        self.run_with_inputs(bytes, extras, models, to, params, limits)
    }
    fn name(&self) -> &'static str {
        "oc-ai"
    }

    fn version(&self) -> String {
        #[cfg(all(windows, has_ort))]
        {
            oc_ai::runtime_version().unwrap_or_else(|_| "onnxruntime (version unavailable)".into())
        }
        #[cfg(not(all(windows, has_ort)))]
        {
            "no runtime".into()
        }
    }

    /// Probing is not this worker's job.
    ///
    /// Identity comes from the sniffer and the image workers; a model has
    /// nothing to add about what a file IS, and answering anyway would put a
    /// neural network's opinion into detection, which is the one place this
    /// product must stay deterministic.
    fn probe(&self, _bytes: &[u8]) -> Result<Probed, String> {
        Err("oc-ai does not identify files; it only runs models over them".into())
    }

    fn run(&self, _bytes: &[u8], _to: &str, _limits: &RunLimits) -> Result<Converted, String> {
        Err("oc-ai needs a model; this request carried none".into())
    }

    fn run_model(
        &self,
        bytes: &[u8],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        if models.is_empty() {
            return Err("oc-ai needs a model; this request carried none".into());
        }
        let task = params
            .iter()
            .find(|(k, _)| k == "task")
            .map(|(_, v)| v.as_str())
            .ok_or("oc-ai needs a `task` parameter naming which adapter to run")?;

        #[cfg(all(windows, has_ort))]
        {
            match task {
                "remove-background" => {
                    let [model] = models else {
                        return Err(format!(
                            "background removal takes one model, got {}",
                            models.len()
                        ));
                    };
                    oc_ai::remove_background(bytes, model, to, limits)
                }
                // THE QUALITY TIER TAKES A SECOND MODEL, AND IT IS NOT OPTIONAL
                // DECORATION.
                //
                // MODNet is a PORTRAIT matting network. On a person it is the
                // better of the two and that is why it is the default. On
                // anything else it can return a matte with almost nothing in
                // it -- and "nothing" written to a PNG is a file that opens as
                // an empty transparent rectangle, which is what was reported.
                // A tool that silently produces an empty picture is worse than
                // one that produces a slightly softer edge.
                //
                // So: run MODNet, look at what it produced, and fall back to
                // u2netp -- a general saliency model, weaker on hair and
                // reliable on everything -- when the matte is degenerate. The
                // receipt says which one made the alpha either way, so a
                // fallback is never silent.
                // The best tier. Same two-model shape as the quality tier --
                // the model, then the net under it.
                "remove-background-best" => {
                    let [best, fallback] = models else {
                        return Err(format!(
                            "the best tier takes the matting model and the fallback, got {}",
                            models.len()
                        ));
                    };
                    oc_ai::remove_background_birefnet(bytes, best, fallback, to, limits)
                }
                "remove-background-quality" => {
                    let [quality, fallback] = models else {
                        return Err(format!(
                            "the quality tier takes the matting model and the fallback, got {}",
                            models.len()
                        ));
                    };
                    oc_ai::remove_background_best(bytes, quality, fallback, to, limits)
                }
                "ocr" => {
                    if params.iter().any(|(k, v)| k == "format" && v == "layout") {
                        oc_ai::ocr_to_layout(bytes, models, limits)
                    } else {
                        oc_ai::ocr_to_text(bytes, models, limits)
                    }
                }
                "transcribe" => oc_ai::transcribe_to_text(bytes, models, limits),
                "upscale" => oc_ai::upscale_image(bytes, models, to, limits),
                "denoise" => oc_ai::denoise_audio(bytes, models, limits),
                other => Err(format!(
                    "oc-ai has no adapter for `{other}`; this build runs: {}",
                    oc_ai::ADAPTERS.join(", ")
                )),
            }
        }
        #[cfg(not(all(windows, has_ort)))]
        {
            let _ = (bytes, to, limits, task);
            Err(
                "this build of oc-ai has no ONNX Runtime linked, so no model can run. \
                 Reinstall the runtime engine library and rebuild to enable inference"
                    .into(),
            )
        }
    }
}

fn main() -> std::io::Result<()> {
    run_real(&Ai)
}
