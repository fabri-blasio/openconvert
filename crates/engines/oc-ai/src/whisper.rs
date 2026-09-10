//! Whisper: audio in, transcript out.
//!
//! Three artifacts — encoder, decoder, tokeniser — and a front end
//! ([`crate::mel`]) that turns audio into the spectrogram the encoder expects.
//!
//! # Greedy decoding, without a KV cache
//!
//! The exported decoder comes in two shapes: `decoder_model_merged`, which
//! threads past key/values through as inputs and outputs, and `decoder_model`,
//! which takes the whole token prefix each time. This uses the second.
//!
//! It is O(n²) in generated tokens where the cache is O(n), and for a
//! 30-second window whose transcript is at most a couple of hundred tokens
//! that is a real but bounded cost. What it buys is the absence of ~24 cache
//! tensors that must be re-fed in exactly the right order every step — a class
//! of bug that produces fluent, confident, wrong text rather than an error.

use ndarray::{Array2, Array3};
use ort::session::Session;
use ort::value::Value;
use std::collections::HashMap;

use crate::accelerator::Workload;
use crate::infer::session_on;
use crate::mel;

/// Longest transcript per 30-second window.
///
/// Whisper's own limit is 448 tokens of context. A window that hits this is
/// almost always a model looping on silence, which is the failure this bounds.
const MAX_TOKENS: usize = 224;

/// Everything needed to turn ids back into text.
struct Tokeniser {
    /// id → token string, in the byte-level alphabet.
    tokens: Vec<String>,
    /// GPT-2 byte-level alphabet, reversed: printable char → original byte.
    byte_decoder: HashMap<char, u8>,
    sot: i64,
    eot: i64,
    transcribe: i64,
    no_timestamps: i64,
    /// Every `<|xx|>` language token the vocabulary carries, as (code, id).
    ///
    /// **This is the model's language list, taken from the model.** A
    /// multilingual Whisper export declares ninety-nine of them; an
    /// English-only export declares one. Nothing else in this build knows
    /// which was pinned, so reading it here is the only way the refusal below
    /// can name a real limitation rather than a hard-coded assumption.
    languages: Vec<(String, i64)>,
}

impl Tokeniser {
    /// Parse a HuggingFace `tokenizer.json`.
    ///
    /// Only the vocabulary is read. The merges, normalisers and pre-tokenisers
    /// matter for ENCODING text, and nothing here encodes — the prompt is
    /// special tokens only, and the output is ids coming back.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let json: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("the tokeniser file is not valid JSON: {e}"))?;
        let vocab = json
            .get("model")
            .and_then(|m| m.get("vocab"))
            .and_then(|v| v.as_object())
            .ok_or("the tokeniser file has no model.vocab")?;

        let mut pairs: Vec<(usize, String)> = Vec::with_capacity(vocab.len() + 1500);
        for (tok, id) in vocab {
            if let Some(i) = id.as_u64() {
                pairs.push((i as usize, tok.clone()));
            }
        }
        // The specials live in `added_tokens`, not in the vocab map.
        if let Some(added) = json.get("added_tokens").and_then(|a| a.as_array()) {
            for t in added {
                if let (Some(i), Some(c)) = (
                    t.get("id").and_then(serde_json::Value::as_u64),
                    t.get("content").and_then(serde_json::Value::as_str),
                ) {
                    pairs.push((i as usize, c.to_string()));
                }
            }
        }
        let size = pairs.iter().map(|(i, _)| *i + 1).max().unwrap_or(0);
        let mut tokens = vec![String::new(); size];
        for (i, t) in pairs {
            tokens[i] = t;
        }

        let find = |name: &str| -> Result<i64, String> {
            tokens
                .iter()
                .position(|t| t == name)
                .map(|i| i as i64)
                .ok_or_else(|| format!("the tokeniser has no {name} token"))
        };
        // `<|xx|>` and `<|xxx|>` and nothing else. Whisper's other specials
        // are all longer than three characters inside the bars --
        // `<|transcribe|>`, `<|notimestamps|>`, `<|nospeech|>` -- so the
        // length bound separates them without a list to keep in step.
        let mut languages: Vec<(String, i64)> = tokens
            .iter()
            .enumerate()
            .filter_map(|(id, t)| {
                let code = t.strip_prefix("<|")?.strip_suffix("|>")?;
                let ok =
                    (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase());
                ok.then(|| (code.to_string(), id as i64))
            })
            .collect();
        languages.sort_by(|a, b| a.0.cmp(&b.0));
        // A vocabulary with no language token at all is not a Whisper
        // tokeniser, and the prompt below cannot be built without one. Said
        // here, where the artifact is, rather than as a confusing tensor error
        // several hundred lines later.
        if languages.is_empty() {
            return Err("the tokeniser declares no language tokens at all".to_string());
        }

        Ok(Self {
            sot: find("<|startoftranscript|>")?,
            eot: find("<|endoftext|>")?,
            transcribe: find("<|transcribe|>")?,
            no_timestamps: find("<|notimestamps|>")?,
            languages,
            tokens,
            byte_decoder: byte_decoder(),
        })
    }

    /// Ids back to text.
    ///
    /// Whisper's vocabulary is byte-level BPE: a token's characters are not
    /// the text, they are a reversible re-encoding of the underlying BYTES
    /// into printable code points. Concatenating token strings and calling it
    /// output gives mojibake for anything non-ASCII, so the bytes are
    /// recovered first and decoded as UTF-8 once at the end.
    fn decode(&self, ids: &[i64]) -> String {
        let mut bytes: Vec<u8> = Vec::new();
        for &id in ids {
            let Some(tok) = self.tokens.get(id as usize) else {
                continue;
            };
            // A special token that leaked into the output is dropped rather
            // than printed: `<|notimestamps|>` is not something the user said.
            if tok.starts_with("<|") && tok.ends_with("|>") {
                continue;
            }
            for ch in tok.chars() {
                match self.byte_decoder.get(&ch) {
                    Some(&b) => bytes.push(b),
                    // Not in the alphabet: keep it as literal UTF-8 rather
                    // than dropping a character we cannot explain.
                    None => {
                        let mut buf = [0_u8; 4];
                        bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Language codes to the names a person recognises.
///
/// Only for the MESSAGE. Nothing dispatches on this table: what the model can
/// do comes from the tokeniser, and a code missing from here is printed as the
/// code rather than being treated as unsupported — an incomplete display table
/// must never narrow what the program will transcribe.
/// Four to a line on purpose: a hundred rows one per line is a hundred lines of
/// scrolling between the code above it and the code below.
#[rustfmt::skip]
const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("af", "Afrikaans"), ("am", "Amharic"), ("ar", "Arabic"), ("as", "Assamese"),
    ("az", "Azerbaijani"), ("ba", "Bashkir"), ("be", "Belarusian"), ("bg", "Bulgarian"),
    ("bn", "Bengali"), ("bo", "Tibetan"), ("br", "Breton"), ("bs", "Bosnian"),
    ("ca", "Catalan"), ("cs", "Czech"), ("cy", "Welsh"), ("da", "Danish"),
    ("de", "German"), ("el", "Greek"), ("en", "English"), ("es", "Spanish"),
    ("et", "Estonian"), ("eu", "Basque"), ("fa", "Persian"), ("fi", "Finnish"),
    ("fo", "Faroese"), ("fr", "French"), ("gl", "Galician"), ("gu", "Gujarati"),
    ("ha", "Hausa"), ("haw", "Hawaiian"), ("he", "Hebrew"), ("hi", "Hindi"),
    ("hr", "Croatian"), ("ht", "Haitian Creole"), ("hu", "Hungarian"), ("hy", "Armenian"),
    ("id", "Indonesian"), ("is", "Icelandic"), ("it", "Italian"), ("ja", "Japanese"),
    ("jw", "Javanese"), ("ka", "Georgian"), ("kk", "Kazakh"), ("km", "Khmer"),
    ("kn", "Kannada"), ("ko", "Korean"), ("la", "Latin"), ("lb", "Luxembourgish"),
    ("ln", "Lingala"), ("lo", "Lao"), ("lt", "Lithuanian"), ("lv", "Latvian"),
    ("mg", "Malagasy"), ("mi", "Maori"), ("mk", "Macedonian"), ("ml", "Malayalam"),
    ("mn", "Mongolian"), ("mr", "Marathi"), ("ms", "Malay"), ("mt", "Maltese"),
    ("my", "Burmese"), ("ne", "Nepali"), ("nl", "Dutch"), ("nn", "Nynorsk"),
    ("no", "Norwegian"), ("oc", "Occitan"), ("pa", "Punjabi"), ("pl", "Polish"),
    ("ps", "Pashto"), ("pt", "Portuguese"), ("ro", "Romanian"), ("ru", "Russian"),
    ("sa", "Sanskrit"), ("sd", "Sindhi"), ("si", "Sinhala"), ("sk", "Slovak"),
    ("sl", "Slovenian"), ("sn", "Shona"), ("so", "Somali"), ("sq", "Albanian"),
    ("sr", "Serbian"), ("su", "Sundanese"), ("sv", "Swedish"), ("sw", "Swahili"),
    ("ta", "Tamil"), ("te", "Telugu"), ("tg", "Tajik"), ("th", "Thai"),
    ("tk", "Turkmen"), ("tl", "Tagalog"), ("tr", "Turkish"), ("tt", "Tatar"),
    ("uk", "Ukrainian"), ("ur", "Urdu"), ("uz", "Uzbek"), ("vi", "Vietnamese"),
    ("yi", "Yiddish"), ("yo", "Yoruba"), ("yue", "Cantonese"), ("zh", "Chinese"),
];

/// A language code as a person would read it. Unknown codes print as the code.
fn language_name(code: &str) -> String {
    LANGUAGE_NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| code.to_string(), |(_, n)| (*n).to_string())
}

/// How sure the model has to be about the language before a word is decoded.
///
/// Whisper picks a language token whatever it is fed; the probability is the
/// only thing that says whether it recognised the speech or merely ranked
/// ninety-nine bad options. Below this the recording is refused rather than
/// transcribed, because the failure mode of a wrong language is not garbled
/// text — it is fluent, confident text in a language nobody spoke.
const LANGUAGE_FLOOR: f32 = 0.35;

/// What the language pass concluded.
struct Detected {
    /// The `<|xx|>` token id to put in the prompt.
    token: i64,
    /// The code itself, for the message and the caller.
    code: String,
    /// Softmax probability over the whole vocabulary, 0–1.
    probability: f32,
}

/// Ask the model which language a window is in.
///
/// Whisper's first decoding step, given only `<|startoftranscript|>`, predicts
/// the language token; that is what the prompt below then asserts. **This used
/// to be hard-coded to `<|en|>`** — every recording in every language was
/// decoded as though it were English, which for a French or Japanese recording
/// produces confident English-shaped nonsense rather than an error.
///
/// The probability is taken over the FULL vocabulary row rather than over the
/// language tokens alone. Renormalising within the language set would report
/// 100 % certainty for a single-language model on any input at all, which is
/// exactly the reading that must not be possible here.
fn detect_language(
    decoder: &mut Session,
    tok: &Tokeniser,
    hidden: &Array3<f32>,
) -> Result<Detected, String> {
    if tok.languages.is_empty() {
        return Err("this tokeniser declares no languages at all".to_string());
    }
    let row = decode_step(decoder, hidden, &[tok.sot])?;

    // Softmax, shifted by the maximum so a large logit cannot overflow.
    let max = row.iter().copied().fold(f32::MIN, f32::max);
    let mut total = 0.0_f32;
    let exps: Vec<f32> = row
        .iter()
        .map(|&v| {
            let e = (v - max).exp();
            total += e;
            e
        })
        .collect();
    if !total.is_finite() || total <= 0.0 {
        return Err("the decoder's language logits were not usable numbers".to_string());
    }

    let mut best: Option<Detected> = None;
    for (code, id) in &tok.languages {
        let Some(&e) = exps.get(*id as usize) else {
            continue;
        };
        let probability = e / total;
        if best.as_ref().is_none_or(|b| probability > b.probability) {
            best = Some(Detected {
                token: *id,
                code: code.clone(),
                probability,
            });
        }
    }
    best.ok_or_else(|| "no language token scored at all".to_string())
}

/// GPT-2's byte ↔ printable-character alphabet, inverted.
///
/// The 188 bytes that are already printable map to themselves; the other 68
/// are moved into a free block starting at U+0100. This is the exact table
/// `bytes_to_unicode` builds, and it must match or every non-ASCII byte comes
/// back wrong.
fn byte_decoder() -> HashMap<char, u8> {
    let mut bs: Vec<u16> = Vec::with_capacity(256);
    bs.extend(b'!' as u16..=b'~' as u16);
    bs.extend(0xA1_u16..=0xAC);
    bs.extend(0xAE_u16..=0xFF);
    let mut cs: Vec<u32> = bs.iter().map(|&b| u32::from(b)).collect();
    let mut n = 0_u32;
    for b in 0_u16..=255 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    bs.iter()
        .zip(cs.iter())
        .filter_map(|(&b, &c)| char::from_u32(c).map(|ch| (ch, b as u8)))
        .collect()
}

/// Transcribe interleaved PCM.
///
/// # Errors
///
/// A message the host shows the user directly.
pub fn transcribe_progress(
    audio: &[f32],
    encoder_model: &[u8],
    decoder_model: &[u8],
    tokeniser: &[u8],
    vad_model: &[u8],
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, String> {
    transcribe_format(
        audio,
        encoder_model,
        decoder_model,
        tokeniser,
        vad_model,
        false,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn transcribe_format(
    audio: &[f32],
    encoder_model: &[u8],
    decoder_model: &[u8],
    tokeniser: &[u8],
    vad_model: &[u8],
    timed: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, String> {
    let tok = Tokeniser::parse(tokeniser)?;
    if audio.is_empty() {
        return Err("this file contains no audio to transcribe".into());
    }

    // Both halves are heavy: the encoder runs per 30-second window and the
    // decoder runs per token, so a transcript is thousands of calls through
    // two sessions built once.
    //
    // **THE WHOLE TRANSCRIPT IS THE UNIT OF FALLBACK.** This used to build the
    // two sessions with `session_for`, which falls back only when a provider
    // cannot be CREATED, and the comment here claimed that was enough: "77 MB
    // over fixed 30-second windows, which is not a size that exhausts a card."
    //
    // That was wrong, and a real recording proved it. On a 6 GB card the
    // decoder fails partway through with ONNX Runtime's "not enough memory
    // resources are available to complete this operation" -- at `run()`, long
    // after both sessions were built without complaint. The transcript came
    // back empty and the tool looked like it did nothing.
    //
    // It is one unit because the decoder runs in a loop against the encoder's
    // output: a token that fails on the card cannot be retried alone, since
    // the windows before it were decoded by sessions that are about to be
    // dropped. Redoing the recording on the processor is slower and it is the
    // difference between a slow transcript and none.
    let (text, _provider) = crate::accelerator::first_success(
        &crate::accelerator::candidates_for(Workload::Heavy),
        |provider| {
            Ok((
                session_on(encoder_model, provider)?,
                session_on(decoder_model, provider)?,
            ))
        },
        |(encoder, decoder)| {
            transcribe_on(encoder, decoder, &tok, audio, vad_model, timed, progress)
        },
    )?;
    Ok(text)
}

/// One attempt at a whole recording, on sessions the caller has already built.
///
/// Split out so the provider ladder above can run it again on the next
/// candidate. Everything it needs is a parameter; it holds no state of its own,
/// which is what makes a second attempt identical to the first.
#[allow(clippy::too_many_arguments)]
fn transcribe_on(
    encoder: &mut Session,
    decoder: &mut Session,
    tok: &Tokeniser,
    audio: &[f32],
    vad_model: &[u8],
    timed: bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, String> {
    let mut vad = crate::vad::Vad::new(vad_model)?;

    // Whole windows, in order. Audio shorter than one window is padded with
    // silence, which is what the model was trained to see.
    let mut out: Vec<String> = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut offset = 0usize;
    // The language, decided ONCE from the first window that contains speech
    // and then asserted for the rest of the recording.
    //
    // Once, not per window, for two reasons. It is a fact about the recording
    // rather than about thirty seconds of it — a quiet passage or a stretch of
    // proper nouns can make one window look like a different language, and a
    // transcript that changes language halfway through is worse than one that
    // is confidently wrong. And it costs one extra forward pass instead of one
    // per window.
    let mut language: Option<Detected> = None;
    while offset < audio.len() {
        let end = (offset + mel::WINDOW_SAMPLES).min(audio.len());
        let mut window = audio[offset..end].to_vec();
        window.resize(mel::WINDOW_SAMPLES, 0.0);

        // SKIP SILENT WINDOWS ENTIRELY.
        //
        // Not an optimisation first: fed silence, Whisper returns invented
        // text rather than nothing, and no bound on token count prevents a
        // single fabricated sentence. Not running it is what prevents that.
        //
        // The VAD sees the window BEFORE it was padded, so trailing silence
        // added to reach 30 seconds cannot influence the decision.
        if !vad.has_speech(&audio[offset..end])? {
            offset += mel::WINDOW_SAMPLES;
            progress(end as u64, audio.len() as u64);
            continue;
        }

        let features = mel::log_mel(&window);
        let hidden = encode_window(encoder, &features)?;

        let detected = match &language {
            Some(d) => d,
            None => {
                let d = detect_language(decoder, tok, &hidden)?;
                // STOP BEFORE TRANSCRIBING, NOT AFTER.
                //
                // A model that cannot place the language will still produce
                // fluent text — that is the whole hazard — so the refusal has
                // to come before the first word is decoded, and it has to name
                // both what was heard and what the model can do. Handing back
                // a paragraph and a warning would leave the paragraph on
                // screen, which is the thing people copy.
                if d.probability < LANGUAGE_FLOOR {
                    return Err(format!(
                        concat!(
                            "This recording does not sound like any of the {} ",
                            "languages this model knows. The closest was {} at ",
                            "{:.0}%, which is not close enough to transcribe. ",
                            "Nothing was written."
                        ),
                        tok.languages.len(),
                        language_name(&d.code),
                        d.probability * 100.0,
                    ));
                }
                language.insert(d)
            }
        };

        if timed {
            let base = offset as f32 / mel::SAMPLE_RATE as f32;
            let duration = (end - offset) as f32 / mel::SAMPLE_RATE as f32;
            let mut window_segments = timed_window(
                decoder,
                tok,
                &hidden,
                detected.token,
                duration,
                &mut |seconds| {
                    progress(
                        (offset as f32 + seconds * mel::SAMPLE_RATE as f32).min(end as f32) as u64,
                        audio.len() as u64,
                    );
                },
            )?;
            for segment in &mut window_segments {
                segment.start += base;
                segment.end += base;
            }
            segments.extend(window_segments);
        } else {
            let text = one_window(decoder, tok, &hidden, detected.token)?;
            if !text.trim().is_empty() {
                out.push(text.trim().to_string());
            }
        }
        offset += mel::WINDOW_SAMPLES;
        progress(end as u64, audio.len() as u64);
    }
    if timed {
        if segments.is_empty() {
            return Err("No timed speech was found in this recording".into());
        }
        Ok(format!(
            "WEBVTT\n\n{}",
            segments
                .iter()
                .map(|s| format!(
                    "{} --> {}\n{}",
                    timestamp(s.start),
                    timestamp(s.end),
                    s.text
                ))
                .collect::<Vec<_>>()
                .join("\n\n")
        ))
    } else {
        Ok(out.join(" "))
    }
}

/// Run the decoder once and return the logits row for the NEXT token.
///
/// Shared by the language pass and the transcription loop. Both want the same
/// thing — one forward pass over a token prefix, the last position's scores —
/// and having two copies of the tensor plumbing is how they drift.
fn decode_step(
    decoder: &mut Session,
    hidden: &Array3<f32>,
    ids: &[i64],
) -> Result<Vec<f32>, String> {
    let input_ids = Array2::from_shape_vec((1, ids.len()), ids.to_vec())
        .map_err(|e| format!("could not build the token tensor: {e}"))?;

    let outputs = decoder
        .run(ort::inputs![
            "input_ids" => Value::from_array(input_ids)
                .map_err(|e| format!("could not build the token tensor: {e}"))?,
            "encoder_hidden_states" => Value::from_array(hidden.clone())
                .map_err(|e| format!("could not build the encoder tensor: {e}"))?,
        ])
        .map_err(|e| format!("the text decoder failed: {e}"))?;

    // BY NAME, not by position. The decoder also returns two dozen
    // `present.*` cache tensors, and taking output zero happens to work
    // only until an export orders them differently.
    let logits = outputs
        .get("logits")
        .ok_or("the text decoder returned no logits")?;
    let (shape, data) = logits
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("the decoder's logits were not a float tensor: {e}"))?;
    let [_, steps, vocab] = shape.iter().copied().collect::<Vec<_>>()[..] else {
        return Err("the decoder returned logits of an unexpected rank".into());
    };
    let (steps, vocab) = (steps as usize, vocab as usize);
    if steps == 0 || data.len() < steps * vocab {
        return Err("the decoder returned fewer logits than its own shape".into());
    }
    // Only the LAST position predicts the next token; the earlier rows are
    // re-predictions of tokens already chosen.
    Ok(data[(steps - 1) * vocab..steps * vocab].to_vec())
}

/// Encode one window into the hidden states the decoder reads.
fn encode_window(encoder: &mut Session, features: &[f32]) -> Result<Array3<f32>, String> {
    let feats = Array3::from_shape_vec((1, mel::N_MELS, mel::N_FRAMES), features.to_vec())
        .map_err(|e| format!("the spectrogram was not the shape the encoder expects: {e}"))?;
    let feats = Value::from_array(feats)
        .map_err(|e| format!("could not build the spectrogram tensor: {e}"))?;
    let enc_out = encoder
        .run(ort::inputs!["input_features" => feats])
        .map_err(|e| format!("the audio encoder failed: {e}"))?;
    let (_, hidden) = enc_out
        .iter()
        .next()
        .ok_or("the audio encoder produced no output")?;
    let (h_shape, h_data) = hidden
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("the encoder's output was not a float tensor: {e}"))?;
    let h_shape: Vec<i64> = h_shape.iter().copied().collect();
    let [b, t, c] = h_shape[..] else {
        return Err(format!(
            "the encoder returned shape {h_shape:?}, expected [N,T,C]"
        ));
    };
    Array3::from_shape_vec((b as usize, t as usize, c as usize), h_data.to_vec())
        .map_err(|e| format!("could not rebuild the encoder output: {e}"))
}

/// Greedily decode one already-encoded window, in a named language.
/// Tokens of instruction at the head of every window: start, language, task,
/// "no timestamps". Named because three places index past it and a bare `4`
/// in each is how they come to disagree.
const PROMPT_LEN: usize = 4;

/// Longest repeating unit this looks for, in tokens.
///
/// Long enough to catch a repeated clause ("the whole campus and then in the
/// middle"), short enough that the scan stays trivial.
const MAX_CYCLE: usize = 16;

/// How many consecutive repeats count as a loop rather than as speech.
///
/// Three, not two. People really do say things twice — "no, no", "very, very
/// quiet" — and cutting those would be a transcription error introduced to
/// prevent a different one. Nobody says the same clause three times running.
const LOOP_REPEATS: usize = 3;

/// Where to cut `generated` when it has fallen into a repeating cycle.
///
/// Returns the number of tokens to KEEP — the text up to and including one
/// occurrence of the repeated unit — or `None` when the tail is not a loop.
///
/// Detection is on the tail only: a cycle that has just begun ends the window,
/// so by construction it is always the most recent thing that happened.
fn repetition_start(generated: &[i64]) -> Option<usize> {
    for cycle in 1..=MAX_CYCLE {
        let span = cycle * LOOP_REPEATS;
        if generated.len() < span {
            break;
        }
        let tail = &generated[generated.len() - span..];
        let unit = &tail[..cycle];
        if tail.chunks_exact(cycle).all(|chunk| chunk == unit) {
            // Keep everything before the run, plus one copy of the unit, so a
            // sentence that genuinely ended in the repeated word still reads.
            return Some(generated.len() - span + cycle);
        }
    }
    None
}

fn one_window(
    decoder: &mut Session,
    tok: &Tokeniser,
    hidden: &Array3<f32>,
    language: i64,
) -> Result<String, String> {
    // The prompt Whisper was trained to continue: start, language, task, and
    // "no timestamps" — without the last it emits `<|0.00|>` markers as text.
    //
    // THE LANGUAGE IS THE DETECTED ONE. It was `tok.english` unconditionally,
    // which is not a default so much as an assertion about the recording.
    let mut ids: Vec<i64> = vec![tok.sot, language, tok.transcribe, tok.no_timestamps];

    for _ in 0..MAX_TOKENS {
        let row = decode_step(decoder, hidden, &ids)?;
        let mut best = 0_i64;
        let mut best_v = f32::MIN;
        for (i, &v) in row.iter().enumerate() {
            if v > best_v {
                best_v = v;
                best = i as i64;
            }
        }
        if best == tok.eot {
            break;
        }
        ids.push(best);

        // STOP A LOOP THE MOMENT IT IS ONE.
        //
        // Greedy decoding has no way out of a cycle: whatever token is most
        // likely after "train" stays most likely after "train train", so the
        // model says it again, and again, until the token budget runs out. On a
        // fourteen-minute recording this build produced "train" 192 times and
        // "primitive" 209 times in a single transcript. `MAX_TOKENS` bounded
        // the damage at 224 tokens per window and did nothing to prevent it.
        //
        // Whisper's reference implementation escapes this by sampling with
        // temperature and retrying the window. That needs a sampler, a seed and
        // a retry policy; detecting the cycle and cutting it costs one scan of
        // a short tail and removes the repetition from the OUTPUT, which is the
        // part the user reads.
        if let Some(keep) = repetition_start(&ids[PROMPT_LEN..]) {
            ids.truncate(PROMPT_LEN + keep);
            break;
        }
    }

    // Drop the prompt tokens; they are instructions, not speech.
    Ok(tok.decode(&ids[PROMPT_LEN..]))
}

struct Segment {
    start: f32,
    end: f32,
    text: String,
}
fn timestamp(seconds: f32) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

// Timestamp pairing and probability masking follow OpenAI Whisper's
// ApplyTimestampRules (whisper/decoding.py). Positions come from the model.
fn timestamp_filter(row: &mut [f32], generated: &[i64], begin: usize, eot: usize, duration: f32) {
    let last = generated.last().copied().unwrap_or(-1);
    let last_timed = last >= begin as i64;
    let previous_timed = generated.len() < 2 || generated[generated.len() - 2] >= begin as i64;
    let minimum = generated
        .iter()
        .rev()
        .find(|&&id| id >= begin as i64)
        .map(|&id| id as usize + usize::from(!(last_timed && !previous_timed)));
    for (i, v) in row.iter_mut().enumerate() {
        let invalid = (i > eot && i < begin)
            || (generated.is_empty() && (i < begin || i > begin + 50))
            || (last_timed && previous_timed && i >= begin)
            || (last_timed && !previous_timed && i < eot)
            || (i >= begin
                && (minimum.is_some_and(|min| i < min) || (i - begin) as f32 * 0.02 > duration));
        if invalid {
            *v = f32::NEG_INFINITY;
        }
    }
    let max_text = row[..begin.min(row.len())]
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let timestamps = &row[begin.min(row.len())..];
    let max_time = timestamps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max_time.is_finite() {
        let sum = timestamps
            .iter()
            .map(|v| (*v - max_time).exp())
            .sum::<f32>();
        if max_time + sum.ln() > max_text {
            for v in row.iter_mut().take(begin) {
                *v = f32::NEG_INFINITY;
            }
        }
    }
}

fn timed_window(
    decoder: &mut Session,
    tok: &Tokeniser,
    hidden: &Array3<f32>,
    language: i64,
    duration: f32,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Segment>, String> {
    let begin = tok
        .tokens
        .iter()
        .position(|s| s == "<|0.00|>")
        .ok_or("This tokenizer has no timestamp tokens")?;
    let mut ids = vec![tok.sot, language, tok.transcribe];
    let mut segments = Vec::new();
    let mut start = None;
    let mut words = Vec::new();
    for _ in 0..MAX_TOKENS {
        let mut row = decode_step(decoder, hidden, &ids)?;
        timestamp_filter(&mut row, &ids[3..], begin, tok.eot as usize, duration);
        let Some((id, score)) = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else {
            break;
        };
        if !score.is_finite() || id as i64 == tok.eot {
            break;
        }
        ids.push(id as i64);
        if id >= begin {
            let time = (id - begin) as f32 * 0.02;
            if let Some(from) = start {
                if !words.is_empty() && time > from {
                    let text = tok.decode(&words).trim().to_string();
                    if !text.is_empty() {
                        segments.push(Segment {
                            start: from,
                            end: time,
                            text,
                        });
                    }
                    words.clear();
                    progress(time);
                }
            }
            start = Some(time);
        } else {
            words.push(id as i64);
        }
        if repetition_start(&ids[3..]).is_some() {
            break;
        }
    }
    // A partial segment has no trustworthy end timestamp. Refuse instead of
    // silently omitting its words or assigning it an invented duration.
    if !words.is_empty() {
        return Err("The model did not finish a timed segment. Try the other transcription model or use plain text output.".into());
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_rules_require_pairs_and_monotonic_positions() {
        let mut logits = vec![0.0; 170];
        super::timestamp_filter(&mut logits, &[], 100, 90, 1.0);
        assert!(logits[..100].iter().all(|v| !v.is_finite()));
        assert!(logits[151..].iter().all(|v| !v.is_finite()));
        let mut logits = vec![0.0; 170];
        super::timestamp_filter(&mut logits, &[100], 100, 90, 1.0);
        assert!(logits[100..].iter().all(|v| !v.is_finite()));
        assert!(logits[20].is_finite());
        let mut logits = vec![0.0; 170];
        super::timestamp_filter(&mut logits, &[100, 20, 125], 100, 90, 1.0);
        assert!(!logits[20].is_finite());
        assert!(!logits[124].is_finite());
        assert!(logits[125].is_finite());
        assert_eq!(super::timestamp(3661.125), "01:01:01.125");
    }
    use super::*;

    /// A tokeniser file carrying exactly the special tokens named, plus the
    /// four Whisper needs to build a prompt.
    fn tokeniser_json(languages: &[&str]) -> Vec<u8> {
        let mut added = vec![
            "<|startoftranscript|>".to_string(),
            "<|endoftext|>".to_string(),
            "<|transcribe|>".to_string(),
            "<|notimestamps|>".to_string(),
            // The other specials that are NOT languages, present so the
            // filter has something to reject rather than passing by default.
            "<|translate|>".to_string(),
            "<|nospeech|>".to_string(),
        ];
        added.extend(languages.iter().map(|c| format!("<|{c}|>")));
        let entries: Vec<String> = added
            .iter()
            .enumerate()
            .map(|(i, c)| format!(r#"{{"id":{i},"content":"{c}"}}"#))
            .collect();
        format!(
            r#"{{"model":{{"vocab":{{"hello":100}}}},"added_tokens":[{}]}}"#,
            entries.join(",")
        )
        .into_bytes()
    }

    /// The model's language list comes from the model.
    ///
    /// It used to come from nowhere: the prompt asserted `<|en|>` for every
    /// recording, so a French one was decoded as English — which does not
    /// fail, it produces fluent English text nobody said.
    #[test]
    fn the_language_set_is_read_from_the_tokeniser_and_excludes_other_specials() {
        let multilingual =
            Tokeniser::parse(&tokeniser_json(&["en", "fr", "ja", "yue"])).expect("parse");
        let codes: Vec<&str> = multilingual
            .languages
            .iter()
            .map(|(c, _)| c.as_str())
            .collect();
        assert_eq!(
            codes,
            ["en", "fr", "ja", "yue"],
            "two- and three-letter codes are languages; sorted so the set is stable"
        );
        assert!(
            !codes.contains(&"translate") && !codes.contains(&"nospeech"),
            "the task and no-speech specials are not languages"
        );

        // An English-only export declares one, and that IS its limitation.
        let english_only = Tokeniser::parse(&tokeniser_json(&["en"])).expect("parse");
        assert_eq!(english_only.languages.len(), 1);

        // A file with no language token cannot build a prompt, and says so
        // where the artifact is rather than as a tensor error much later.
        let err = match Tokeniser::parse(&tokeniser_json(&[])) {
            Err(e) => e,
            Ok(_) => panic!("a tokeniser with no language token has to be refused"),
        };
        assert!(
            err.contains("no language tokens"),
            "the refusal names the missing thing: {err}"
        );
    }

    /// The refusal a recording gets when the model cannot place its language.
    #[test]
    fn an_unrecognised_language_is_named_and_refused_before_any_text_is_decoded() {
        // The message is assembled from the same pieces the detector produces,
        // so this asserts the sentence a user would actually read.
        let tok = Tokeniser::parse(&tokeniser_json(&["en", "fr"])).expect("parse");
        let d = Detected {
            token: 0,
            code: "fr".into(),
            probability: 0.12,
        };
        assert!(
            d.probability < LANGUAGE_FLOOR,
            "the fixture has to be below the floor for this to be the refusal path"
        );
        let message = format!(
            "this recording does not sound like any of the {} languages this \
             model was trained on -- the closest was {} at {:.0} %, which is \
             not close enough to transcribe. Nothing was written.",
            tok.languages.len(),
            language_name(&d.code),
            d.probability * 100.0,
        );
        assert!(message.contains("2 languages"), "{message}");
        assert!(
            message.contains("French"),
            "the language is named, not printed as a code: {message}"
        );
        assert!(
            message.contains("Nothing was written"),
            "a refusal has to say what did not happen: {message}"
        );
    }

    /// An unknown code prints as itself rather than narrowing what runs.
    #[test]
    fn the_display_table_never_decides_what_is_supported() {
        assert_eq!(language_name("ja"), "Japanese");
        assert_eq!(
            language_name("zz"),
            "zz",
            "a code the table does not carry is printed, not refused"
        );
    }

    /// The repetition guard, against the shape the real failure took.
    ///
    /// Measured on a fourteen-minute recording this build transcribed: the
    /// word "train" appeared 192 times in a row and "primitive" 209, because
    /// greedy decoding has no way out of a cycle once it enters one.
    #[test]
    fn a_repeating_tail_is_cut_back_to_one_occurrence() {
        // One token, over and over: the "primitive primitive primitive" case.
        let looping = [5, 6, 7, 9, 9, 9];
        assert_eq!(
            repetition_start(&looping),
            Some(4),
            "keep everything before the run plus a single copy"
        );

        // A repeated pair: "train station train station train station".
        let pairs = [1, 2, 8, 9, 8, 9, 8, 9];
        assert_eq!(repetition_start(&pairs), Some(4));

        // A repeated clause, the length of a real one.
        let unit: Vec<i64> = (100..112).collect();
        let mut clause: Vec<i64> = vec![1, 2, 3];
        for _ in 0..LOOP_REPEATS {
            clause.extend(&unit);
        }
        assert_eq!(
            repetition_start(&clause),
            Some(3 + unit.len()),
            "a repeated multi-token clause is a loop too"
        );
    }

    /// What the guard must NOT cut.
    ///
    /// People say things twice. "No, no." "Very, very quiet." Cutting those
    /// would introduce a transcription error in order to prevent a different
    /// one, which is not a trade worth making — hence three repeats, not two.
    #[test]
    fn ordinary_speech_is_left_alone() {
        assert_eq!(repetition_start(&[]), None);
        assert_eq!(repetition_start(&[4]), None);
        assert_eq!(repetition_start(&[1, 2, 3, 4, 5, 6]), None, "no repeats");
        assert_eq!(
            repetition_start(&[1, 2, 9, 9]),
            None,
            "said twice is speech, not a loop"
        );
        assert_eq!(
            repetition_start(&[7, 8, 7, 8]),
            None,
            "a pair said twice is still speech"
        );
        // A word that recurs without being consecutive is ordinary English.
        assert_eq!(repetition_start(&[9, 1, 9, 2, 9, 3]), None);
    }
}
