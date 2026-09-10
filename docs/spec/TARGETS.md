# Target matrix — decided

**Week 0 decision.** Required by [08 §1](08-EXECUTION-PLAN.md#1-week-0--before-any-code), added in v0.6 because CPU architecture appeared in **zero** live design documents while every confinement mechanism in the design is architecture-specific.

---

## The two lists are not the same list

This is the whole point of the document. A target that **builds** is not a target whose **confinement has been verified**, and conflating them is how a green CI matrix comes to mean nothing.

| Target | Builds | Tested | Confinement verified | Ships at 1.0 |
|---|---|---|---|---|
| `x86_64-pc-windows-msvc` | ✅ | ✅ | ✅ **measured** | ✅ |
| `x86_64-unknown-linux-gnu` | ✅ | ✅ | ⚠️ **WSL2 kernel only** | ✅ |
| `aarch64-unknown-linux-gnu` | ✅ | ⚠️ emulated | ❌ **never executed on real silicon** | ✅ |
| `x86_64-apple-darwin` | ✅ | ❌ | ❌ **written, never executed** | ✅ |
| `aarch64-apple-darwin` | ✅ | ❌ | ❌ **written, never executed** | ✅ |
| `wasm32-unknown-unknown` | ✅ | ✅ | n/a — browser origin | tool pages only |

**Four of six ship with unverified confinement.** That is stated here rather than discovered at week 14, and the runbooks that would close it are in the private platform validation runbooks.

> **macOS moved one square, 2026-08-26.** It was *"zero measurements"* because there was also zero code: `available::probe()` returned `unprobed()`, which fails the isolation floor, so a macOS build launched and refused every file it was given. `crates/openconvert-os/src/macos.rs` now implements the confinement — a Seatbelt `pure-computation` profile applied by the worker to itself, with the same read-back rule as Linux, so a profile that fails to engage reports `false` and the floor refuses the conversion exactly as before.
>
> **This is not a measurement and must not be read as one.** The module typechecks for both Darwin targets in CI (`cross-typecheck`) and has never run on a Mac. Task T1 in the private macOS validation runbook still settles it, and it is still unrun.

---

## The rule that makes this table honest

> **Emulation validates compilation and program logic. It never validates confinement.**

Not a principle — a measured result. `docker buildx` advertises `linux/arm64` while being unable to *execute* it until binfmt is registered, and under emulation the Landlock syscalls returned `ENOSYS` from a kernel that had served them correctly to an amd64 container minutes earlier (S12b).

So: an emulated arm64 run may satisfy the **Tested** column and may never satisfy **Confinement verified**. CI enforces this by refusing to report a confinement result from a runner where `systemd-detect-virt` reports instruction emulation.

---

## Why architecture is load-bearing here

Three concrete hazards, each measured:

| Hazard | Detail |
|---|---|
| **`AUDIT_ARCH` differs** | `AUDIT_ARCH_X86_64` = `0xc000003e`, `AUDIT_ARCH_AARCH64` = `0xc00000b7`. A seccomp filter that does not validate the arch field of `seccomp_data` is a textbook bypass |
| **Syscall numbers differ** | `__NR_socket` is **41** on x86_64 and **198** on aarch64. A filter with hardcoded numbers silently protects nothing on the other architecture |
| **`c_char` signedness differs** | signed on x86_64, unsigned on aarch64. **One of our own spikes compiled for one and failed to compile for the other** (S15) — the architecture problem in miniature, and the reason `std::ffi::c_char` is mandatory rather than preferred |

---

## Support floor

| | Value | Why |
|---|---|---|
| **MSRV** | **1.90** | Pinned in `rust-toolchain.toml`. Raising it is a commit of its own with a stated reason |
| **Linux kernel** | **≥ 5.13** *and* **runtime Landlock ABI negotiation** | 5.13 is when Landlock landed. The negotiation is not optional: the kernel we tested reported **ABI 3, not 4**, and passing an access bit it does not know returns `EINVAL` — so a filter built against the newest ABI silently fails to apply on a slightly older kernel (S10b) |
| **Windows** | **10 1809+** | AppContainer plus the mitigation policies the `Full` tier names |
| **macOS** | **12+** *(provisional)* | Unverified, and now also **implemented against**: `bundle.macOS.minimumSystemVersion` is `12.0` and `openconvert-os::macos` calls `sandbox_init`, deprecated since 10.8. Whether it still confines a sidecar under a hardened runtime is task T1 in the private macOS validation runbook |

## Engine linkage

**Dynamic, on every target, without exception.** Recorded here because it is a week-0 decision that binds packaging, code signing and the LGPL position simultaneously — see [03 D19](03-ARCHITECTURE.md#16-decisions) and `engines.toml`.

---

## What week 0 could not settle

Two items in [08 §1](08-EXECUTION-PLAN.md#1-week-0--before-any-code) are **not** engineering and remain open:

- **Trademark search, Nice classes 9 and 42.** `01 §10` says a conflict there means *stop and revisit*, with runners-up already chosen. Changing a name now costs a day; in month six it costs a rebrand.
- **Registry reservations** — GitHub org, crates.io, npm, PyPI, Docker Hub.

Neither can be done from a repository. **They gate the name, not the code**, so building continues — but they should close before anything is published under it.
