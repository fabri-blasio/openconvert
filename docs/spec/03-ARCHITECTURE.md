# OpenConvert — Architecture

How it's built: the smallest design that delivers the promise, and the seams that let it grow.

Status: design record, **v0.6 — the measured revision** · 2026-08-14
Part 3 of 9 — see [README](README.md). Threat model: [09-THREAT-MODEL](09-THREAT-MODEL.md).

> ### v0.6 — what 41 measurements changed
>
> Every previous revision of this document was written by reasoning about the design. **This one was written against 42 spikes, 41 of them run** — real code, on real machines, testing whether the claims here are true. **Nineteen predictions were refuted**, and the full change order with per-claim evidence is [DESIGN-DELTAS.md](DESIGN-DELTAS.md).
>
> Four claims in the v0.5 record were **false**, not merely imprecise: `#![forbid(unsafe_code)]` does not prevent linking a C library ([§5](#5-five-crates-and-the-engine-workers)); the core could not have been pure as described, because it held a file handle ([§11.3](#113-input-identity)); `expansion_ratio` cannot separate an archive bomb from a disk image ([§10.1](#101-per-step)); and the filename rule rejected five of ten ordinary filenames ([§8.2](#82-outbound)).
>
> Two things this revision corrects that *the measurement work itself* got wrong first — recorded because a document that only reports its successes is not evidence: an earlier write-up claimed ACG silently declines (it was measured on a plain process, which this product never creates — [§9.2](#92-the-ladder-re-derived-from-what-is-actually-reachable)), and a review finding about `O_EXCL` weakening on non-local filesystems was refuted by test ([09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against)).
>
> **The single most consequential result:** Windows and Linux are safe in *opposite places*, and one model cannot describe both ([§9.2](#92-the-ladder-re-derived-from-what-is-actually-reachable)).
>
> **What is still unmeasured is stated as such.** macOS has zero measurements; the hardened-Debian and arm64 cases have none on real hardware. Those sections are unchanged and marked, with runbooks retained in the private validation record.

---

## Contents

1. [What changed in v0.5, and why](#1-what-changed-in-v05-and-why)
2. [What earlier revisions cut, and why that still stands](#2-what-earlier-revisions-cut-and-why-that-still-stands)
3. [The whole system in one line](#3-the-whole-system-in-one-line)
4. [Functional core, imperative shell](#4-functional-core-imperative-shell)
5. [Five crates and the engine workers](#5-five-crates-and-the-engine-workers)
6. [Routing without a graph search](#6-routing-without-a-graph-search)
7. [Making it hard to break](#7-making-it-hard-to-break)
8. [The broker — both directions, both paths](#8-the-broker--both-directions-both-paths)
9. [Isolation, the profile, and the floor](#9-isolation-the-profile-and-the-floor)
10. [Limits, budgets, and conflicts](#10-limits-budgets-and-conflicts)
11. [Detection in two phases, and input identity](#11-detection-in-two-phases-and-input-identity)
12. [State on disk](#12-state-on-disk)
13. [Failure is designed](#13-failure-is-designed)
14. [Prediction](#14-prediction)
15. [The seams — how it grows](#15-the-seams--how-it-grows)
16. [Decisions](#16-decisions)
17. [Build sequence](#17-build-sequence)
18. [Rules that keep it simple](#18-rules-that-keep-it-simple)

---

## 1. What changed in v0.5, and why

v0.4 closed the holes an adversarial review found in the *controls*. A second review found that the controls were sound and the **execution model underneath them was missing**: the architecture described how engines are confined without ever saying what an engine *is* as a shipped artifact, and two of its most load-bearing security claims had no type carrying them.

| # | Problem in v0.4 | Fix in v0.5 |
|---|---|---|
| 1 | **No engine artifact existed.** `EngineBin` names *programs*, so every engine must be a subprocess. Nothing said what those programs are, who builds them, where the FFI `unsafe` lives, or how they return `Properties`. Week 0 measured a *linked* stub — the configuration the design forbids. | **Engine workers are first-class artifacts.** `crates/engines/*` are our binaries; `openconvert-worker` is the shared worker-side runtime; the host↔worker protocol is specified. FFI `unsafe` lives in the engine crates, never in the boundary. [§5](#5-five-crates-and-the-engine-workers) |
| 2 | **SR-2 named an enforcement that did not exist.** "`Isolation` on every `Step`" — but `Step` had `Class` and `Limits` and nothing else. The exact hole v0.4 fixed for `Limits` was still open for isolation, on the same default path. | **`Isolation` is a required field of `Step`** (I10), assigned by `route()` from the engine registry's declared minimum. [§7.1](#71-illegal-states-dont-compile) |
| 3 | **`SandboxProfile` had public fields in `core`.** Any crate could construct one claiming AppContainer on Linux. The receipt line the whole product is sold on was the one value nobody had sealed — while `FileFacts` and `Plan` were. | Private fields; construction behind a feature only `openconvert-os` and the WASM executor enable, asserted by a CI gate. Classified honestly as **Hard**, not Impossible. [§7.1](#71-illegal-states-dont-compile), [§7.4](#74-impossible-hard-forbidden) |
| 4 | **The `Reduced` tier was unobtainable in the case it was invented for.** It required a bind-mount namespace, which requires a user namespace — exactly what is missing on a `unprivileged_userns_clone=0` Debian. The machine fell to `Minimal`, below the floor, and refused: the outcome the profile existed to prevent. | **Landlock does not need a user namespace.** The ladder is re-derived from what is actually reachable unprivileged. [§9](#9-isolation-the-profile-and-the-floor) |
| 5 | **SR-1 said "no egress, ever" while the default floor admitted profiles that could not deny egress** (Windows without AppContainer denies network only by an admin-only firewall rule). | The floor is **two independent predicates plus a strength**. `NetworkRequirement::Denied` is **not lowerable by policy** — there is no config key for it. [§9](#9-isolation-the-profile-and-the-floor) |
| 6 | **An archive member could destroy existing user files.** Every traversal control passed a member named `.bashrc` — correctly, because it is not traversal. Undo could not restore it. | Every create is `O_EXCL`/`CREATE_NEW`, on both the job-dir and the copy-out side; `Policy::on_conflict` exists from week 1. [§10.3](#103-conflicts), [A13/SR-15](09-THREAT-MODEL.md#3-attack-classes) |
| 7 | **Nothing tied the bytes routed on to the bytes converted.** Sniff read the path, the engine re-opened it, the receipt hashed it a third time. "Parse, don't validate" was applied to names and not to content, so the receipt could attest to bytes that were never converted. | The file is **opened once and read only through that handle**; untrusted-provenance and volatile-volume inputs are **copied** before routing; `content_id` is computed once. *(v0.5 put the handle inside `FileFacts` in the pure core; v0.6 moved it to `-run` behind an `InputToken` — same guarantee, and the core is testable without a filesystem.)* [§11.3](#113-input-identity), [A14/SR-16](09-THREAT-MODEL.md#3-attack-classes) |
| 8 | **The Windows shell extension ran our parser in `explorer.exe`**, on by default in v1, with no requirement, no test, no gate and no file in the schema — the only place our code runs outside our process, and the one place the threat model never looked. | **Cut from v1** with a named trigger and a design that opens no file at all. [§15](#15-the-seams--how-it-grows), [SR-17](09-THREAT-MODEL.md#5-requirements-to-tests) |
| 9 | **`Environment` was an unnamed eighth noun** carrying the route table, the engine set and the sandbox profile — the value I9 compares the security floor against — with no file and no rule about who may build one. | Named. Eighth noun, `core/src/environment.rs`, with the same written argument `Limits` got. [§3](#3-the-whole-system-in-one-line) |
| 10 | **Failure outside the sandbox lifecycle was undesigned.** Truncated file, zero-byte file, read-only destination, disk full mid-write, receipt unwritable: nothing. | [§13](#13-failure-is-designed) — a table of every failure, its mechanism, its message and its test. |
| 11 | **Persistent state had no home.** History, journal, config, recipes, crash counters and the revocation list are all attacker-influenceable and none had a location, a format or an owner. | [§12](#12-state-on-disk) — one directory, one owner, every file treated as untrusted on read. |
| 12 | **`every_pair_has_route_or_reason` made adding a format quadratic.** Format 21 owed 40 new table decisions. Nobody noticed that the invariant making gaps visible also made extension expensive. | Scoped to same-`MediaKind` pairs plus an explicit cross-kind list. [§6](#6-routing-without-a-graph-search) |
| 13 | **`§13`'s extension table was optimistic** and omitted the most common change of all — adding a field to `Step` — which is the most expensive one. | Replaced with measured counts, including the expensive rows. [§15.1](#151-what-extension-actually-costs) |
| 14 | **The 25-week schedule assumed a headcount it never stated**, and was ~2× optimistic on its two hardest phases. | **44 weeks, two engineers, stated.** [§17](#17-build-sequence) |

Two consequences fall out rather than standing alone. **Prediction moves into the core** — the scoring function is pure, so it belongs there, which makes its p99 gate a pure benchmark and gives the CLI, the GUI and any later surface one implementation instead of three. And **`Properties` becomes an enum per `MediaKind`**, so adding a format inside a kind stops editing a type every kind shares.

---

## 2. What earlier revisions cut, and why that still stands

The governing distinction, unchanged: **YAGNI fights uncertainty-driven complexity; it does not fight clarity-driven design.** Structure that exists because a v1 user story needs it stays. Structure that exists because someone imagined a future goes.

| Was | Now | Why it was speculative |
|---|---|---|
| **12 crates** | **5** | Each split is a stated trigger firing, not a taxonomy — see [§5](#5-five-crates-and-the-engine-workers) |
| **8 IR hubs** | **1** (`AvGraph`) | Six existed for formats v1 doesn't convert; `RasterIR` went too — see below |
| **A\* over a capability graph** | **An ordered route table** | Graph search on ~50 nodes solves a problem that doesn't exist — and the table is *better*, see [§6](#6-routing-without-a-graph-search) |
| **4 isolation tiers** | **2 variants + a profile value** | WASM existed for third-party modules; there are none. microVM is for Paranoid mode; not in v1. |
| **A daemon + JSON-RPC** | **Deleted** | Nothing in v1 needs it. The worker session in [§8.4](#84-the-worker-session-and-how-many-files-one-worker-sees) is *not* this — see the note there. |
| **Content-addressed cache in M1** | **When batches exist — which is week 31** | Named, dated, and now budgeted; see [§15](#15-the-seams--how-it-grows) |
| **6-layer policy resolution** | **3** (defaults → user → flags) | Workspace, profile and org layers had no user |
| **28 prediction features + bandit** | **5 signals, hand-weighted** | You cannot weight 28 features without data, and the bandit needs data to sample |
| **Receipts with VMAF + C2PA** | **Engines, versions, params, class, limits, isolation, profile, hashes** | The rest is post-v1 |
| **A module manifest spec** | **Deleted** | Deferring the implementation but specifying the format is still speculative design |
| **148 pre-planned commits** | **4 weeks detailed, then outcomes** | Commit-level planning beyond a month is fiction |

**`RasterIR` stays cut.** `AvGraph` earns its place outright — it is what makes `mkv→mp4` a stream copy without decoding, which is the flagship demo. `RasterIR` does not: v1's image work is single-hop format conversion where image-rs and libvips each decode and encode directly, and an intermediate linear-light RGBA hub is a lossy, memory-expensive layer that nothing in the build sequence requires. **Named trigger: the first genuine multi-hop raster path** — a conversion where no single engine spans input to output. That trigger is observable in the route table: it fires the first time a `Route` needs two `StepKind`s with an image in between.

**Kept, because a v1 story needs it:** the Plan as an inspectable value (the plan-preview feature *is* this), the fidelity Class on every step (the core promise), the Limits and the Isolation on every step (the safety floor), sandboxing (the product), receipts (the promise), and the route table as data (it generates `openconvert routes`, `openconvert formats`, and the docs site for free).

---

## 3. The whole system in one line

```
bytes ─▶ open ─▶ sniff ─▶ probe ─▶ Facts ─▶ route(Request, Environment) ─▶ Plan ─▶ execute ─▶ Output + Receipt
        └─ shell ─┘  └─sandboxed─┘   └────────── pure core ──────────┘   └────── shell ──────┘
```

**Eight nouns:** `Facts` · `Target` · `Policy` · **`Environment`** · `Plan` · `Step` · `Limits` · `Receipt`
**Two verbs:** `route` (pure) · `execute` (impure)

### On the eighth noun

The rule says an eighth noun is a signal to stop and think, and to write the thinking down. Here it is.

`Environment` was already in the design — it is the second argument of `route()`, it has been since v0.4, and it was simply never counted. That is worse than adding a noun: it is an uncounted one. It carries the route table, the engine registry with declared minimum isolations and versions, and the `SandboxProfile` the machine actually offers. **I9 compares the policy's isolation floor against a value inside it**, which makes it the most security-relevant noun in the list after `Step`, and until v0.5 nobody had asked who was allowed to construct one.

So: named, given a file, and given a rule ([§7.1](#71-illegal-states-dont-compile), I11). The seven-noun count was not wrong about complexity — it was wrong about arithmetic.

### The honest concept count

Eight nouns is what a reader must hold to follow the *data flow*. To read the code they must also hold roughly a dozen supporting types: `Sniff`, `Properties`, `PlanRequest`, `StepKind`, `Class`, `Isolation`, `Warning`, `Route`, `Requirement`, `RouteTable`, `SandboxProfile`, `Strength`, plus five in the boundary crate (`EngineBin`, `Argv`, `BrokeredPath`, `BrokeredOutput`, `OutputName`).

**That is about twenty concepts, not eight, and pretending otherwise is how a "seven nouns" rule turns into a renaming exercise.** The eight are the ones with independent lifecycles that appear in the receipt and the JSON; the rest are supporting vocabulary that exists inside one of them. The rule that actually does work is [rule 8](#18-rules-that-keep-it-simple): no type may be constructible outside the crate that owns its invariant.

---

## 4. Functional core, imperative shell

The single most valuable structural decision, and it costs nothing.

| | Core (pure) | Shell (impure) |
|---|---|---|
| Does | Decides *what will happen* | Makes it happen |
| Contains | Types, the format table, the route table, `route()`, warnings, class / limit / isolation assignment, **prediction scoring** | File I/O, subprocesses, sandboxes, temp dirs, history |
| I/O | **None** | All of it |
| Clock / RNG / env | **None** — time and randomness are inputs, never reads | All of it |
| Tests | Property tests, no fixtures, no files, instant | Golden corpus, integration |
| `unsafe` | `#![forbid(unsafe_code)]` | `forbid` in `-run` and `-sandbox`; permitted in `-os` (syscalls) and `crates/engines/*` (FFI) |
| **Purity enforced by** | **a checked-in `clippy.toml`** — see below | — |

**How purity is enforced, measured rather than assumed.** v0.5 asserted the core's purity and named no mechanism, and the obvious candidate does not work: `std` is fully available on `wasm32-unknown-unknown`, so a core holding `std::fs::File`, `SystemTime::now()` and `std::env::var` **compiles cleanly for that target and produces an rlib** (S1). A wasm32 build is a *portability* gate — worth having, because [04 §5](04-GROWTH.md#5-free-tool-pages)'s tool pages depend on it — but it is not a purity gate and must not be labelled as one.

What does work is a checked-in `clippy.toml`:

```toml
disallowed-types   = ["std::fs::File", "std::path::PathBuf", "std::net::TcpStream", "std::time::SystemTime"]
disallowed-methods = ["std::time::SystemTime::now", "std::env::var", "std::env::vars", "std::fs::read"]
```

It caught all four leak classes in testing while leaving `Vec`, `String` and `HashMap` untouched. A second gate greps for `#[allow(clippy::disallowed` across `-core`, so the bypass is itself gated. Honest class, by [§7.4](#74-impossible-hard-forbidden)'s own taxonomy: **Hard**, not Impossible — `#![no_std]` would be Impossible, and is the upgrade path if the core ever stops needing `alloc`-backed collections at its edges.

Four things fall out for free:

1. **The plan preview is not a feature — it's just not calling `execute`.** The most distinctive thing in the product exists because of the architecture, not in addition to it.
2. **The interesting logic is trivially testable.** "Does `mkv→mp4` choose stream copy when codecs are compatible?" is a pure function call with no files involved. So is "does any default policy ever arm a Class D step?" and, since v0.5, "does any plan ever place a memory-unsafe engine in-process?"
3. **Prediction is a pure function too**, so its `p99 < 10 ms` gate is a criterion benchmark over vectors rather than an integration test over a filesystem — and one implementation serves the CLI, the GUI and any later surface.
4. **The scary code is small and it is two crates.** Everything that brokers a path is in `openconvert-sandbox`, which is `#![forbid(unsafe_code)]`; everything that touches a raw syscall — **including process creation**, see [§5](#5-five-crates-and-the-engine-workers) — is in `openconvert-os`.

**How the pure core learns about an impure world.** `route()` takes an `Environment`. The shell probes once and builds it; the core consumes it as a value. That is how the plan preview can say *"AppContainer unavailable on this machine — running with a restricted token and a Job Object, and network confinement is therefore unavailable, so this conversion is blocked"* without the core reading anything.

---

## 5. Five crates and the engine workers

```
crates/
├── openconvert-core/     the pure core — types + tables + route() + predict. No I/O. No unsafe.
├── openconvert-sandbox/  THE BOUNDARY — broker, EngineBin, Argv, WorkerSession, sweep. No unsafe.
├── openconvert-os/       confinement syscalls + spawn() + available(). Unsafe lives here.
├── openconvert-run/      the shell — detect, engine adapters, execute, receipts, state
├── openconvert/          the CLI binary
└── engines/            OUR WORKER BINARIES — one per engine family. FFI unsafe lives here.
    ├── openconvert-worker/       shared worker-side runtime: protocol, openat discipline
    ├── oc-images/              libvips · libheif · libavif · libjxl · libraw · lcms2
    ├── oc-pdf/                 pdfium · qpdf
    ├── oc-archive/             libarchive
    └── oc-audio/               LAME · libopus · libFLAC   (Symphonia decode stays in-process)
```

Dependency order is a straight line: `core ← sandbox ← os ← run ← cli`. Core depends on nothing. The engine workers depend on `openconvert-worker` and `openconvert-core` (for `Limits`, `OutputName`, the protocol types) and on **nothing else** — in particular they never depend on `-run`, because they are the other side of a trust boundary.

Later, when each earns it: `openconvert-server` and `apps/desktop`.

### Why five, and why the workers are separate binaries

v0.3 said three, splitting on pain. v0.4 added a fourth because a security boundary needs a crate boundary. v0.5 adds a fifth for a reason the earlier revisions had backwards.

**The boundary crate cannot also be the `unsafe` crate.** v0.4 said `openconvert-sandbox` is "the only crate permitted `unsafe`, confined to OS syscalls and FFI" *and* that it is the small, auditable thing a third party reviews. Those are different jobs, and separating them buys two things: a **pointed audit** — 800 lines of invariant logic rather than that plus a thousand lines of `libc` calls — and a **clear blast radius**, because a bug in the syscall crate is then classifiable as a confinement failure rather than an unknown.

> **What it does not buy, contrary to v0.5.** That revision justified the split by claiming a syscall bug in a crate "sharing its address space" could defeat the invariants. Both crates link into the same process either way; the split changes nothing about address space. The two reasons above are sufficient, and the record should not claim a third that is not true.

So: **`openconvert-sandbox` is `#![forbid(unsafe_code)]`.** It is the invariant crate — `OutputName::parse`, `BrokeredPath`, `BrokeredOutput`, `EngineBin`, `Argv`, `WorkerSession`, `sweep` — expected to stay under ~800 lines. **`openconvert-os`** holds the raw syscalls: `landlock_*`, `seccomp`, `clone3`, `CreateAppContainerProfile`, `sandbox_init`, `SetInformationJobObject`, `available()` — **and `spawn()`.**

**Process creation moved to `-os` in v0.6, for a measured reason.** v0.5 put `Worker` and the spawn in `-sandbox` and described it as "pure logic over `std::fs` and `std::process`". It cannot be. Handing a worker pre-opened descriptors and nothing else requires `unsafe` on **both** platforms, by two independent routes:

- **Windows:** `std::process::Command` sets `bInheritHandles = TRUE` and the child then inherits *every* inheritable handle in the parent. A test child enumerated **64 handles it was never given and wrote through two of them** (S3). The only fix is `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, which `Command` cannot express — it needs raw `CreateProcessW`.
- **Linux:** Rust opens everything `CLOEXEC`, so a pre-opened fd is closed by `exec` unless a `pre_exec` hook clears the flag. `pre_exec` is `unsafe`.

`-sandbox` therefore calls one typed entry point and holds no syscalls:

```rust
// openconvert-os
pub fn spawn(bin: &EngineBin, argv: &Argv, handles: &HandleSet, conf: &Confinement)
    -> Result<Worker, SpawnError>;
```

The audit ([09 §9](09-THREAT-MODEL.md#9-security-engineering-practices)) covers both crates and knows which is which.

**Engine workers are separate binaries because `EngineBin` says so.** A closed enum of executable programs is only meaningful if the programs exist. libvips, libheif, libavif, libjxl, libraw and lcms2 are *libraries*; pdfium has no usable CLI at all; and none of them emit the structured `Properties` that [two-phase detection](#11-detection-in-two-phases-and-input-identity) requires. Depending on stock upstream CLIs would mean shipping ten third-party executables we do not control, parsing their human-readable output, and having no probe protocol. So we write four small binaries instead, grouped by engine family so that the count of things to sign, notarize and confine stays at four rather than ten.

Each worker is: a `main` that installs its own confinement (`openconvert-os`), reads length-prefixed [protocol](#52-the-hostworker-protocol) messages from fd 3, calls its C libraries through FFI, and writes results to fd 4. **FFI `unsafe` lives here, in a process that is already confined, on the far side of the boundary from every invariant.** That is the correct place for it, and it is why this split is not crate inflation: it is the difference between `unsafe` inside the trust boundary and `unsafe` outside it.

**A fifth trigger joins the split list:** *`unsafe` may not share a crate with a compile-time security invariant.* Like the fourth, this is clarity-driven — the structure exists because a claim already made requires it.

### 5.1 What each crate is

```
openconvert-core/               the brain — eleven files
├── lib.rs
├── format.rs      FormatId, MediaKind + the format table (data, incl. magic signatures)
├── facts.rs       FileFacts, Sniff, Properties (enum per MediaKind), InputToken
├── target.rs      Target — what the user asked for
├── policy.rs      Policy — defaults → user → flags; floor, max_auto_class, on_conflict, worker_reuse
├── limits.rs      Limits (per step) + Budget (per job) + LimitCeiling
├── isolation.rs   Isolation, SandboxProfile (sealed), Strength, IsolationFloor
├── environment.rs Environment — the machine, the engines, the tables
├── plan.rs        PlanRequest, Plan, Step, Class, Warning, version
├── route.rs       Route, Requirement, RouteTable, route() — the interesting part
├── wire.rs        the ONLY module with Deserialize — see §5.2
└── predict.rs     the five signals, scored — pure, no history I/O

openconvert-sandbox/            THE BOUNDARY — #![forbid(unsafe_code)]
├── lib.rs
├── broker.rs      BrokeredPath · BrokeredOutput · OutputName · DestinationName
├── engine_bin.rs  EngineBin — the closed set of programs we may execute
├── argv.rs        Argv — no command strings exist
├── session.rs     WorkerSession — owns a job dir; grants and revokes per job (§8.4)
├── protocol.rs    the host↔worker wire format (§5.2)
└── sweep.rs       startup reaping of orphaned job directories

openconvert-os/                 the syscalls — the only host-side unsafe
├── available.rs   probe the machine once -> SandboxProfile   (constructs the sealed type)
├── spawn.rs       spawn() — handle lists, pre_exec, confinement application
├── readback.rs    query what ACTUALLY engaged, in the child (§9.5)
└── linux.rs · macos.rs · windows.rs

openconvert-run/                the shell — #![forbid(unsafe_code)]
├── lib.rs         execute(plan, ctx, cancel) -> Outcome
├── handles.rs     HandleTable — InputToken -> File. The core never holds one (§11.3)
├── receipt.rs     Receipt from an execution log
├── detect/        sniff · polyglot · probe · provenance
├── engines/       adapters: argv builders + protocol clients (image_rs is in-process)
├── ir/avgraph.rs  codec-preserving stream model
└── state/         history · journal · config · quarantine · paths   (§12)

openconvert/                    the CLI binary
└── src/main.rs · cmd/* · output.rs

engines/*                     our worker binaries — FFI unsafe permitted here
```

Eleven core files contain the entire decision-making of the application. Only `route.rs` is interesting.

### 5.2 The host↔worker protocol

Private to one job, one client, and unversioned as a public contract — but it is a real format, so it is specified rather than assumed.

```
host                                            worker (confined)
  spawn EngineBin::Images with Argv
    fd 3 = requests (r)   fd 4 = responses (w)
    fd 5 = job-dir fd     fd 6.. = pre-opened read-only input fds
  ── Probe { sniff }              ──▶
  ◀── Properties(..) | Failed{..} ───
  ── Run { step, inputs, outputs }──▶
  ◀── Progress { done, total }    ───   (repeated, optional)
  ◀── Wrote(OutputName)           ───   (repeated)
  ◀── Done | Failed { .. }        ───
  finish(): copy out, remove job dir
```

Length-prefixed `postcard`, one `protocol_version` field, both directions fuzzed. Round-trip cost is **348 µs p50 / 614 µs p99** at 4 KB (S6), which is what makes two-phase detection affordable — [08 §1](08-EXECUTION-PLAN.md#1-week-0--before-any-code)'s sub-millisecond criterion is met with room to spare.

Six properties matter. The first three were in v0.5; the last three are v0.6, and each closes something a spike found.

- **The worker never receives a path.** Inputs arrive as pre-opened read-only file descriptors — not opaque names, *no names at all*. That closes the filename-as-attack-vector class ([CVE-2025-55298](09-THREAT-MODEL.md#2-evidence-base)) more completely than renaming did, and it is what lets the Linux ladder work without a mount namespace.
- **The worker creates outputs only as `openat(fd 5, name, O_CREAT|O_EXCL|O_NOFOLLOW)`**, where `name` came from the host as an `OutputName`.
- **`Failed` is structured**, so [§13](#13-failure-is-designed) can turn an engine failure into a message naming the file, the step and the engine.
- **The worker holds no descriptor it was not handed — and that requires an explicit inherit list.** See [§5.3](#53-what-the-implementer-must-not-discover-later).
- **Everything crossing the wire arrives as a `wire::` type, never a domain type.** `Deserialize` is implemented in exactly one module, `core/src/wire.rs`, and a lint asserts it appears nowhere else in `-core` or `-sandbox`. Each `wire::` value converts to its domain type through a fallible clamping constructor. `OutputName` and `DestinationName` cross as `String` and are **re-parsed**; they never `#[derive(Deserialize)]`, because deriving it would reconstruct a validated type without running its validator.
- **Engine-supplied facts may narrow a limit and never widen one.** `Policy` carries a `LimitCeiling`; the conversion clamps against it. A compromised engine reporting `width = 4_000_000_000` to inflate a pixel budget is refused at the boundary rather than believed (S6). Asserted by `wire::engine_facts_cannot_widen_limits` from w2.

**Frame bounds are host-side and checked before allocation.** `max_frame_bytes` and `max_messages_per_step` are fields of the session, not of the decoder. This is deliberate: an attacker-chosen length prefix is a *correct* decode as far as any parser is concerned, so no amount of fuzzing will find it — 7.4M fuzz cases over the parser did not (S29). Only a bound catches it.

### 5.3 What the implementer must not discover later

Three things cost a week each if they are found during implementation rather than read here first. All three were found by spikes, and two of them produce errors that look like something else entirely.

**1 · The job directory must be ACL'd for the container SID, and the handle alone is not enough.**
Handing a worker a pre-opened job-directory handle is *necessary and not sufficient* on Windows. Inside an AppContainer, `NtCreateFile` performs a **fresh access check for the new file** against the child's token, and returns `STATUS_ACCESS_DENIED` (`0xC0000022`) even through a valid handle (S9). Every job directory is ACL'd for the AppContainer SID before the worker starts. The startup sweep ([§12](#12-state-on-disk)) must expect to find directories carrying those ACLs.

**2 · The same applies to every shared library the worker loads.**
Since engines link dynamically ([02 §11](02-FEATURES.md#11-model-licensing-policy)), the worker's `.dll`/`.so`/`.dylib` files are opened by the loader *inside* the container, under the container's identity — not the installer's. **A worker whose libraries are not readable by the container SID fails before `main` runs**, with a loader error that names a missing dependency rather than a permission problem. Installers grant read+execute to the AppContainer SID on the engine library directory; packaging tests assert it.

**3 · All host-side handles are opened non-inheritable by default.**
Inheritance is granted per spawn through `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, never by leaving `bInheritHandles = TRUE` and hoping. The default is the safe one because the failure mode of the other default is silent: the child works fine and simply holds 64 more handles than intended.

---

## 6. Routing without a graph search

A\* over a capability graph is the wrong tool here, and replacing it with an ordered table makes the product *better*, not merely simpler.

### The route table

```rust
pub struct Route {
    steps:    &'static [StepKind],
    class:    Class,
    requires: &'static [Requirement],
}

impl RouteTable {
    pub fn routes_for(&self, from: FormatId, to: FormatId) -> &[Route];
}

pub fn route(req: &PlanRequest, env: &Environment) -> Plan;
```

`MKV → MP4` has exactly two entries, in order:

| # | Route | Requires | Class |
|---|---|---|---|
| 1 | `StreamCopy` | codecs are MP4-compatible | **A — lossless** |
| 2 | `Decode → Encode` | *(always applies)* | B — lossy |

`route()` returns the first route whose requirements hold, attaches the `Class`, the `Limits` and the `Isolation` the policy and environment imply, and records the requirement that eliminated each rejected route — which is where the explanation comes from. Thirty lines instead of a graph search.

`RouteTable` is a value living in `Environment`, so a module can append routes when that day comes without changing a signature or a caller. The trigger stays deferred; the type stops foreclosing it.

### Why the table is better

| | Route table | A\* search |
|---|---|---|
| **Inspectable** | `openconvert routes mkv mp4` prints it. Ships as docs and as the website's `/formats` section. | You cannot print a search |
| **Explainable** | *"Stream copy, because the codecs are compatible. The alternative was re-encode."* Falls out of the requirement that failed. | Requires reconstructing a heuristic |
| **Debuggable** | A wrong choice is a wrong requirement — one line | A wrong choice is a wrong weight, somewhere |
| **Tunable** | Nothing to tune | Weights need calibration; calibration needs data you don't have |
| **Testable** | Assert on a table | Assert on emergent behaviour |
| **One artefact, four consumers** | The docs, the website, `openconvert routes`, and the property test all read the same table, so a wrong route is a one-line diff visible in four places | Four separate explanations of one heuristic |

That last row is the strongest argument and v0.4 did not make it.

### Coverage without quadratic cost

v0.4 asserted that **every** format pair has a route or an explicit `NoRoute` reason. That makes gaps visible — and makes adding format 21 owe 40 new table decisions, growing with the square of the catalogue. Nobody noticed.

**The property is now scoped:** every pair *within a `MediaKind`* has a route or an explicit reason, plus an explicit list of cross-kind pairs a user could plausibly attempt (`image→pdf`, `video→audio`, `pdf→image`, `doc→pdf`, …). Everything else falls to a generated `NoRoute::CrossKind` reason that is still printed, still explains itself, and costs nothing to add a format to. Coverage of the cases a human would try is unchanged; the cost of a new format drops from O(n) to O(1).

### When A\* earns its place

When routes need genuine multi-hop composition the table can't express — a chain like `X → IR → IR' → Y` across many formats. At 20 formats there is almost no such path, and a search of the v1 format set found none. **The rule: add search when the table can't express something, not before.**

---

## 7. Making it hard to break

"Hard to break" is a **type-system** property, not a discipline property — but only where the type system can actually reach. [§7.4](#74-impossible-hard-forbidden) says plainly which of these are which.

### 7.1 Illegal states don't compile

| # | Invariant | How it is enforced | Class |
|---|---|---|---|
| I1 | Nothing can invent facts about a file | `FileFacts` has private fields; only `detect()` constructs it | **Impossible** |
| I2 | Nobody can hand-build a plan | `Plan` has private fields and no `Deserialize`. Only `route()` produces one. | **Impossible** |
| I3 | A step cannot exist without a fidelity class | `Class` is a non-optional field of `Step` | **Impossible** |
| I4 | A step cannot exist without resource limits | `Limits` is a non-optional field of `Step`; `execute()` takes them from the step, never from a default | **Impossible** |
| **I10** | **A step cannot exist without an isolation** | **`Isolation` is a non-optional field of `Step`.** `route()` sets it from the engine registry's declared minimum in `Environment`; there is no default and no `Option`. | **Impossible** |
| I5 | An engine cannot receive a user path | Engines take `BrokeredPath`, or — since v0.5 — a pre-opened fd and no name at all. The constructor is `pub(crate)` in **another crate**. | **Impossible** |
| I6 | An engine cannot write outside its job directory | Writes take `BrokeredOutput`, obtainable only by joining `OutputName`s under the job root, created `O_EXCL|O_NOFOLLOW` | **Impossible** (to represent) |
| I7 | Shell injection is unrepresentable | `Argv` names its program with `EngineBin`, a **closed enum**. No `Other(String)`, no `From<&str>`, and `sh` is not a variant. | **Impossible** |
| I8 | A generative step cannot be armed automatically | `route()` filters routes above `Policy::max_auto_class` (default `B`). Class D is reachable only from a `Target` naming the operation. | **Impossible** |
| I9 | A conversion cannot run below the policy's isolation floor | `route()` compares the profile in `Environment` against `Policy::floor` and emits a blocking `Warning` instead of steps | **Impossible**, given I11 |
| **I11** | **Only the OS prober can assert what confinement engaged** | `SandboxProfile` has private fields; its constructor is behind `feature = "attest"`, enabled by `openconvert-os` and the WASM executor only, asserted by a CI gate over every `Cargo.toml`. **Since v0.6 the values come from read-back, not from the request** ([§9.5](#95-the-profile-is-read-back-not-requested)) | **Hard** — see [§7.4](#74-impossible-hard-forbidden) |
| **I12** | **A conversion cannot write over a file that already exists** | Every create on both sides is `O_EXCL`/`CREATE_NEW`; `Policy::on_conflict` decides suffix-or-fail *before* the write, never `O_TRUNC`. Enforced by the **destructive-API lint** — eleven APIs, not one ([§7.4](#74-impossible-hard-forbidden)) | **Hard.** v0.5 called this Impossible on the strength of banning one API; S4/S25 found seven live routes past that ban |
| **I13** | **The bytes routed on are the bytes converted** | `FileFacts` holds an `InputToken`; `-run`'s `HandleTable` holds the one open handle and every read goes through it; untrusted-provenance inputs are copied first | **Hard** — see [§7.4](#74-impossible-hard-forbidden) |
| **I14** | **A wire value cannot become a domain value without being clamped** | `Deserialize` exists only in `core/src/wire.rs`, asserted by a lint over `-core` and `-sandbox`; every `wire::` → domain conversion is fallible and clamps against `LimitCeiling` ([§5.2](#52-the-hostworker-protocol)) | **Hard** — the lint is what holds it |

I5, I6, I7, I10 and I12 matter most. The [ImageMagick command-injection and filename-format-string CVEs](09-THREAT-MODEL.md#2-evidence-base) and the [Zip Slip class](09-THREAT-MODEL.md#3-attack-classes) are entire bug *families* these make impossible to write.

> **The security model is enforced by the compiler across a crate boundary — except where it is not, and [§7.4](#74-impossible-hard-forbidden) says exactly where.**

### 7.2 Parse, don't validate — names *and* content

Detection *parses* bytes into `FileFacts` once, at the boundary. Archive entry names *parse* into `OutputName` once, at the boundary. Destination names *parse* into `DestinationName` once, at the boundary.

v0.4 applied this to names and not to content: `FileFacts` described a path, and every later stage re-opened it. Since v0.5 the file is **opened once and read only through that handle** ([§11.3](#113-input-identity)), so downstream code receives a value whose validity is established *and whose subject cannot change underneath it*.

**Since v0.6 the handle lives in `-run`, not in the core.** v0.5 put an `InputHandle` field inside `FileFacts`, which is a live OS resource in a crate this document declares has no I/O and tests with "no fixtures, no files". `FileFacts` now carries an opaque `InputToken`; `openconvert-run` owns the `HandleTable` that maps it to the open file. The guarantee is unchanged — it comes from the pairing of token and table, not from where the field sits — and the core stays testable without a filesystem, which is what [§4](#4-functional-core-imperative-shell) promised.

### 7.3 The tests that hold the line

Every one is a CI gate that fails the build; see [09 §5](09-THREAT-MODEL.md#5-requirements-to-tests) for the requirement→test map and [08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates) for the week each goes live.

| Test | Catches |
|---|---|
| Class A round-trips byte-identically | The lossless claim degrading unnoticed |
| Class B byte-identical under `--deterministic`; SSIM floor otherwise | Quality regressions, and the reproducibility claim rotting |
| No default policy arms a Class D step (property) | The generative rule rotting into a comment |
| Every step carries limits (property) | The v0.3 hole reopening |
| **Every step carries an isolation; no memory-unsafe engine is `InProcess`** (property) | **The v0.4 hole reopening** |
| **No profile without network denial ever yields steps** (property) | SR-1's "ever" becoming an aspiration |
| **No destructive filesystem API is reachable** (lint over 11 APIs + corpus) | Silent destruction of user data — see [§7.4](#74-impossible-hard-forbidden) for why one API was not enough |
| **The core is pure** (`clippy.toml` + `#[allow]` grep) | The functional-core claim rotting into a convention |
| **A `wire::` value cannot become a domain value unclamped** (lint on `Deserialize`) | An engine widening its own limits |
| **The read-back profile never claims an unengaged mitigation** | The receipt attesting to intentions instead of facts |
| **Mutation score on `-core`** (`cargo-mutants`) | **The tests themselves being vacuous** — see below |
| Fuzz: sniffer, route table, entry names, destination names, receipts, recipes, **and every engine adapter** | Every parser of untrusted or semi-trusted bytes |

> **Why mutation testing is on that list.** Run against a parser that already passed 24 evil-corpus cases and 4,000 property cases, `cargo-mutants` found **10 of 30 mutants survived** — including a corpus test that **passes vacuously when its own input is emptied**, and a branch that was unreachable. In the same period `cargo-fuzz` ran **7.4 million cases** against that parser and found nothing (S29, S30).
>
> The two tools answer different questions. **Fuzzing tests the code; mutation testing tests the tests.** A suite this document leans on as heavily as it does needs the second.
| Sandbox escape attempts, per profile tier | Network, filesystem, process and path-traversal escapes |
| Sandbox overhead thresholds | The one architectural risk, asserted rather than benched |

### 7.4 Impossible, Hard, Forbidden

A claim is only worth what its weakest enforcement is. Ours:

| Claim | Enforcement | Honest class |
|---|---|---|
| No shell can be spawned | Closed enum in another crate + source scan for `Command::new` | **Impossible.** Widening the enum is a cross-crate diff, and the scan catches the alternative |
| No traversal out of the job dir | `OutputName::parse` + `O_NOFOLLOW` + post-canonicalisation assert, run twice | **Impossible to represent**; the residual is a bug in `parse`, which is fuzzed |
| Nothing overwrites an existing file | The **destructive-API lint** over `-sandbox` and `-run`, enumerating eleven APIs (below), plus the `O_EXCL` corpus test | **Hard.** v0.5 claimed **Impossible** on the strength of banning `File::create`; measurement found seven other live routes |
| Every step has class, limits, isolation | Non-optional fields | **Impossible** |
| The recorded profile is the one that engaged | Private fields + a feature only `-os` enables + a CI gate over every `Cargo.toml` + **read-back in the child** ([§9.5](#95-the-profile-is-read-back-not-requested)) | **Hard.** A contributor can enable the feature in one line; it is a visible manifest diff, blockable by CODEOWNERS and failed by the gate. Rust cannot express "only this crate may call this" across a crate boundary, so we do not claim it does |
| The bytes routed on are the bytes converted | One open handle in `-run`'s `HandleTable` + copy-on-untrusted-provenance | **Hard.** An open handle defeats replace-by-rename, which is the realistic attack; it does not defeat in-place modification of the same inode by a process that already has write access. Stated in [09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against) |
| No third-party parser runs in-process | I10 + a property test over the registry | **Impossible**, given the registry is honest — which is a data claim, gated by a review rule on one table |
| The core is pure | `clippy.toml` `disallowed-types`/`disallowed-methods` + a grep for `#[allow(clippy::disallowed` ([§4](#4-functional-core-imperative-shell)) | **Hard.** `#![no_std]` would be Impossible; a `wasm32` build enforces nothing |

**Six of fourteen invariants are Hard rather than Impossible.** v0.5 said three of thirteen. **The number went up because two claims were tested and found weaker than stated, not because the design got worse** — and a design whose honest count rises after measurement is working exactly as intended.

#### The destructive-API list

`File::create` was the whole of v0.5's ban. S4 and S25 enumerated what a file-destroying contributor could reach without touching it: **five routes on Windows, seven on Linux.** The lint now names all of them.

```
File::create · OpenOptions{create,truncate} · fs::write · fs::copy · fs::rename
fs::remove_file · fs::remove_dir_all · File::set_len · std::os::unix::fs::symlink
MoveFileExW(MOVEFILE_REPLACE_EXISTING) · SetFileInformationByHandle(FileRenameInfo)
```

Two deserve naming, because they are the ones a scan for "truncating open" was never going to find:

- **`set_len(0)`** truncates through a handle opened with plain `write(true)`. There is no truncating *open* anywhere in the call path.
- **`fs::rename`** silently replaced its target on both filesystems tested. Write-to-temp-then-rename is *the* idiomatic atomic write, and [§13](#13-failure-is-designed)'s "partial output removed" rows push an implementer straight toward it. This is the likeliest way I12 gets broken by someone acting in good faith.

`File::create_new` is safe on both platforms and is the only creation API permitted.

> macOS adds `copyfile(3)` and `clonefile(2)`, which have no equivalent elsewhere. They are listed in the private macOS validation runbook T4 and go into the lint when a Mac has confirmed their behaviour. **Until then this list is Linux- and Windows-complete only**, which is stated rather than assumed.

### 7.5 Errors

`thiserror` with exhaustive typed errors in `core`, `sandbox` and `os`; `anyhow` with context in `run` and the binaries. The boundary is the crate boundary — no exceptions. Every user-facing error names **which file, which step, which engine**, and what to do about it; [§13](#13-failure-is-designed) is the table of what that means for each failure.

---

## 8. The broker — both directions, both paths

**Capability brokering is the day-one contract, not a hardening pass.** v0.3 got the inbound half right and had no outbound half. v0.4 added the outbound half but applied both only to *sandboxed* steps, leaving the in-process default path — the highest-volume formats — outside the broker entirely.

### 8.1 Inbound

A job directory is created for **every** step, in-process or sandboxed. Inputs are made available to the engine as:

- **Sandboxed:** a pre-opened, read-only file descriptor. No path, no name. The engine cannot open anything else because the OS will not let it — **and because it was given an explicit inherit list**, without which Windows hands it every inheritable handle in the parent ([§5.3](#53-what-the-implementer-must-not-discover-later)).
- **InProcess:** the handle `-run` holds for this `InputToken` ([§11.3](#113-input-identity)). No re-open, no second `stat`.

Where a copy is required (untrusted provenance, network or removable volume, or an engine that insists on a path), it is copied into the job directory under an opaque `[a-f0-9]{16}.bin` name — which never starts with `-`, so it cannot be reinterpreted as a flag by an engine's own argument parser.

### 8.2 Outbound

An engine's output is not a path. It is a **name**, parsed:

```rust
pub struct OutputName(String);          // exactly one path component
pub struct DestinationName(String);     // same rules; applied to the user-facing copy-out

impl OutputName      { pub fn parse(s: &str) -> Result<Self, NameError>; }
impl BrokeredOutput  {
    pub fn child(&self, name: &OutputName) -> Result<BrokeredOutput, IoError>;
    pub fn create_new(&self, name: &OutputName) -> Result<File, IoError>;  // O_EXCL. There is no create().
}
```

`OutputName::parse` rejects, by construction:

| Rejected | Because |
|---|---|
| `""`, `"."`, `".."` | traversal |
| anything containing `/` or `\` | traversal; a member path is a *sequence* of names, parsed one at a time |
| `C:`, `\\?\`, `\\server\share`, leading `/` | absolute and drive-relative forms |
| `:` anywhere | NTFS alternate data streams (`file.txt:evil.exe`) |
| NUL and control characters | truncation tricks across the FFI boundary |
| `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, **`CONIN$`, `CONOUT$`**, with or without an extension | Windows reserved device names — see the note below on *why* |
| trailing `.` or space | Windows silently strips these, so `evil.exe.` and `evil.exe` collide |
| > 255 bytes | filesystem name length |

**Normalisation is a collision rule, not a rejection rule — corrected in v0.6.** v0.5 rejected "a component that normalises to a different name under NFC/NFD". Tested against ten ordinary filenames, **that rule rejects five of them** — `café.jpg`, `Müller-Bericht.pdf`, and the Greek, Cyrillic and French cases (S24). Every archive produced on macOS, which stores NFD, would fail extraction with a security error.

So: **names are preserved byte-for-byte, and NFC is used only as a comparison key.** Within one extraction, two entries whose NFC forms are equal is a *collision*, resolved by `Policy::on_conflict` like any other.

The premise underneath the old rule was also wrong. **NTFS does not fold NFC and NFD** — both files exist side by side (S13). APFS folds; ext4 does not. That is three filesystems and three different outcomes for one archive, which is a cross-platform determinism problem for a product whose second pillar is telling you exactly what happened; it is recorded in [09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against).

**Reserved names: right rule, wrong reason.** Of ten reserved names tested, **only `NUL` vanished into a device**; `CON`, `PRN`, `AUX`, `COM1`, `LPT1`, `CONIN$` and `CONOUT$` all became ordinary files, because Rust's `std::fs` uses verbatim `\\?\` paths that bypass Win32 device parsing (S13). The predicate stays — it protects **the C engines, which open ordinary Win32 paths, and whatever the user opens the output with** — but it protects them, not us, and the record should say so. The failure it prevents is in [§13](#13-failure-is-designed): a write that succeeds, raises no error, and discards the data.

**Path length is bounded at the job root.** `OutputName` bounds a *name*; nothing bounded the *path*. An 829-character path with a 255-character leaf was created without complaint (S28) — fine for our Rust, but `MAX_PATH` binds the C engines. The job-directory path is bounded at creation so that job root + deepest legal member stays inside what an engine can open, and worker manifests set `longPathAware`.

`child()` creates each level itself with `O_NOFOLLOW` (POSIX) / `FILE_FLAG_OPEN_REPARSE_POINT` (Windows), refuses to traverse an existing symlink or junction, and asserts after canonicalisation that the result is still under the job root. Archive entries that are symlinks, hardlinks, devices, or FIFOs are **refused by default**; a policy key can permit within-tree symlinks.

**`create_new` is the only creation API, and it is `O_EXCL`.** There is no truncating open anywhere in `openconvert-sandbox` or `openconvert-run`; a source scan gates it (I12). This is what stops an archive member named `.bashrc` from destroying a file that traversal checks correctly let through.

**Every check runs twice.** Once inside the sandbox as the engine extracts, and again in the copy-out, which is trusted code — and the copy-out additionally parses each name as a `DestinationName` and applies [§10.3](#103-conflicts).

### 8.3 The copy-out, and the conflict policy

The copy-out is the one place trusted code handles untrusted names, so it does three things in order: parse each name as a `DestinationName`; resolve conflicts per `Policy::on_conflict` **before** opening anything; create with `CREATE_NEW`. A collision under `OnConflict::Fail` aborts the step with a message naming the colliding file, and nothing is written — not even the outputs that would have succeeded, because a half-extracted archive is worse than a refused one.

### 8.4 The worker session, and how many files one worker sees

A `WorkerSession` owns **one job directory at a time** and carries a `CancelToken`. Whether the underlying process is reused for the next job is [`Policy::worker_reuse`](#84-the-worker-session-and-how-many-files-one-worker-sees) — a user-facing setting, for reasons the measurements made unavoidable.

```
WorkerSession::open(isolation, limits, reuse)
   ├─ grant(job)       -> capabilities handed to the process   // DuplicateHandle
   ├─ probe(sniff)     -> Properties         // §11, inside the sandbox
   │     … route() runs on the host, pure …
   ├─ run(step)        -> StepOutcome
   ├─ revoke(job)      -> capabilities withdrawn               // DUPLICATE_CLOSE_SOURCE
   └─ finish()         -> outputs copied out, job dir removed
```

#### Why this is a setting and not a constant

v0.5's D11 was *"one process per file, for the duration of that file"* — the strongest isolation available, and unaffordable on one platform:

| | Windows | Linux |
|---|---|---|
| Fresh worker per file | **40.15 ms** — 161% of the 25 ms budget | 1.96 ms — 7.9% |
| Reused worker | **2.19 ms** — 8.8% | — *(already inside budget)* |

The sandbox is **4.43 ms** of that forty; the rest is Windows process creation, which is slow for everyone — a Microsoft-signed system binary costs 15.0 ms to create against our 15.8 (S7, S8, S16). It is not a signing, packaging or binary-size problem, and none of those levers move it.

**Reuse is safe in the way that matters and unsafe in one way that does not always matter.** A capability can be granted to a running confined worker and **revoked from it** — `DuplicateHandle` in, `DUPLICATE_CLOSE_SOURCE` out, the worker getting `STATUS_INVALID_HANDLE` if it tries afterwards — and this works **inside an AppContainer** (S9 × S16). What cannot be revoked is memory. A worker compromised by file A can retain A's bytes and write them into the output of file B in the same batch.

Whether that matters is a property of the user's threat model rather than of ours, so it is theirs to set:

```rust
pub enum WorkerReuse {
    Balanced,   // default: reuse across files of trusted provenance;
                //          an untrusted-provenance file always gets its own worker
    Isolated,   // one worker per file, always. v0.5's D11.
}
```

- **`Balanced`** groups by the same `Provenance` that already raises the isolation floor ([§9.2](#92-the-ladder-re-derived-from-what-is-actually-reachable)) — so this reuses a signal the design already computes rather than introducing one. A leak between two files the user already trusts is a leak from the user to themselves; a downloaded file never shares a worker with anything.
- **`Isolated`** is the strongest setting, and the UI states its cost rather than hiding it: *"about 38 ms extra per file on Windows."*
- Like `IsolationFloor`, **a policy layer may raise `worker_reuse` to `Isolated` and may never lower it.** Managed deployments pin it; individuals choose. Both halves of that — the one-way ratchet and "untrusted never shares" — are property tests from w2 under [SR-20](09-THREAT-MODEL.md#5-requirements-to-tests).
- The receipt records which files shared a worker and which did not. That is not an apology for the tradeoff — it is the product's second pillar applied to its own internals.

**This is not the daemon that v0.3 deleted.** The daemon was a long-lived background service with a discoverable socket, a versioned public protocol and multiple independent clients. A session has none of those: it lives for one batch at most, its pipe is private, it has exactly one client, and it exits when the batch is done.

What the session buys:

- **Cancellation has a unit.** `CancelToken` → close the pipe → `SIGKILL` / `TerminateJobObject`. That is what the `Panic` hotkey does, for every session at once, bounded at 500 ms and asserted in CI.
- **Cleanup has an owner.** The job dir is created by the session and removed by it. Each dir holds a lock file; a **startup sweep** removes any dir whose lock is unheld. The sweep only ever touches directories under our own per-user state directory ([§12](#12-state-on-disk)) — never a shared temp root, because a sweep over a world-writable directory is a deletion primitive.
- **Crashes are contained and reported honestly.** An engine that dies mid-conversion produces `StepOutcome::Crashed { engine, version, signal }`, the receipt records it, and no partial output is copied out. **Under `Balanced` a crash ends the batch**, not just the file — the remaining files are re-dispatched to a fresh worker, and the receipt says so. Three crashes on one format auto-disable that path locally with a visible reason.
- **A limit that kills the worker is still reported.** A Job Object cap terminates the process without a `Failed{..}` message; the completion port is what recovers the reason. See [§10.1](#101-per-step).

---

## 9. Isolation, the profile, and the floor

```rust
pub enum Isolation {
    InProcess,                  // pure-Rust parsers only — enforced by I10 + property test
    Sandboxed(SandboxProfile),  // OS-confined subprocess — the profile says how
}

pub struct SandboxProfile {     // PRIVATE fields; see I11
    filesystem: FsConfinement,  // Landlock | AppContainer | AppSandbox | BindMount | BrowserOrigin | None
    network:    NetConfinement, // EmptyNetns | SeccompBlock | CapabilitySid | NoEntitlement | Csp | None
    syscalls:   SyscallFilter,  // SeccompBpf | None
    resources:  ResourceEnforcement, // CgroupV2 | JobObject | Rlimit | None
    privileges: PrivDrop,       // UserNs | RestrictedToken | None
}

impl SandboxProfile {
    pub fn strength(&self)            -> Strength;  // Full | Reduced | Minimal | None
    pub fn denies_network(&self)      -> bool;
    pub fn confines_filesystem(&self) -> bool;
    #[cfg(feature = "attest")]
    pub fn attest(..) -> Self;        // openconvert-os and the WASM executor only (I11)
}
```

### 9.1 The floor is three predicates, not one number

v0.4 had a single `min_strength`, and it could not express the thing SR-1 actually requires. A machine can offer meaningful filesystem confinement and no way to deny network; a single ordinal has to call that either "good enough" or "refuse," and both are wrong.

```rust
pub struct IsolationFloor {
    pub network:    NetworkRequirement,  // Denied — NOT LOWERABLE. There is no config key.
    pub filesystem: FsRequirement,       // Confined (default) | Unconfined (explicit opt-out)
    pub strength:   Strength,            // Reduced (default) | Full ("Elevated" in the UI)
}
```

**`network: Denied` has no setting.** SR-1 says no conversion process reaches the network, *ever*; if that is true it is not a preference, and a policy key that can turn it off is a policy key an attacker or an impatient admin will turn off. `route()` emits no executable steps for any `Sandboxed` step whose profile does not `denies_network()`, and a property test asserts it over every profile the prober can produce. That is what makes "ever" a true word.

### 9.2 The ladder, re-derived from what is actually reachable

v0.4's `Reduced` tier required a bind-mount namespace, which requires a user namespace — precisely what is missing in three of the four cases it listed as causes of a drop. The machine fell through to `Minimal`, below the floor, and refused: the outcome the profile was invented to prevent. The mistake was coupling Landlock to user namespaces. **Landlock needs neither privilege nor a namespace** — only `no_new_privs` and kernel ≥ 5.13.

| | Linux | macOS | Windows |
|---|---|---|---|
| **Full** | Landlock + seccomp-bpf + **empty netns** + cgroup v2 + `no_new_privs` + user namespace | App Sandbox profile + `sandbox_init` in the worker, **no network entitlement**, hardened runtime, notarized, rlimit | **AppContainer** (low-privilege SID) + restricted token + **Job Object** + **no network capability SID** + ACG/DEP/CFG. **CIG does not engage — see below** |
| **Reduced** | **Landlock + seccomp-bpf (network denied at the syscall filter) + rlimit + `no_new_privs`.** No user namespace required. | *(none — App Sandbox is present on every supported release)* | AppContainer + Job Object, without ACG/CIG |
| **Minimal** | seccomp-bpf + rlimit + `no_new_privs`, **no filesystem confinement** | — | restricted token + low integrity + Job Object, **no network denial** |
| **Meets the default floor?** | Full ✅ · Reduced ✅ · Minimal ❌ *(fs)* | Full ✅ | Full ✅ · Reduced ✅ · Minimal ❌ *(network)* |
| **Typical cause of a drop** | kernel < 5.13 → no Landlock → Minimal. `unprivileged_userns_clone=0`, unprivileged container, AppArmor userns restriction → **Reduced, which still converts** | — | ACG incompatible with an injected AV DLL → Reduced. Managed-desktop policy blocking AppContainer → Minimal |

The hardened-Debian case — the one v0.4 named and did not fix — now lands on **Reduced and converts**, with Landlock confining the filesystem and seccomp denying the network, and the UI naming both. The cases that still refuse are the ones that genuinely cannot satisfy SR-1 or SR-2, and the message says which mechanism is missing and what the user can do:

> `blocked — network confinement unavailable`
> `AppContainer is disabled by policy "Restrict App Containers" on this machine.`
> `Without it an engine cannot be denied network access. Ask your administrator to permit`
> `AppContainer for signed applications, or run OpenConvert on a machine where it is available.`

`Policy::floor.strength` defaults to `Reduced`. `Full` ("Elevated" in the UI) refuses anything less. Files carrying mark-of-the-web or a quarantine attribute raise the floor to `Full` automatically — **and that detection fails open**, which is stated in [09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against) rather than implied away: a file that reached the disk by a route that strips the zone identifier looks local, because the OS has told us nothing else.

**Two Windows corrections from measurement, one of which corrects an earlier correction.**

**ACG comes from the container, not from the request.** A 2×2 matrix — plain vs AppContainer, mitigation requested vs not — read `ProhibitDynamicCode` back from each cell (S17b, S20b):

| | no mitigation requested | ACG requested |
|---|---|---|
| **plain process** | 0 | 0 |
| **AppContainer** | **1** | **1** |

An earlier revision of this work published *"ACG is requested successfully and silently declines"* — measured on a **plain process**, which is not a configuration this product ever creates. ACG engages wherever the `Full` row actually runs. The explicit request is redundant, not broken.

**CIG genuinely does not engage.** Re-tested the same way, a non-Microsoft DLL loaded successfully in **all four cells**. `Full` is therefore constructible as written, with one of its four listed mitigations not doing anything. It stays in the table because requesting it is free and costs nothing if a future Windows build honours it — but it is not load-bearing, and [§9.5](#95-the-profile-is-read-back-not-requested) means the receipt will never claim it engaged.

**The platforms are safe in opposite places.** This is the single most consequential thing the measurements produced, and one model for both platforms cannot express it:

| | Windows | Linux |
|---|---|---|
| `..` through the job-directory handle | **Refused by the kernel** (`0xC0000033`) | **Escapes** — an ordinary component to `openat` |
| Descriptors the child was not handed | **64 visible, 2 writable** by default | **0** — `CLOEXEC` is the default |

Two consequences follow, and both change what a component is *for*:

1. **`OutputName::parse`'s separator rejection is load-bearing on Linux and defence-in-depth on Windows.** [§5.2](#52-the-hostworker-protocol) previously described the `openat` discipline as "defence in depth, not the guarantee" — true on Windows, and on Linux it is closer to the guarantee whenever Landlock is absent.
2. **The Linux `Minimal` tier provides no filesystem containment whatsoever**, which is now an empirical justification for refusing it rather than a cautious one. Landlock blocks the same traversal with `EACCES`, which is why it is mandatory on Linux and not merely preferred.

A worker can also apply Landlock to **itself** using only the descriptor it was handed — `landlock_add_rule` takes a `parent_fd` — so confinement composes with "no names at all" without the worker ever learning a path.

### 9.5 The profile is read back, not requested

I11 seals `SandboxProfile` so that only the OS prober may construct one. That guards against a *dishonest* constructor. It does not guard against an **honest one recording a request the OS ignored** — and measurement shows that error runs in both directions: CIG is requested and does not engage; ACG engages without being requested.

So the profile is built **in the child, after it starts**, from read-back queries — `GetProcessMitigationPolicy` on Windows, the applied ruleset on Linux — and returned over the protocol as a `wire::` type, validated like any other ([§5.2](#52-the-hostworker-protocol)).

**Divergence between requested and engaged is a first-class event.** It lowers the reported tier, appears in the plan preview, and is recorded in the receipt. A CI test requests a mitigation known to be declined on the test machine and asserts the resulting profile does **not** claim it.

> The receipt is the product's second pillar. A receipt that records what we *asked for* is a receipt about our intentions; only one that records what *happened* is evidence.

### 9.3 macOS, honestly

> ⚠️ **Unmeasured. This is the only section of this document with no supporting spike.** Windows and Linux claims here have been tested; every macOS claim in this record is reasoning. The runbook that would settle it is the private macOS validation runbook — eight tasks, run once per architecture — and the headline question is exactly the one this section answers by assertion. **Nothing below has been revised in v0.6, deliberately: editing it without a Mac would only launder an assumption into a finding.**

`sandbox_init` has been formally deprecated since 10.8. It works, every shipping sandboxing tool on the platform uses it, and Apple has not removed it — but a design that depends on a deprecated API should say so and carry a plan. Ours: the worker binaries are separately signed with their own entitlements, so if `sandbox_init` is withdrawn the fallback is App Sandbox entitlements applied to the sidecar at signing time rather than at runtime. That is a packaging change, not an architecture change, which is why this is a risk and not a blocker.

### 9.4 The profile absorbs WASM — with the honest caveat

The browser tool pages ([04-GROWTH §5](04-GROWTH.md#5-free-tool-pages)) compile `openconvert-core` to `wasm32` and pair it with a thin in-page executor. It reports `filesystem: BrowserOrigin, network: Csp, resources: Rlimit`.

**Two things must be said about that, which v0.4 did not say.** The page origin confines what the *page* can reach; it does not confine `image-rs` from the rest of the page's memory — so `BrowserOrigin` is a weaker claim than `Landlock`, and `strength()` scores it `Reduced`, never `Full`. And `Rlimit` in a browser means the WASM memory ceiling, not a wall-clock limit; wall time is enforced by the executor's own watchdog. The variants are named for what they are. The payoff of making the profile a value is real; the payoff is not that a new target gets to describe itself flatteringly.

---

## 10. Limits, budgets, and conflicts

### 10.1 Per step

```rust
pub struct Limits {
    pub wall_time:           Duration,
    pub cpu_time:            Duration,
    pub memory_bytes:        u64,
    pub output_bytes:        u64,
    pub temp_bytes:          u64,
    pub expansion_ratio:     u32,   // output ÷ input
    pub decode_pixels:       u64,
    pub archive_depth:       u8,
    pub archive_entries:     u32,
    pub archive_total_bytes: u64,
}
```

A required field of `Step` (I4), derived by `route()` from `Policy` and `Facts`, shown in the plan preview, recorded in the receipt.

#### The values

v0.5 specified ten fields and **not one number** — the only figure anywhere in the record was a `120 s` inside an example error string. A limit without a threshold is not a control, so here they are, derived against libarchive 3.8.5 (S23):

| Field | Default | Where the number comes from |
|---|---|---|
| `archive_depth` | **32** | Legitimate nesting topped out at 8 in testing; the bomb was 41 |
| `archive_entries` | **100_000** | The bomb had 40,000 — **and so does [01 §7](01-VISION.md)'s archivist workload.** This one cannot be tightened without breaking a named user |
| `expansion_ratio` | **≥ 10_000, tripwire only** | **Not a primary control.** See below |
| `archive_total_bytes` | from `Budget` | With a job budget, this is what actually bounds a bomb |
| `wall_time` | `base + k · input_bytes`, per `MediaKind` | A four-hour remux and a 5 MB JPEG cannot share a constant |
| `memory_bytes` | **1 GiB** default, per-`MediaKind` override | Above the largest legitimate working set measured; enforced by the OS, so overshoot is recoverable |
| `decode_pixels` | **256 Mpx** | ~1 GiB at 4 bytes/px — the in-process ceiling, checked from the header |
| `output_bytes` · `temp_bytes` | from `Budget` | Per-job, not per-step |
| `cpu_time` | `2 × wall_time` | Catches a spin that a wall clock alone would let run |

**`expansion_ratio` cannot separate a bomb from real data, and the design must stop implying it can.** Measured:

| Archive | Ratio |
|---|---|
| 200 MB zeroed disk image — **legitimate** | **1029×** |
| 512 MB zip bomb | **1028×** |

Any threshold that catches the bomb also refuses compressed disk images, VM images, database dumps and sparse files. **Depth and absolute size do the work**; the ratio is a tripwire for logging, set high enough to be meaningless as a defence.

**libarchive imposes nothing of its own** — 512 MB extracted from a 522 KB archive with no error, no warning and no cap. Every limit here comes from us.

#### How each is enforced

| Path | Mechanism |
|---|---|
| `Sandboxed` | cgroup v2 (Linux) / Job Object (Windows) / `setrlimit` + App Sandbox (macOS), plus a host-side watchdog for wall time — the OS enforces even if the engine is compromised |
| `InProcess` | **enforced before allocation.** `decode_pixels` is read from the header and checked *before* decode, then passed to the decoder's own limit API (`image::Limits`). `output_bytes` goes through a counting writer that errors at the cap. `wall_time` is a cancellation token checked in the decode loop. |

The in-process case matters because `InProcess` is the default for the highest-volume formats: a 4 KB PNG declaring 500 megapixels is a ~2 GB allocation in the same address space as the GUI. Checking the header before decoding costs nothing and is the whole fix — and it produces an actionable error rather than an OOM kill. Note that `expansion_ratio` is no more discriminating here than for archives: a 4 KB PNG of flat colour legitimately decodes to hundreds of megabytes. **`decode_pixels`, read from the header, is the control that works.**

Archive limits are checked **during** extraction against a running total, so a bomb is stopped before it is realised.

#### When a limit kills the worker, the host must still be able to explain it

A Job Object memory cap works — a capped worker dies where an uncapped one survives — but it **terminates the process with `0xC0000409` and no `Failed{..}` message** (S14). [§13](#13-failure-is-designed) promises a message naming the file, the step and the engine, and an abort code supplies none of that.

The fix is measured: **associate the Job Object with an IO completion port.** `JOBOBJECT_ASSOCIATE_COMPLETION_PORT` delivers `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` *before* the exit code arrives (S18), so the host synthesises the message the worker could not send. This is part of worker start, not an optional diagnostic.

**Linux differs and the record should not flatten it.** `RLIMIT_AS` produces a recoverable allocation failure, so a Linux worker can catch it and send a proper `Failed{..}` itself. Windows cannot; it needs the completion port. One design, two mechanisms, stated rather than averaged.

### 10.2 Per job

```rust
pub struct Budget {                 // spans every step of a run
    pub total_temp_bytes:   u64,
    pub total_output_bytes: u64,
    pub max_parallel_jobs:  u16,
}
```

`Limits` is per step. A 40,000-file archival batch can therefore fill a disk without any single step exceeding anything — which is exactly what the archivist persona would hit first. `Budget` is checked by the executor between steps and before each worker spawn, and the preflight for a batch reports the projected total against free space *before* the first file.

**Note where the default comes from.** The sensible default is "20% of free space, capped at 50 GB" — and free space is a filesystem read, which the core may not do. So `Budget` is a plain value carried in `Environment`: the shell measures free space once when it builds the environment, computes the default, and hands it in. The core never learns what a disk is. This is the same shape as the sandbox profile, and it is worth stating because "put a sensible default on the type" is exactly how a clock, an environment variable or a `statvfs` call leaks into a pure crate — one leak and the testability argument in [§4](#4-functional-core-imperative-shell) is gone.

### 10.3 Conflicts

```rust
pub enum OnConflict { Suffix, Skip, Fail }   // default: Suffix
```

Resolved before any file is opened, applied to both the job-dir side and the copy-out. Combined with I12 (`O_EXCL` everywhere, no truncating open in the codebase), this closes the one path by which a dropped file could destroy data the user already had. `Suffix` produces `report (2).pdf`; the receipt records the name actually written, so an automated caller is never confused about what it got.

---

## 11. Detection in two phases, and input identity

### 11.1 The two phases

You cannot sandbox based on facts you do not have yet, and `FileFacts` must carry properties for `route()` to evaluate requirements — properties which, for the flagship format, come from libheif. So the question is split.

```rust
// Phase 1 — pure Rust, in-process, safe. Decides identity and isolation.
pub fn sniff<R: Read + Seek>(src: &mut R, hint: Option<&OsStr>) -> Sniff;

// Phase 2 — inside the sandbox, over the worker protocol. Extracts properties.
pub fn probe(sniff: &Sniff, input: &InputFd, limits: &Limits) -> Properties;
```

**Phase 1** uses `infer`, `tree_magic_mini`, and the magic signatures in the format table — all pure Rust, no `unsafe`, fuzzed. It yields the `FormatId`, the declared-vs-detected mismatch, and polyglot signals. That is enough to choose an isolation level.

**Phase 2** runs the format-specific parser under exactly the isolation the conversion will use, as the first message on the worker session — so it costs a message, not a process. For formats whose properties come from a pure-Rust header parse (PNG, JPEG, WAV), phase 2 stays in-process and is free.

`Properties` is an **enum per `MediaKind`** (`Image`, `Audio`, `Video`, `Document`, `Archive`, `Tabular`), not one flat struct. Adding a format inside a kind touches no shared type; adding a kind touches one.

**The mismatch is surfaced, always.** A PostScript file named `.jpg` routes as PostScript, the receipt records both types, and the plan preview shows a warning. Polyglots are quarantined and the user is asked. The UI's file-type icon is drawn from the *detected* type.

### 11.2 Streaming

Both take `Read + Seek`, not `bytes`. Sniffing needs the first few KB; probing seeks. Nothing loads a file to decide what it is — which is what lets the multi-gigabyte `mkv→mp4` demo work at all.

### 11.3 Input identity

v0.4 read the input three times: sniff read the path, the engine re-opened it, and the receipt hashed it. Nothing tied those reads together, so on a network share, a sync folder, or any machine with a second process, the file routed on need not be the file converted — and the receipt could honestly attest to bytes that were never touched. For a product whose second pillar is verifiable conversion, that is the wrong failure.

```rust
pub struct FileFacts {          // in openconvert-core — no OS resources
    token:      InputToken,     // opaque; resolves in -run's HandleTable
    content_id: Blake3,         // computed once, through the one open handle
    sniff:      Sniff,
    props:      Properties,
    provenance: Provenance,
}
```

- **One open.** `detect()` opens the file and `-run` keeps the handle in its `HandleTable`. Every later read — the header check, the decode, the hash — goes through it. That defeats replace-by-rename, which is the realistic attack.
- **The handle is not in the core.** v0.5 put an `InputHandle` field here, which put a live OS resource inside the crate [§4](#4-functional-core-imperative-shell) declares has no I/O and tests with no files. The token/table pairing gives the same guarantee and keeps the core testable without a filesystem.
- **Copy when the handle is not enough.** If provenance is untrusted (mark-of-the-web, quarantine xattr), or the file is on a network or removable volume, the broker **copies** into the job dir first and everything downstream reads the copy. Cost: one copy, on exactly the inputs where it is worth paying.
- **`content_id` is the receipt's subject.** The receipt records the hash of the bytes that were actually converted, not of a fourth read.

**Provenance is captured once, before the copy, and never re-derived from it.** This is a v0.6 correction, and it closes a conflict between two controls that were each individually right. [§9.2](#92-the-ladder-re-derived-from-what-is-actually-reachable) raises the isolation floor for files carrying mark-of-the-web. This section copies untrusted files before routing. But measurement shows the copy destroys the evidence (S26):

| Copy method | `Zone.Identifier` |
|---|---|
| `fs::copy` | preserved |
| `fs::rename` | preserved |
| **`read` + `create_new` + `write`** | **STRIPPED** |

The third is the path I12 mandates, because `create_new` is *"the only creation API"*. **The copy taken *because* a file is untrusted would erase the mark that says it is untrusted** — and since detection already fails open, the file would then look local. So `Provenance` is a field of `FileFacts`, established at `detect()` time before any copy exists, and no later stage re-reads the zone identifier.

The residual — an attacker with write access to the same inode modifying it in place while we hold a read handle — is real, is not fixable this side of copying everything, and is written down in [09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against).

---

## 12. State on disk

Six things persist. Until v0.5 not one of them had a location, a format, or an owner — and every one is attacker-influenceable, which is why they get a section rather than a footnote.

```
<state>/                     platform: %LOCALAPPDATA%\OpenConvert · ~/Library/Application Support/OpenConvert · $XDG_STATE_HOME/openconvert
├── config.toml              the user layer of Policy                 (user-authored)
├── history.jsonl            prediction history, append-only          (our writes, user-readable)
├── quarantine.json          crash counters + the engine revocation list
├── recipes/*.recipe.toml    saved recipes                            (SHAREABLE — untrusted)
├── journal/<batch>.jsonl    PlanRequests for resumable batches
└── jobs/<id>/               live job directories + lock files        (swept at startup)
```

Four rules govern all of it:

1. **Every file here is untrusted input on read.** Including the ones we wrote: a corrupted `history.jsonl` is an ordinary consequence of a power cut, and a hand-edited one is an ordinary consequence of the file being documented and user-readable. Each has a parser, each parser is fuzzed, and a parse failure degrades rather than aborts — a bad history line is skipped, a bad `config.toml` falls back to defaults *and says so*.
2. **Nothing here can influence a security decision.** `quarantine.json` may only ever *disable* an engine path, never enable one; `config.toml` cannot lower the isolation floor's network requirement ([§9.1](#91-the-floor-is-three-predicates-not-one-number)); `history.jsonl` feeds prediction, which ranks targets and cannot arm a Class D operation (I8). An attacker who owns this directory owns the user's account anyway — but they still cannot use it to turn the sandbox off.
3. **Recipes are the one shareable format**, so they are the one with a trust story: a recipe stores a `PlanRequest` and a plan hash, never a `Plan`; opening one re-routes; a hash mismatch is shown, not executed. Signing is post-v1 and gated on a key model existing — an unsigned recipe is treated exactly as untrusted as a signed one whose key we do not know, which is what makes deferring the signature safe.
4. **`jobs/` is under our own per-user directory, never a shared temp root.** The startup sweep deletes directories whose lock is unheld; a sweep pointed at `/tmp` would be a deletion primitive for any local user. Two implementation constraints attach to this directory and are easy to miss: **each job dir carries an AppContainer-SID ACL on Windows** ([§5.3](#53-what-the-implementer-must-not-discover-later)), so the sweep must expect permissions the installer did not set; and **the path is length-bounded at creation** ([§8.2](#82-outbound)) so that job root + deepest legal archive member stays inside `MAX_PATH` for the C engines.

---

## 13. Failure is designed

Every one of these has a mechanism, a message and a test. The message rule is [§7.5](#75-errors)'s: name the file, the step and the engine, and say what to do.

| Failure | What happens | What the user sees | Test |
|---|---|---|---|
| **Malformed input** | Engine returns `Failed`; no output copied out | `photo.heic — decode failed in oc-images (libheif 1.19.5). The file appears truncated or corrupt.` | fuzz + corpus |
| **Truncated file** | Sniff succeeds, decode fails; if the container declares a length we can check, we fail *before* the engine | `download.mp4 — declares 480 MB, file is 12 MB. Likely an interrupted download; re-download and retry.` | `failure::truncated` |
| **Zero-byte file** | `sniff` returns `FormatId::Unknown`; `route()` yields no steps and a `Warning` | `notes.txt — the file is empty. Nothing to convert.` | `failure::empty` |
| **Wrong extension** | Routed by content; both types in the receipt and the preview | `invoice.jpg is actually PostScript. Routing as PostScript.` | `detect::routes_by_content` (SR-4) |
| **File larger than RAM** | Streaming throughout; `Limits.memory_bytes` caps the engine | *(no message — this is the normal path)* | `limits::large_tiff` |
| **Read-only destination** | Checked in **preflight**, before any work | `Cannot write to /Volumes/Archive — the volume is read-only. Choose another destination.` | `failure::readonly_dest` |
| **Disk full mid-write** | The counting writer's `ErrorKind::StorageFull`; partial output removed; batch pauses rather than failing every remaining file | `Disk full after 312 of 4,000 files. 312 completed. Free space and run 'openconvert resume <batch>'.` | `failure::disk_full` |
| **Destination collision** | `Policy::on_conflict`, resolved before opening (I12, [§10.3](#103-conflicts)) | `report.pdf exists — writing report (2).pdf.` | `traversal::collision` (SR-15) |
| **Receipt cannot be written** | **The step fails and the output is removed.** A receipt is not optional; an output without one breaks the product's second pillar | `Converted photo.jpg but could not write its receipt (permission denied). Output removed. Use --no-receipt to convert without one.` | `failure::receipt_unwritable` |
| **Engine hangs** | `Limits.wall_time` → watchdog → `SIGKILL`/`TerminateJobObject` | `movie.mkv — oc-images exceeded 120 s and was stopped. Raise with --wall-time.` | `limits::wall_time` |
| **Engine crashes** | `StepOutcome::Crashed`; recorded honestly; three on one format auto-disables that path locally | `scan.pdf — oc-pdf (pdfium 128.0) crashed (SIGSEGV). No output written. This is the 2nd crash on PDF; a 3rd disables this path.` | `lifecycle::engine_crash` |
| **App killed mid-job** | Lock released; startup sweep reaps the job dir; no partial output was ever copied out | *(next launch, silently clean)* | `lifecycle::kill_mid_conversion` (SR-14) |
| **Cancelled** | `CancelToken` → pipe closed → kill, bounded at 500 ms | `Cancelled. 4 of 12 files completed; nothing partial was written.` | `lifecycle::cancel_bounded` (SR-14) |
| **Below the isolation floor** | `route()` returns no steps and a blocking `Warning` naming the mechanism ([§9.2](#92-the-ladder-re-derived-from-what-is-actually-reachable)) | see the block message in §9.2 | `route::below_floor_yields_no_steps` |
| **Memory limit trips (Windows)** | Job Object terminates the worker with no `Failed{..}`; the **completion port** supplies the reason ([§10.1](#101-per-step)) | `scan.tif — oc-images exceeded its 1 GiB memory limit and was stopped. Raise with --memory.` | `limits::job_object_memory_reported` |
| **Memory limit trips (Linux)** | `RLIMIT_AS` returns a recoverable allocation error; the worker sends `Failed{..}` itself | *(same message, different mechanism)* | `limits::rlimit_recoverable` |
| **A mitigation was requested and did not engage** | The read-back profile omits it, the reported tier drops, the receipt records the divergence ([§9.5](#95-the-profile-is-read-back-not-requested)) | `Running with Reduced isolation — Code Integrity Guard is unavailable on this machine.` | `isolation::readback_rejects_unengaged` |
| **An output name resolves to a device on Windows** | Refused by `OutputName::parse` before any write | `Archive member "NUL" refused — reserved device name.` | `traversal::device_names` (SR-15) |
| **Engine library not readable by the container** | Worker fails before `main`; the loader error is translated rather than surfaced raw ([§5.3](#53-what-the-implementer-must-not-discover-later)) | `oc-images could not start — its engine libraries are not readable inside the sandbox. Reinstall to repair permissions.` | `packaging::container_can_load_engines` |
| **Archive entry collides under NFC** | `Policy::on_conflict`, exactly like any other collision ([§8.2](#82-outbound)) | `Two entries in scan.zip resolve to "café.jpg" — writing café (2).jpg.` | `traversal::nfc_collision` |

**The rule this table encodes:** a failure that has not been given a message has not been designed. Every row was written before the code, which is the only time it is cheap.

> **Six rows are new in v0.6, and every one exists because a spike produced a failure this table could not describe.** That is the intended relationship between measurement and this document: a measurement that does not add a row, change a number or delete a claim was not worth running.

---

## 14. Prediction

Five signals, hand-weighted. **The scoring function is pure and lives in `openconvert-core`**; gathering the signals (reading `history.jsonl`, listing the folder) is I/O and lives in `openconvert-run`.

That split is not tidiness. v0.4 put prediction in `openconvert/src/predict.rs` — inside the *CLI binary* — while the GUI, which is where "the guess is the product" actually happens, is a different binary that cannot link it. One implementation would have become two, then three, and they would have diverged on the one behaviour the product is named for.

| # | Signal | Why it's in v1 |
|---|---|---|
| 1 | Detected file type | The cold-start prior, from public search volumes |
| 2 | Personal history for this type | Dominates after ~3 uses |
| 3 | **`has_sibling_output`** — does `photo.jpg` already sit next to `photo.heic`? | Near-certainty; the strongest single signal |
| 4 | Batch shape (one file vs many) | Separates single-file tools from batch export |
| 5 | Source folder category | `Screenshots` → compress; `Scans` → OCR |

Blend: `w(n) = n / (n + 3)` — one formula gives the whole cold-start curve (global at drop 1, personal by drop 6).

Prediction ranks **targets**, never plans. It cannot arm a Class D operation, because `route()` filters on `Policy::max_auto_class` regardless of what suggested the target (I8) — a property test asserts it over every signal combination. It also cannot influence isolation, limits or the conflict policy, all of which are `route()`'s.

**How a sixth signal earns its place:** it measurably improves top-1 acceptance on a frozen, hand-curated benchmark set the model never trains on. Build that set *before* adding any signal.

Everything in [06-ML-RUNBOOK](06-ML-RUNBOOK.md) — a learned ranker, a bandit, the advisor LLM — happens after v1 ships and there is data.

---

## 15. The seams — how it grows

Upgradability comes from one rule:

> **Make the extension points data, not code** — and be honest about where that is not true.

### 15.1 What extension actually costs

v0.4's version of this table listed one or two files per row and omitted the most common change of all. These counts are derived from the file layout in [§5.1](#51-what-each-crate-is) and are what a contributor should expect.

| To add… | Files | Where the cost is |
|---|---|---|
| **A new format** | **1–2** | One row in `formats.toml`, including its magic signature — detection genuinely needs no code. **Plus a `Properties` variant only if the format's kind is new.** Routes come next but are a separate change |
| **A conversion between existing formats** | **2–5** | Route entry + a `Requirement` variant if it needs a new one + an adapter if the `StepKind` is new + a golden fixture. If a new engine is involved, add its rows below |
| **A new engine** | **6–8, across 3 crates** | Worker crate under `engines/` + an `EngineBin` variant (**in the boundary crate — this is a reviewed, CODEOWNED change**) + an adapter in `-run` + `engines.toml` licence and linkage + a fuzz target + packaging and signing entries. Not cheap, and it should not be: a new engine is a new attack surface |
| **A field on `Step` or `Plan`** | **8–10, plus the TS client** | `plan.rs` · `route.rs` (populate) · property tests (construct) · `receipt.rs` · the `version` decision and the compatibility path for old receipts · CLI human output · CLI `--json` + the parity test · the generated `ipc.ts` · `PlanTable.svelte` · later, the server's OpenAPI. **This is the most expensive change in the system** and it is the price of `Plan` being one inspectable value serving five surfaces. It is worth paying; it is not worth pretending is free |
| **A new isolation mechanism** | **6–8, across 2 crates + 2 renderers** | An enum variant in `core/isolation.rs` · `strength()` / `denies_network()` / `confines_filesystem()` scoring · the **probe** in `-os` · the **application** in `-os/<platform>.rs` (probing is not engaging — v0.4's table listed only the probe) · the fallback ordering · the GUI badge label and hover text · the CLI renderer · the ladder table in [09 §4](09-THREAT-MODEL.md#4-per-os-implementation-and-fallbacks) · an escape test for the new tier |
| **A new prediction signal** | **2 + a measurement** | One function and one weight in `core/predict.rs`, one gatherer in `run/state/history.rs` — genuinely additive since [§14](#14-prediction) moved it. The real cost is the frozen benchmark run that justifies it |
| **A new CLI subcommand** | **3–4** | `cmd/x.rs` + the clap enum in `main.rs` + human and `--json` rendering + the parity test |

**Three things genuinely cost less than they look**, and the design earned them:

- **The plan preview is free** — it is `route()` without `execute()`.
- **`openconvert routes`, `openconvert formats`, the `/formats` site section and the docs conversion matrix are one artefact.** A wrong route is a one-line diff visible in four places.
- **The WASM build of the core is free.** Pure, no I/O, no clock, no `unsafe` — it compiles to `wasm32` unchanged, which is what makes the tool pages possible at all. **That build is a portability gate, not a purity gate** — `std` is available on `wasm32`, so it would compile a core full of file handles just as happily ([§4](#4-functional-core-imperative-shell)).

### 15.2 Compatibility

- **`Plan` and `Receipt` carry a `version` field from commit one.** Receipts persist on disk and in customer archives.
- **`Plan` serialises but never deserialises.** Anything that receives a plan — the server, a recipe, a resumed batch — stores a `PlanRequest` and a plan hash, and re-routes. If the re-routed hash differs, the environment changed and the caller is told.
- **The core crate gets semver discipline** the moment anything outside the repo depends on it.
- **Ranker and prior updates are declinable**, same as an engine version.

### 15.3 What is deferred, and the trigger that fires

A trigger nobody can detect is a deletion in disguise. Three of v0.4's were exactly that, and are rewritten here.

| Deferred | Trigger | Observable how? |
|---|---|---|
| `RasterIR` | The first `Route` needing two `StepKind`s with a raster in between | **In the route table.** The person adding the route hits it |
| **Module system + WASM tier** | ~~"A real third-party module wanting to exist"~~ → **the third external pull request adding a format adapter, or month 12, whichever first** | **Countable.** The old trigger was circular: nobody can want a module for a system with no ABI, so it could never fire — while [04-GROWTH §9](04-GROWTH.md#9-community) makes modules the community strategy |
| microVM tier + Paranoid mode | A design-partner contract naming untrusted-document workflow | A signature |
| **Headless server** | ~~"A design partner asking"~~ → **the first design partner contract, or the first 25 GitHub issues tagged `server`** | Countable. And [04-GROWTH](04-GROWTH.md) no longer builds a revenue model on a component the plan defers — see its §10 |
| Daemon | Watch folders shipping | A feature landing |
| **Content-addressed cache** | Batch conversion — **which is week 31, inside v1** | It fires during the build, so it is budgeted there rather than deferred to nowhere |
| **Shell extension** (Explorer / Finder) | **v1.1, unconditionally** — it is scheduled, not deferred | See [SR-17](09-THREAT-MODEL.md#5-requirements-to-tests); the v1 cut is [§1](#1-what-changed-in-v05-and-why) item 8 |
| Learned ranker | ~~"Website data"~~ → **10,000 opt-in local history exports, or the first design partner supplying a corpus** | The old trigger was unusable: the tool pages record only aggregated `(from → to)` pairs, which can inform signal 1 and nothing else — you cannot train a personal-history ranker on data with no person in it |
| Advisor LLM | The format knowledge base proving insufficient — measured as advisor-eligible questions the tables cannot answer | A counter |
| Studio node canvas | Evidence anyone wants it | Issue count |

---

## 16. Decisions

| # | Decision | Choice |
|---|---|---|
| D1 | Language | **Rust** — the thesis is "we parse hostile input safely" |
| D2 | GUI | **Tauri v2 + Svelte 5**, with a strict `csp` and no remote origin, gated in CI ([08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates)). Rejected pure-Rust toolkits: accessibility is a procurement gate and Rust GUI a11y is materially behind a webview |
| D3 | Licence | **Apache-2.0** — the patent grant matters given codec patents |
| D4 | FFmpeg | **Not in core.** Optional module, LGPL-only, subprocess, prefers a system install |
| D5 | Ghostscript | **Not in core.** A PostScript interpreter that escaped its own sandbox in the wild |
| D6 | Crates | **Five**, each from a stated trigger. Plus our engine worker binaries |
| **D7** | **Engine artifact** | **We build and ship four worker binaries.** No stock third-party CLI, no linked engine in `-run` |
| D8 | Routing | **Ordered route table**, not graph search |
| D9 | Isolation | **Two variants + a sealed profile value**, growing by field |
| **D10** | **The floor** | **Three predicates. `network: Denied` is not lowerable and has no config key** |
| **D11** | **Process model** | **`Policy::worker_reuse`, a user setting.** `Balanced` (default) reuses a worker across files of trusted provenance and never across an untrusted one; `Isolated` is one process per file. Raisable by policy, never lowerable. No daemon until watch folders ([§8.4](#84-the-worker-session-and-how-many-files-one-worker-sees)) |
| D12 | Plan versioning | `version` from commit one; `Plan` serialises, `PlanRequest` deserialises |
| D13 | Broker direction | **Bidirectional, on both paths.** Inputs are fds; outputs are parsed names under a job root |
| D14 | Spawn API | **`EngineBin` is a closed enum.** No program is nameable by string |
| D15 | Limits | **A required field of `Step`**, plus a per-job `Budget` |
| **D16** | **Isolation on `Step`** | **A required field of `Step`.** SR-2's carrier |
| **D17** | **Writes** | **`O_EXCL` only**, enforced by a lint over **eleven** destructive APIs, not one ([§7.4](#74-impossible-hard-forbidden)); conflicts are policy, resolved before opening |
| **D18** | **Update check** | **On by default, weekly, contentless, disableable** — the only default-on network call, and the reason the kill switch works. See [09 SR-9](09-THREAT-MODEL.md#5-requirements-to-tests) |
| **D19** | **Engine linkage** | **Dynamic, always.** LGPL relinking is discharged by shipping the libraries as separate files; `engines.toml` records `link_mode` and CI fails an LGPL engine that is static ([02 §11](02-FEATURES.md#11-model-licensing-policy)) |
| **D20** | **Trust across the wire** | **`wire::` types only, `Deserialize` in one module, limits may narrow and never widen** ([§5.2](#52-the-hostworker-protocol)) |
| **D21** | **What the receipt records** | **What engaged, read back from the OS in the child** — not what was requested ([§9.5](#95-the-profile-is-read-back-not-requested)) |

---

## 17. Build sequence

**44 weeks, two engineers.** v0.4 committed to 25 weeks without stating a headcount, which is not a schedule — and was roughly 2× optimistic on its two hardest phases. The numbers below are estimates, and the two marked ⚠️ are the ones most likely to move.

| Step | Weeks | Outcome |
|---|---|---|
| **0 Measure** | 0 | Trademark, registries, and the two numbers: **worker-set packaged size** (not a linked stub) and bare confined spawn cost, on all three OSes |
| **1 Core** | 1–2 | The twelve pure files: types, both tables, `route()`, `Limits` **with values**, `Isolation`, `Environment`, `wire`, `predict`. Property tests, zero I/O, **purity and mutation gates live from w1** |
| **2 Skeleton** | 3–4 | `openconvert convert a.png -t jpg` end-to-end, in-process, with limits and conflict policy enforced, a receipt, and eight new CI gates. **Plus JPEG metadata strip in w3** — the first Class A operation, which is what gives the Class A gate a subject ([08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates)). **Already shippable** |
| **3 Boundary** ⚠️ | **5–14** | `openconvert-sandbox` + `openconvert-os` + the worker protocol. Windows 7–9 (Tauri sidecar spike in 9), Linux 10–12 (three tiers), macOS 13, escape + overhead + fuzz + sandboxed probe in 14 |
| **4 Breadth** ⚠️ | **15–26** | Weeks 15–17 build the **worker template**: one crate, cross-compiled and signed for three OSes, in CI. Then `oc-images`, `oc-pdf`, `oc-archive`, `oc-audio`, one at a time, each with a fuzz target and a golden corpus |
| **5 Containers** | 27–30 | `AvGraph`, remux route, lossless trim, audio extraction. **`mkv→mp4` = stream copy, asserted bit-identical in CI** |
| **6 CLI & state** | 31–33 | Batch + resume from the journal, recipes, naming templates, the content-addressed cache, `verify`, `doctor`, `config explain`, the batch manifest |
| **7 GUI** | 34–38 | Tauri + Svelte on the same library. Drop → prediction → plan → result. a11y and CSP gates in CI |
| **8 Prediction** | 39–40 | Benchmark set first, then the five signals wired to history. p99 < 10 ms as a CI gate |
| **9 Ship** | 41–44 | Signing, notarization of every worker binary, installers, package managers, LGPL compliance artifacts, docs, threat model, SBOM, launch |

**Why 10 weeks on the boundary and not 6.** AppContainer with a restricted token, a Job Object and no network capability SID is a multi-week job alone; so is Landlock + seccomp + cgroup v2 with three probed tiers and honest fallback reporting; and macOS is the *hardest* week, not the easiest, because confining a child process there is genuinely awkward and `sandbox_init` is deprecated ([§9.3](#93-macos-honestly)). v0.4 also put the macOS sandbox, the escape tests, the sandboxed probe, the overhead gates and the fuzz targets in the same week.

**Why 12 weeks on breadth and not 5.** With [D7](#16-decisions) resolved, this is a build-engineering phase: four worker crates, each cross-compiled and code-signed for three platforms, each speaking the probe protocol. The first three weeks build the template so the other nine are repetition rather than discovery.

### The kill criterion, restated so it can be measured

Overhead is dominated by process startup, which is a fixed cost — so a percentage is not a well-defined quantity without naming the workload. **Nor is a single number well-defined across platforms**, which v0.5 assumed and measurement refuted: the same operation costs **40.15 ms on Windows and 1.96 ms on Linux**, a 20× gap that no amount of engineering on our side closes.

The gate is therefore stated **per platform, and on the amortised per-file basis** that [`worker_reuse`](#84-the-worker-session-and-how-many-files-one-worker-sees) creates:

| Gate | Threshold | Basis | Checked |
|---|---|---|---|
| Added latency, 5 MB JPEG, p50 — **Linux, macOS** | ≤ 25 ms | either `worker_reuse` setting | w14 |
| Added latency, 5 MB JPEG, p50 — **Windows, `Balanced`** | ≤ 25 ms | amortised over a 12-file batch | w14 |
| Added latency, 5 MB JPEG, p50 — **Windows, `Isolated`** | **≤ 60 ms** | per file, unamortised — **the honest cost of the strongest setting** | w14 |
| Added latency, 5 MB JPEG, p99 — all platforms | ≤ 80 ms · **Windows `Isolated` ≤ 140 ms** | as above | w14 |
| Throughput regression, 200 MB TIFF streaming path | ≤ 5% | — | w14 |
| **Windows AppContainer + Tauri sidecar** | **a spike proves it, or the GUI plan changes** | — | **w9** |

All of them are rows in [08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates), because a criterion that is not in the gate table is prose. The Tauri gate moved to week 9 for a reason: it was previously due in week 10 and could not be tested, because no Tauri application existed until week 20. A three-day spike in week 9 answers it while the answer can still change the architecture.

> **The `Isolated` row is deliberately a different number rather than a waiver.** A user who chooses maximum isolation should get a slower product, not an unmeasured one — and a gate that quietly exempts the strongest setting would leave the safest configuration as the only untested one.

Miss the latency gates and the response is to move more formats onto the in-process path — never to weaken the sandbox. That response is now measured as well as principled: **the sandbox is 4.43 ms of the 40**, so weakening it would buy 11% of the problem while giving up the entire premise. Miss the Windows gate and the architecture needs rethinking before any UI exists.

---

## 18. Rules that keep it simple

1. **Eight nouns, honestly counted** — and about twenty concepts. A ninth noun is a signal to stop and write the thinking down, as [§3](#3-the-whole-system-in-one-line) does for the eighth.
2. **Rule of three.** Duplicate twice; abstract on the third.
3. **Data over code.** New capability should be a table row — and where it is not, [§15.1](#151-what-extension-actually-costs) says so with a number.
4. **Pure core.** If it does I/O, reads a clock, or reads the environment, it doesn't belong in `openconvert-core`.
5. **No interface with one implementation** *and no named second one*.
6. **`unsafe` never shares a crate with a compile-time security invariant.**
7. **Every abstraction pays rent** in code deleted or bugs prevented — measured, not assumed.
8. **No type may be constructible outside the crate that owns its invariant** — and where the language cannot enforce that, [§7.4](#74-impossible-hard-forbidden) says which class it really is.
9. **Every threat has a requirement, every requirement has a test, every test fails the build**, at a week the test can actually pass.
10. **Every failure has a message, decided before the code** ([§13](#13-failure-is-designed)).
11. **Plan detail decays with distance.** Four weeks of commits, then outcomes.
12. **When two designs work, ship the one a newcomer understands faster.**

---

## Sources

[YAGNI and premature abstraction](https://dev.to/walternascimentobarroso/yagni-the-principle-that-protects-you-from-building-the-future-too-early-2o7d) · [Premature abstraction in system design](https://codeworld.blog/posts/system%20design/architecture/PrematureAbstraction/) · [The rule of three](https://en.wikipedia.org/wiki/Rule_of_three_(computer_programming)) · [Rule of three and the wrong abstraction](https://www.ruzora.com/blog/rule-of-three-and-the-wrong-abstraction) · [Cargo workspace best practices](https://reintech.io/blog/cargo-workspace-best-practices-large-rust-projects) · [Rust module and crate organization](https://softwarepatternslexicon.com/rust/idiomatic-rust-patterns/module-and-crate-organization-best-practices/) · [Functional core, imperative shell](https://kennethlange.com/functional-core-imperative-shell/) · [Hexagonal architecture in Rust](https://www.howtocodeit.com/guides/master-hexagonal-architecture-in-rust) · [Ports and adapters, and the mess in the middle](https://levelup.gitconnected.com/ports-and-adapters-and-the-mess-in-the-middle-dbe1d98c7172) · [Parse, don't validate](https://github.com/andrewbanchich/parse_dont_validate_rs) · [Making illegal states unrepresentable in Rust](https://medium.com/@indrajit7448/the-rust-deep-dive-making-illegal-states-unrepresentable-with-type-driven-design-%EF%B8%8F-a0b4055dccac)
