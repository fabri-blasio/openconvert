// NEGATIVE FIXTURE -- this file is SUPPOSED to fail a gate.
// Outside the workspace; never compiled. See xtask/src/verify.rs.

// Gate: heavy inference falls back at run time.
//
// The realistic route in: someone adds a new model adapter, copies the two
// lines every other adapter starts with, and gets a session that falls back
// when DirectML is ABSENT and not when it runs out of video memory. That is
// the defect this gate was written after -- `session_for`'s own doc comment
// claimed it covered both for a long time, and nothing noticed, because the
// failure needs a graphics card, a large model and an image big enough to
// exhaust it.
//
// `run_for` is the one that covers a run. Reaching past it has to be said out
// loud on the line, which is what the fixture below deliberately does not do.

pub fn segment_something(model: &[u8], image: &[u8]) -> Result<Vec<u8>, String> {
    let (mut session, _provider) = session_for(model, Workload::Heavy)?;
    let out = session.run(image)?;
    Ok(out)
}
