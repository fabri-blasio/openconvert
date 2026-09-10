//! Prediction properties.
//!
//! This file exists because `predict.rs` landed with **no tests**, and the
//! mutation score fell from 95% to 67% in one commit. Nothing else changed;
//! the module was simply new surface nobody had checked. That is the gate doing
//! the one job fuzzing cannot: telling you the suite stopped covering the code.
//!
//! Prediction is the one part of the product where being *wrong* is cheap and
//! being *unpredictable* is not — a suggestion that changes between two
//! identical drops teaches the user to ignore it.

use openconvert_core::format::FormatId;
use openconvert_core::predict::{
    global_prior, personal_weight, rank, score, should_arm, should_show_lead, FolderCategory,
    Signals, Suggestion, ARM_THRESHOLD, CHOOSER_THRESHOLD,
};
use openconvert_core::target::{Operation, Target};

fn cold(input: FormatId) -> Signals {
    Signals {
        input,
        history_count: 0.0,
        history_to_target: 0.0,
        folder_history_count: 0.0,
        folder_history_to_target: 0.0,
        folder_pattern: 0,
        sibling_same_stem: false,
        batch_size: 1,
        folder: FolderCategory::Unknown,
    }
}

// ---------------------------------------------------------------------------
// The cold-start curve
// ---------------------------------------------------------------------------

/// `w(n) = n / (n + 3)`: the global prior dominates at the first drop, and
/// personal history dominates by roughly the sixth.
///
/// One formula gives the whole curve, and the constant 3 is the only tunable
/// number in the module — deliberately, because a model with many knobs needs
/// calibration data that does not exist until the product ships.
#[test]
fn the_blend_moves_from_global_to_personal() {
    assert_eq!(
        personal_weight(0.0),
        0.0,
        "at drop 1 the prior must be everything"
    );
    assert!(
        personal_weight(3.0) > 0.49 && personal_weight(3.0) < 0.51,
        "n=3 is the half-way point"
    );
    assert!(
        personal_weight(6.0) > 0.66,
        "by drop 6 personal history should dominate"
    );
    assert!(personal_weight(100.0) > 0.97);

    // Monotonic: more evidence never means less personal weight.
    let mut prev = -1.0_f32;
    for n in 0..50 {
        let w = personal_weight(n as f32);
        assert!(w > prev, "weight decreased at n={n}");
        assert!((0.0..1.0).contains(&w));
        prev = w;
    }
}

/// The prior is a table, so it is inspectable and diffable rather than emergent.
#[test]
fn the_prior_is_a_real_table_not_a_constant() {
    assert!(
        global_prior(FormatId::Heic, FormatId::Jpeg) > 0.8,
        "the flagship conversion"
    );
    assert!(global_prior(FormatId::Mkv, FormatId::Mp4) > 0.7);
    // Something nobody does.
    assert!(global_prior(FormatId::Csv, FormatId::Tiff) < 0.1);
    // The control: not every pair returns the same number.
    assert_ne!(
        global_prior(FormatId::Heic, FormatId::Jpeg),
        global_prior(FormatId::Jpeg, FormatId::Png),
        "the prior table is flat, so it carries no information"
    );
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// A FOLDER PATTERN is near-certainty; the same file's own output is not.
///
/// These were one signal, floored at 0.90 -- above `ARM_THRESHOLD`, so it ran
/// on Enter without asking. It matched on the stem, so what it actually
/// observed was "this file has already been converted", and converting it
/// again either fails or writes `photo (2).jpg`.
///
/// The two are separated now. Other files in the folder having been converted
/// this way predicts what to do with THIS one and still arms at three
/// examples; this file already having been done leads but no longer arms.
#[test]
fn the_folder_pattern_arms_and_the_same_stem_does_not() {
    let mut s = cold(FormatId::Tiff);
    // Weak means "the chooser takes over", which is the behaviour that
    // matters, rather than a bare number that moves whenever the prior does.
    let weak = score(&s, Target::Format(FormatId::Gif));
    assert!(
        weak < CHOOSER_THRESHOLD,
        "tiff -> gif should not lead on its own, got {weak}"
    );

    // This exact file, already converted once: enough to lead, not to arm.
    s.sibling_same_stem = true;
    let same_stem = score(&s, Target::Format(FormatId::Gif));
    assert!(
        same_stem > weak,
        "an existing output beside the file should still count for something"
    );
    assert!(
        same_stem < ARM_THRESHOLD,
        "this file having been converted once must not auto-run it again: {same_stem}"
    );
    assert!(
        should_show_lead(&Suggestion {
            target: Target::Format(FormatId::Gif),
            score: same_stem
        }),
        "it should still lead the ranking: {same_stem}"
    );

    // The folder doing it repeatedly IS evidence about a new file.
    let mut folder = cold(FormatId::Tiff);
    folder.folder_pattern = 3;
    let pattern = score(&folder, Target::Format(FormatId::Gif));
    assert!(
        pattern >= 0.90,
        "three converted pairs in the folder did not arm: {pattern}"
    );
    assert!(pattern <= 1.0, "score exceeded 1.0: {pattern}");

    // And it is graded, so one example is not three.
    let mut one = cold(FormatId::Tiff);
    one.folder_pattern = 1;
    let single = score(&one, Target::Format(FormatId::Gif));
    assert!(
        single < pattern,
        "one example should be weaker than three: {single} vs {pattern}"
    );
}

/// Personal history overrides the global prior once there is enough of it.
#[test]
fn personal_history_overrides_the_prior() {
    // Globally, png -> jpeg is common. This user always goes to webp.
    let s = Signals {
        input: FormatId::Png,
        history_count: 20.0,
        history_to_target: 20.0,
        ..cold(FormatId::Png)
    };
    let webp = score(&s, Target::Format(FormatId::Webp));

    let no_history = score(&cold(FormatId::Png), Target::Format(FormatId::Webp));
    assert!(
        webp > no_history,
        "20 consistent uses did not move the score ({no_history} -> {webp})"
    );
    assert!(
        webp > 0.8,
        "a unanimous personal history should be strong: {webp}"
    );
}

/// Folder intent nudges, and only for the pairing it is about.
#[test]
fn folder_category_nudges_the_right_target_only() {
    let plain = score(&cold(FormatId::Png), Target::Format(FormatId::Jpeg));

    let shots = Signals {
        folder: FolderCategory::Screenshots,
        ..cold(FormatId::Png)
    };
    assert!(
        score(&shots, Target::Format(FormatId::Jpeg)) > plain,
        "the Screenshots folder did not favour compression"
    );
    // And leaves an unrelated target alone.
    let unrelated_plain = score(&cold(FormatId::Png), Target::Format(FormatId::Tiff));
    assert!(
        (score(&shots, Target::Format(FormatId::Tiff)) - unrelated_plain).abs() < 1e-6,
        "the folder multiplier leaked onto an unrelated target"
    );
}

/// A batch favours format conversion over a one-off operation.
#[test]
fn a_batch_deprioritises_single_file_operations() {
    let single = Signals {
        batch_size: 1,
        ..cold(FormatId::Jpeg)
    };
    let batch = Signals {
        batch_size: 200,
        ..cold(FormatId::Jpeg)
    };
    let op = Target::Operation(Operation::StripMetadata);
    assert!(
        score(&batch, op) < score(&single, op),
        "a 200-file drop did not deprioritise a one-off operation"
    );
}

// ---------------------------------------------------------------------------
// Totality — this is fuzzed, so these are the invariants the fuzzer asserts
// ---------------------------------------------------------------------------

/// Never `NaN`, never outside `0..=1`, whatever the inputs.
///
/// A `NaN` score sorts unpredictably, so the suggestion shown would depend on
/// the sort implementation rather than on the evidence.
#[test]
fn score_is_total_over_hostile_inputs() {
    let hostile = [
        (0.0, 0.0),
        (0.0, f32::MAX), // more conversions to a target than conversions
        (f32::MAX, f32::MAX),
        (f32::MAX, 0.0),
        (1.0, 1_000_000.0),
        // The weights come off a disk this module does not control, so the
        // ones a hand-edited journal or a clock change could produce are here
        // too. A `NaN` weight must not become a `NaN` score.
        (f32::NAN, 1.0),
        (1.0, f32::NAN),
        (f32::INFINITY, f32::INFINITY),
        (-1.0, -1.0),
    ];
    for (count, to_target) in hostile {
        for &sib in &[true, false] {
            let s = Signals {
                input: FormatId::Png,
                history_count: count,
                history_to_target: to_target,
                folder_history_count: to_target,
                folder_history_to_target: count,
                folder_pattern: u32::MAX,
                sibling_same_stem: sib,
                batch_size: u32::MAX,
                folder: FolderCategory::Scans,
            };
            let v = score(&s, Target::Format(FormatId::Jpeg));
            assert!(v.is_finite(), "score was {v} for {s:?}");
            assert!((0.0..=1.0).contains(&v), "score {v} out of range for {s:?}");
        }
    }
}

/// Identical signals give an identical answer.
///
/// Stated as a test because the alternative is a UI that flickers between two
/// identical drops, which erodes trust faster than a wrong guess does.
#[test]
fn scoring_is_deterministic() {
    let s = Signals {
        history_count: 7.0,
        history_to_target: 3.0,
        ..cold(FormatId::Webp)
    };
    let first = score(&s, Target::Format(FormatId::Png));
    for _ in 0..100 {
        assert_eq!(score(&s, Target::Format(FormatId::Png)), first);
    }
}

// ---------------------------------------------------------------------------
// Ranking and arming
// ---------------------------------------------------------------------------

/// Ranking is descending, and ties keep their input order.
#[test]
fn ranking_is_descending_and_stable() {
    let s = cold(FormatId::Heic);
    let candidates = [
        Target::Format(FormatId::Jpeg),
        Target::Format(FormatId::Png),
        Target::Format(FormatId::Tiff),
        Target::Format(FormatId::Gif),
    ];
    let ranked = rank(&candidates, |_| s);

    assert_eq!(
        ranked.len(),
        candidates.len(),
        "ranking dropped a candidate"
    );
    for w in ranked.windows(2) {
        assert!(w[0].score >= w[1].score, "ranking is not descending");
    }
    assert_eq!(
        ranked[0].target,
        Target::Format(FormatId::Jpeg),
        "heic should rank jpeg first"
    );

    // TIED CANDIDATES KEEP THE ORDER THEY WERE GIVEN.
    //
    // This used to pair two formats that both fell through to the flat 0.05
    // fallback. There is no flat fallback any more -- that was the point of
    // the change -- so a tie now has to be constructed rather than assumed,
    // and the test asserts it IS a tie before asserting what a tie does.
    let tied = [Target::Format(FormatId::Cr2), Target::Format(FormatId::Nef)];
    let scores: Vec<f32> = tied.iter().map(|&t| score(&s, t)).collect();
    assert_eq!(
        scores[0], scores[1],
        "this test needs two candidates that genuinely tie"
    );
    let r = rank(&tied, |_| s);
    assert_eq!(
        r[0].target, tied[0],
        "a tie was reordered, so the UI would flicker"
    );
    // And the reverse order gives the reverse answer, which is what makes the
    // first assertion about stability rather than about luck.
    let flipped = [tied[1], tied[0]];
    assert_eq!(rank(&flipped, |_| s)[0].target, flipped[0]);
}

/// The arming threshold is a real boundary in both directions.
///
/// A wrong armed suggestion costs more than an absent one: the user has to
/// notice it, undo it, and then stop trusting the feature.
#[test]
fn arming_is_bounded_on_both_sides() {
    let just_under = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: ARM_THRESHOLD - 0.01,
    };
    let exactly = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: ARM_THRESHOLD,
    };
    let over = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: 0.99,
    };
    assert!(!should_arm(&just_under), "armed below the threshold");
    assert!(should_arm(&exactly), "refused to arm AT the threshold");
    assert!(should_arm(&over));
    // Compile-time, not runtime: the value is a const, so a runtime assert on
    // it can never fail at a moment anyone is watching. This fails the BUILD if
    // someone sets the threshold somewhere that is not a meaningful confidence.
    const _: () = assert!(
        ARM_THRESHOLD > 0.5 && ARM_THRESHOLD < 1.0,
        "ARM_THRESHOLD must be a real confidence: above a coin flip, below certainty"
    );
}

/// A cold, unremarkable pairing does **not** arm.
///
/// The control for the threshold: without it, `should_arm` returning `true`
/// unconditionally would satisfy every other test in this file.
#[test]
fn a_weak_guess_is_shown_but_not_armed() {
    let s = cold(FormatId::Tiff);
    let sug = Suggestion {
        target: Target::Format(FormatId::Gif),
        score: score(&s, Target::Format(FormatId::Gif)),
    };
    assert!(
        !should_arm(&sug),
        "a 0.05-confidence guess was armed ({})",
        sug.score
    );
}

/// I8, restated where prediction can see it: a suggestion is a **target**, not
/// a plan.
///
/// Nothing here produces a `Plan`, so nothing here can arm a Class D
/// operation — `route()` filters on `max_auto_class` regardless of what
/// suggested the target. This test asserts the type-level fact, which is the
/// only place it can be asserted cheaply.
#[test]
fn prediction_produces_targets_and_never_plans() {
    let ranked = rank(&[Target::Format(FormatId::Jpeg)], |_| cold(FormatId::Png));
    let _: Target = ranked[0].target;
    // If `Suggestion` ever grows a `Plan`, this file stops compiling — which is
    // the intended alarm.
}

/// The blend is exactly `prior * (1 - w) + personal * w`.
///
/// Four mutants survived inside that one expression — the division that forms
/// the personal ratio flipped to a multiplication, and each operator in the
/// weighted sum flipped in turn. Every one changes what the product suggests,
/// and none changed a test result, because the tests above assert *directions*
/// and never the arithmetic itself.
#[test]
fn the_blend_is_the_stated_formula() {
    // n = 3 puts the weight at exactly 0.5, so the expected value is the plain
    // average of prior and personal — chosen because it is checkable by eye.
    let s = Signals {
        input: FormatId::Png,
        history_count: 3.0,
        history_to_target: 3.0, // personal = 1.0
        ..cold(FormatId::Png)
    };
    let prior = global_prior(FormatId::Png, FormatId::Jpeg); // 0.55
    let w = personal_weight(3.0); // 0.5
    let expected = prior * (1.0 - w) + 1.0 * w;

    let got = score(&s, Target::Format(FormatId::Jpeg));
    assert!(
        (got - expected).abs() < 1e-5,
        "blend is {got}, formula says {expected}"
    );

    // Half the history went elsewhere: the personal term halves, and so does
    // its contribution. This is the assertion the `/` -> `*` mutant fails.
    let split = Signals {
        history_to_target: 1.0,
        ..s
    };
    let expected_split = prior * (1.0 - w) + (1.0 / 3.0) * w;
    let got_split = score(&split, Target::Format(FormatId::Jpeg));
    assert!(
        (got_split - expected_split).abs() < 1e-5,
        "split history gives {got_split}, formula says {expected_split}"
    );
    assert!(got_split < got, "less consistent history scored higher");
}

/// Every pair in the prior table is reachable and distinct from the fallback.
///
/// Mutants deleted individual match arms and nothing objected, because no test
/// touched those rows. The table is the *product's cold-start behaviour* — a
/// silently deleted row means a first-time user sees a worse first guess, which
/// is precisely the moment the guess matters most.
#[test]
fn every_prior_row_is_reachable() {
    const FALLBACK: f32 = 0.05;
    let rows = [
        (FormatId::Heic, FormatId::Jpeg),
        (FormatId::Png, FormatId::Jpeg),
        (FormatId::Jpeg, FormatId::Png),
        (FormatId::Webp, FormatId::Png),
        (FormatId::Webp, FormatId::Jpeg),
        (FormatId::Mkv, FormatId::Mp4),
        (FormatId::Webm, FormatId::Mp4),
        (FormatId::Flac, FormatId::Mp3),
        (FormatId::Wav, FormatId::Mp3),
        (FormatId::Wav, FormatId::Flac),
        (FormatId::Docx, FormatId::Pdf),
        (FormatId::Odt, FormatId::Pdf),
        (FormatId::Csv, FormatId::Json),
    ];
    for (from, to) in rows {
        let p = global_prior(from, to);
        assert!(
            p > FALLBACK,
            "{from} -> {to} fell through to the fallback; its row is gone"
        );
    }
    // The control: an unlisted pair falls through to the DERIVED prior, which
    // is what replaced the flat 0.05. Bmp -> Ogg is cross-kind and Ogg is a
    // middling destination, so it lands low -- but it is no longer tied with
    // every other unlisted pair, which was the whole problem.
    let unlisted = global_prior(FormatId::Bmp, FormatId::Ogg);
    assert!(
        unlisted < 0.2,
        "an image -> audio pair should stay weak: {unlisted}"
    );
    assert!(
        (unlisted - global_prior(FormatId::Bmp, FormatId::Mp3)).abs() > 1e-6,
        "unlisted pairs must not all score the same; that was the defect"
    );
}

/// The Scans folder favours PDF, and only PDF.
#[test]
fn the_scans_folder_favours_pdf() {
    let scans = Signals {
        folder: FolderCategory::Scans,
        ..cold(FormatId::Png)
    };
    let plain = cold(FormatId::Png);
    assert!(
        score(&scans, Target::Format(FormatId::Pdf)) > score(&plain, Target::Format(FormatId::Pdf)),
        "the Scans folder did not favour PDF"
    );
}

/// The confidence contract, as `02` �12.2 states it.
///
/// =85% arms; below 60% shows a chooser. The two thresholds are exported
/// precisely so the interface layer reads them instead of hard-coding its own
/// numbers � which means the contract is only real if the values here match
/// the spec, and if arming sits strictly above choosing.
#[test]
fn the_confidence_contract_matches_the_spec() {
    // 02-FEATURES §12.2: "≥85% arms the top suggestion; <60% shows a chooser".
    // The values themselves are consts read from the module, so pinning them
    // here would be a tautology; what this pins is the CONTRACT SHAPE — that
    // arming sits strictly above choosing, and that each band behaves as the
    // spec says.

    let at_arm = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: ARM_THRESHOLD,
    };
    let under_arm = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: ARM_THRESHOLD - 0.01,
    };
    assert!(should_arm(&at_arm), "arming is inclusive of the threshold");
    assert!(!should_arm(&under_arm));

    let lead = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: CHOOSER_THRESHOLD,
    };
    let between = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: (CHOOSER_THRESHOLD + ARM_THRESHOLD) / 2.0,
    };
    assert!(should_show_lead(&lead), "a score AT the chooser line leads");
    assert!(
        should_show_lead(&between) && !should_arm(&between),
        "the band between chooser and arm shows a ranked lead but never auto-runs"
    );
    let below = Suggestion {
        target: Target::Format(FormatId::Jpeg),
        score: CHOOSER_THRESHOLD - 0.01,
    };
    assert!(
        !should_show_lead(&below) && !should_arm(&below),
        "below the chooser line nothing leads and nothing arms"
    );
}

// ---------------------------------------------------------------------------
// The derived prior, and folder-local history
// ---------------------------------------------------------------------------

/// Every routable pair used to score 0.05 unless someone hand-wrote a row.
///
/// Thirteen pairs had rows; the route table now carries well over a hundred.
/// So for most inputs every candidate tied at 0.05 -- under
/// `CHOOSER_THRESHOLD`, which means the format picker rather than a
/// suggestion, and tied, which means the order was whatever `routable_targets`
/// happened to return.
///
/// This asserts the property that fixes it: unlisted pairs are *ordered*.
#[test]
fn unlisted_pairs_are_ranked_rather_than_tied() {
    // None of these three is in the hand table.
    let gif_to_png = global_prior(FormatId::Gif, FormatId::Png);
    let gif_to_webp = global_prior(FormatId::Gif, FormatId::Webp);
    let gif_to_bmp = global_prior(FormatId::Gif, FormatId::Bmp);

    assert!(
        gif_to_webp > gif_to_bmp,
        "webp is a destination people choose and bmp is not: {gif_to_webp} vs {gif_to_bmp}"
    );
    assert!(
        gif_to_png > gif_to_bmp,
        "png beats bmp as a destination: {gif_to_png} vs {gif_to_bmp}"
    );

    // A camera raw is a source, essentially never a target.
    let to_raw = global_prior(FormatId::Jpeg, FormatId::Cr3);
    assert!(
        to_raw < gif_to_bmp,
        "converting TO a camera raw should rank below everything: {to_raw}"
    );
}

/// Cross-kind stays weak, but never zero.
///
/// `pdf -> png`, OCR and transcription are all real routes. A hard zero would
/// rank them below pairs that do not route at all, and they would never be
/// offered.
#[test]
fn cross_kind_is_weak_but_reachable() {
    let cross = global_prior(FormatId::Pdf, FormatId::Png);
    let same = global_prior(FormatId::Gif, FormatId::Png);
    assert!(cross > 0.0, "a real cross-kind route must not score zero");
    assert!(
        cross < same,
        "cross-kind should rank below a same-kind conversion: {cross} vs {same}"
    );
}

/// Going lossy -> lossless is discouraged; the reverse is the normal reason to
/// convert.
///
/// The hand table encoded exactly this for one pair -- `png->jpeg` at 0.55
/// against `jpeg->png` at 0.30. The derived prior applies it to every pair
/// instead of the one somebody wrote down.
#[test]
fn the_lossy_asymmetry_generalises() {
    // Neither direction is in the hand table.
    let lossless_to_lossy = global_prior(FormatId::Tiff, FormatId::Webp);
    let lossy_to_lossless = global_prior(FormatId::Gif, FormatId::Tiff);
    assert!(
        lossless_to_lossy > lossy_to_lossless,
        "a PNG of a JPEG is bigger and no more detailed; \
         the ranking should say so: {lossless_to_lossy} vs {lossy_to_lossless}"
    );
}

/// A folder's own habit beats the same user's habit everywhere else.
///
/// The signal a single global counter could not express: someone who sends
/// `~/Scans` to PDF and `~/Web` to WebP has two habits, and one tally averages
/// them into a third that is neither.
#[test]
fn a_folder_habit_outweighs_the_global_one() {
    // Globally this user converts png to jpeg, every time.
    let mut s = cold(FormatId::Png);
    s.history_count = 40.0;
    s.history_to_target = 40.0;
    let jpeg_global = score(&s, Target::Format(FormatId::Jpeg));
    assert!(
        jpeg_global > ARM_THRESHOLD,
        "forty conversions should arm: {jpeg_global}"
    );

    // In THIS folder they have gone to webp instead, several times. The jpeg
    // suggestion must give way, without the folder needing forty of its own.
    let mut here = s;
    here.history_to_target = 40.0; // still the global jpeg habit
    here.folder_history_count = 8.0;
    here.folder_history_to_target = 0.0; // none of them went to jpeg
    let jpeg_here = score(&here, Target::Format(FormatId::Jpeg));
    assert!(
        jpeg_here < jpeg_global,
        "the folder's habit did not move the score: {jpeg_here} vs {jpeg_global}"
    );
    assert!(
        jpeg_here < ARM_THRESHOLD,
        "a settled contrary folder habit should stop this arming: {jpeg_here}"
    );
}

/// One visit to a folder does not overturn a hundred conversions elsewhere.
///
/// The blend is weighted by how much evidence each level has, which is what
/// keeps the folder from being a switch rather than a signal.
#[test]
fn a_folder_seen_once_does_not_erase_a_settled_habit() {
    let mut s = cold(FormatId::Png);
    s.history_count = 100.0;
    s.history_to_target = 100.0;
    let settled = score(&s, Target::Format(FormatId::Jpeg));

    let mut once = s;
    once.folder_history_count = 1.0;
    once.folder_history_to_target = 0.0;
    let after = score(&once, Target::Format(FormatId::Jpeg));

    assert!(
        after < settled,
        "one contrary example should count for something"
    );
    assert!(
        after > CHOOSER_THRESHOLD,
        "but it must not demote a hundred conversions to a coin flip: {after}"
    );
}

/// Evidence weights are continuous, and half a conversion is not a conversion.
///
/// History is decayed by age now, so these arrive as fractions. The curve has
/// to stay monotonic across them or a habit could get *stronger* as it aged.
#[test]
fn decayed_weights_rank_monotonically() {
    let mut prev = -1.0_f32;
    for step in 0..40 {
        let w = step as f32 * 0.25;
        let mut s = cold(FormatId::Png);
        s.history_count = w;
        s.history_to_target = w;
        let v = score(&s, Target::Format(FormatId::Jpeg));
        assert!(v.is_finite());
        assert!(
            v >= prev - 1e-6,
            "score fell as evidence grew: {v} after {prev} at weight {w}"
        );
        prev = v;
    }
}
