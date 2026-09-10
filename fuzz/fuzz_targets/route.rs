//! Routing over arbitrary inputs.
//!
//! Free, because the core is pure: no files, no setup, no teardown. And a
//! panic in `route()` is a DoS in the CLI, the GUI and the browser build
//! simultaneously, since all three call this one function.
#![no_main]
use libfuzzer_sys::fuzz_target;
use openconvert_core::environment::Environment;
use openconvert_core::facts::Properties;
use openconvert_core::format::TABLE;
use openconvert_core::isolation::SandboxProfile;
use openconvert_core::plan::PlanRequest;
use openconvert_core::policy::Policy;
use openconvert_core::route::{route, RouteTable};
use openconvert_core::target::Target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 10 {
        return;
    }
    let pick = |i: usize| TABLE[data[i] as usize % TABLE.len()].id;
    let env = Environment::new(
        SandboxProfile::unconfined(),
        Vec::new(),
        RouteTable::v1(),
    );
    let props = Properties::Image {
        width: u32::from(data[2]) << 8 | u32::from(data[3]),
        height: u32::from(data[4]) << 8 | u32::from(data[5]),
        has_alpha: data[6] & 1 == 1,
        frames: u32::from(data[7]),
    };
    let plan = route(
        PlanRequest {
            input: pick(0),
            target: Target::Format(pick(1)),
            // Fuzz the flag too: a polyglot must yield no steps whatever else
            // the input says, and that is asserted below.
            polyglot: data[8] & 1 == 1,
        },
        props,
        &Policy::default(),
        &env,
    );
    // The invariant, asserted inside the fuzzer: a blocked plan has no steps.
    // Fuzzing that only checks for panics misses a plan that is wrong rather
    // than absent.
    if plan.warnings().iter().any(|w| w.is_blocking()) {
        assert!(plan.steps().is_empty());
    }
    // A6, unconditionally: no machine configuration makes a polyglot safe.
    if data[8] & 1 == 1 {
        assert!(plan.steps().is_empty(), "a polyglot produced steps");
    }
});
