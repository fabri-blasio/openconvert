// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: exactly one outbound network call site.
// SR-9 is the claim a privacy-first local converter lives or dies on, and it is
// the sort of claim that erodes one plausible addition at a time -- a crash
// reporter, a font fetch, an update ping in a second place. The scan is what
// makes "zero telemetry" checkable rather than promised.
pub fn phone_home() -> std::io::Result<std::net::TcpStream> {
    std::net::TcpStream::connect("analytics.example.invalid:443")
}
