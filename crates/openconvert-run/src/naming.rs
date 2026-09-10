//! Output naming templates: `{name}` `{ext}` `{date}` `{index}`.
//!
//! The template is a *preference* the user writes; the name it renders is
//! something a filesystem must accept and every other consumer must survive.
//! Two rules make that combination safe:
//!
//! 1. **Validation happens at SET time.** A template that cannot render —
//!    unknown token, empty result, traversal, reserved device name — is
//!    refused when it is configured, not discovered mid-batch where file 40
//!    of 200 would fail on a mistake made weeks earlier.
//! 2. **Every rendered name is re-checked here**, because `{name}` comes from
//!    a file whose name nobody here chose. Rendering is therefore total over
//!    any input filename: the worst a hostile stem can do is fail validation,
//!    never traverse.
//!
//! `{index}` disambiguates batch collisions instead of failing the run; the
//! caller passes the per-batch position and the renderer substitutes it
//! verbatim (1-based).

/// The tokens a template may use.
pub const TOKENS: &[&str] = &["{name}", "{ext}", "{date}", "{index}"];

/// Why a template or a rendered name was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NamingError {
    /// A `{token}` this module does not define.
    // The token arrives WITH its braces (see `validate_template`), so this
    // must not add another pair: it printed `{{{nope}}}` at the user until
    // the multi-file path made the message common enough to notice.
    #[error("unknown token {0}; allowed: {1}")]
    UnknownToken(String, String),
    /// The template rendered to an empty string.
    #[error("the template renders to an empty name")]
    Empty,
    /// A path separator or drive prefix in the result — one `.`-step short of
    /// traversal, refused rather than sanitised.
    #[error("the rendered name {0:?} contains a path separator")]
    PathSeparator(String),
    /// `.` / `..` components.
    #[error("the rendered name {0:?} traverses directories")]
    Traversal(String),
    /// A Windows reserved device name (`CON`, `NUL`, `COM1`…).
    #[error("{0:?} is a reserved device name on Windows")]
    ReservedName(String),
}

/// What the renderer needs beyond the template itself.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    /// The input's file stem, as the source file names it.
    pub name: &'a str,
    /// The output extension, without the dot ("png").
    pub ext: &'a str,
    /// Today's date, ISO order (2026-08-24). Passed in so rendering stays
    /// pure and testable.
    pub date: &'a str,
    /// One-based position in the batch, for collision disambiguation.
    pub index: usize,
}

/// Validate a template WITHOUT rendering it: unknown tokens are refused here
/// so a bad pattern fails at configuration time, not mid-batch.
///
/// # Errors
///
/// [`NamingError::UnknownToken`] for anything outside [`TOKENS`].
pub fn validate_template(template: &str) -> Result<(), NamingError> {
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        // Relative index of the next closing brace.
        let Some(close) = rest[open..].find('}') else {
            // An unclosed brace is text, not a token — filenames may contain
            // braces. Only `{...}` shapes are tokens.
            break;
        };
        // `{}` with nothing inside is literal text, not a broken token.
        if close > 1 {
            let full = &rest[open..=open + close];
            if !TOKENS.contains(&full) {
                return Err(NamingError::UnknownToken(
                    full.to_string(),
                    TOKENS.join(", "),
                ));
            }
        }
        rest = &rest[open + close + 1..];
    }
    Ok(())
}

/// Render the template against a context, then vet the result through the
/// same rules an OutputName must survive.
///
/// # Errors
///
/// See [`NamingError`]. Every variant is a configuration problem a user can
/// fix before running anything.
pub fn render(template: &str, ctx: &RenderContext<'_>) -> Result<String, NamingError> {
    validate_template(template)?;

    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        match &rest[open..=open + close] {
            "{name}" => out.push_str(ctx.name),
            "{ext}" => out.push_str(ctx.ext),
            "{date}" => out.push_str(ctx.date),
            "{index}" => out.push_str(&ctx.index.to_string()),
            other => out.push_str(other),
        }
        rest = &rest[open + close + 1..];
    }
    // Whatever follows the last token — or the whole template when it holds
    // none — is literal text.
    out.push_str(rest);

    check_rendered(&out)?;
    Ok(out)
}

/// The checks every RENDERED name must pass, whoever produced it.
fn check_rendered(name: &str) -> Result<(), NamingError> {
    if name.is_empty() {
        return Err(NamingError::Empty);
    }
    if name.contains('/') || name.contains('\\') || name.contains(':') {
        return Err(NamingError::PathSeparator(name.to_string()));
    }
    if name == "." || name == ".." {
        return Err(NamingError::Traversal(name.to_string()));
    }
    // Windows reserves device names with or without an extension: "CON.png"
    // opens the console on some APIs. The stem alone decides.
    let stem = name.split('.').next().unwrap_or(name);
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(NamingError::ReservedName(name.to_string()));
    }
    Ok(())
}

/// Today's date, ISO order — the value `{date}` renders.
///
/// Read from the clock ONCE per call and pure thereafter, which keeps
/// rendering testable (`RenderContext` takes the date as a value) while the
/// CLI-sized callers get the real answer.
#[must_use]
pub fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    // Civil-from-days (Howard Hinnant's algorithm), same as the receipt
    // store's fallback renderer.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RenderContext<'static> {
        RenderContext {
            name: "photo",
            ext: "png",
            date: "2026-08-24",
            index: 3,
        }
    }

    #[test]
    fn default_template_preserves_the_old_shape() {
        assert_eq!(render("{name}.{ext}", &ctx()).unwrap(), "photo.png");
    }

    #[test]
    fn every_token_substitutes() {
        assert_eq!(
            render("{date}_{name}_{index}.{ext}", &ctx()).unwrap(),
            "2026-08-24_photo_3.png"
        );
    }

    #[test]
    fn plain_text_passes_through_including_braces() {
        // Braces that do not form a known token are literal filename text.
        assert_eq!(
            render("report{}v2.{ext}", &ctx()).unwrap(),
            "report{}v2.png"
        );
        assert!(render("{weird}.png", &ctx()).is_err());
    }

    #[test]
    fn unknown_token_fails_validation_not_rendering() {
        // THE rule: bad patterns die at set time. Both entry points agree.
        let err = validate_template("{width}x{height}.png").unwrap_err();
        assert!(matches!(&err, NamingError::UnknownToken(w, _) if w == "{width}"));
        // The token carries its own braces; the message must not add a
        // second pair around them.
        let shown = err.to_string();
        assert!(
            shown.starts_with("unknown token {width};"),
            "brace escaping regressed: {shown}"
        );
        assert!(render("{width}.png", &ctx()).is_err());
    }

    #[test]
    fn empty_render_refuses() {
        assert!(matches!(render("", &ctx()), Err(NamingError::Empty)));
    }

    #[test]
    fn a_hostile_stem_cannot_traverse() {
        let evil = RenderContext {
            name: r"..\..\windows\system32\evil",
            ext: "png",
            ..ctx()
        };
        assert!(render("{name}.{ext}", &evil).is_err());
    }

    #[test]
    fn reserved_device_names_refuse() {
        for stem in ["CON.png", "nul", "Com1.txt"] {
            let c = RenderContext {
                name: stem.split('.').next().unwrap(),
                ext: "png",
                ..ctx()
            };
            let rendered = stem.to_string();
            assert!(
                check_rendered(&rendered).is_err(),
                "{stem} should be refused"
            );
            let _ = c;
        }
    }

    #[test]
    fn index_disambiguates_without_failing_the_run() {
        // Two files with the same stem render different names by index alone.
        let a = render("{name}_{index}.{ext}", &RenderContext { index: 1, ..ctx() }).unwrap();
        let b = render("{name}_{index}.{ext}", &RenderContext { index: 2, ..ctx() }).unwrap();
        assert_ne!(a, b);
        assert_eq!((a.as_str(), b.as_str()), ("photo_1.png", "photo_2.png"));
    }
}
