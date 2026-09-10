//! Build-time gates. `cargo xtask <command>`.
//!
//! Every gate here fails the build. Every one also has a **negative fixture**
//! under `xtask/fixtures/`, and `cargo xtask verify-gates` asserts each fixture
//! actually fails -- because a gate nobody has watched fail is a gate nobody has
//! tested. That is not a hypothetical concern: spike S30 found a corpus test in
//! this project that passed vacuously when its own input was deleted, after
//! 7.4 million fuzz cases had failed to notice.

mod bundle;
mod ci;
mod depcheck;
mod deps;
mod desktop;
mod engines;
mod lint;
mod manifest;
mod routes;
mod verify;

fn main() -> anyhow::Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "ci" => ci::run(),
        "bundle-check" => bundle::run(),
        "deps" => deps::run(),
        "manifest" => manifest::run(),
        "depcheck" => depcheck::run(),
        "engines" => engines::run(),
        "lint" => lint::run(),
        "desktop" => desktop::run(),
        "verify-gates" => verify::run(),
        "routes" => routes::run(std::env::args().nth(2).as_deref() == Some("--check")),
        "all" => {
            // Manifest first: if a referenced file is missing, every gate after
            // it is reporting on an incomplete tree.
            manifest::run()?;
            depcheck::run()?;
            lint::run()?;
            desktop::run()?;
            verify::run()?;
            // Last, because it is a documentation gate rather than a security
            // one: a stale reference should not mask a failing sandbox check.
            routes::run(true)
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <command>\n\
                 \n\
                 \x20 deps           fetch the native libraries, pinned by sha256\n\
                 \x20 ci             every step the GitHub workflow runs, in order\n\
\x20 bundle-check   the staged engines are the release ones, before packaging\n\
                 \x20 all            the gates below, without the supply-chain steps\n\
                 \x20 manifest       every file the build references exists\n\
                 \x20 depcheck       no boundary crate links a native library\n\
                 \x20 lint           the six source scans\n\
                 \x20 desktop        the webview boundary: plugins, capabilities, CSP\n\
                 \x20 verify-gates   each gate fires on a fixture built to trip it\n\
                 \x20 routes         regenerate docs/ROUTES.md (--check to verify)"
            );
            std::process::exit(2);
        }
    }
}
