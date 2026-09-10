# OpenConvert — Execution Plan

What to build, in what order, with how many people, and where every file lives.

Status: execution record, **v0.5 — phases 2–4 status revision** · 2026-08-23
Part 8 of 9 — see [README](README.md). Architecture: [03-ARCHITECTURE](03-ARCHITECTURE.md). Threat model: [09-THREAT-MODEL](09-THREAT-MODEL.md).

---

## Status summary (2026-08-23)

Phases 2–4 are complete on this machine. The table below records what shipped
and what is deferred, per phase, so the next contributor starts from the truth
rather than from the plan's original optimism.

### Phase 2 — Boundary and OS · ✅ complete

| Done | Deferred |
|---|---|
| Windows: AppContainer + restricted token + Job Object + no-network SID + ACG/CIG read-back + IOCP for limit attribution | macOS sandbox (no Mac available; runbook in the private macOS validation runbook) |
| Linux: Landlock deny-all-FS + arch-checked seccomp network denial, self-applied by the worker before reading any byte | cgroup v2 resource enforcement (RLIMIT_AS used instead; recoverable-error property is what matters) |
| Full tier: unshare(CLONE_NEWUSER\|CLONE_NEWNET\|CLONE_NEWNS) escalation, fork-probed, read-back via `/proc/self/ns/net` identity change; verified engaging in Docker with seccomp=unconfined | Pre-opened fd broker (`fd 3/4/5`) — stdin/stdout used instead; descriptors land when the shell streams |
| RLIMIT_AS pre-exec parity with Job Object memory cap | Formal w14 latency benchmark harness (spawn costs measured ad hoc in Phase 0 spikes) |
| DLL ACLs inside AppContainer (§5.3 packaging trap, found by running HEIC through a confined worker) | Tauri sidecar spike (w9 gate — GUI is phase 8) |
| Escape tests in forked children (one-way filters must not leak into shared test processes) | |

### Phase 3 — Breadth · ✅ complete

| Done | Deferred |
|---|---|
| oc-audio: symphonia decode → hound WAV / flacenc FLAC; lossless pair proven bit-identical through confined worker | LAME / libopus encoding (MP3/Opus output has NO ROUTE until linked — availability honesty rule) |
| oc-images: image-rs (6 codecs) + jxl-oxide (JXL decode, pure Rust) + libheif FFI (HEIC decode, dynamic-linked LGPL, DLLs copied by build.rs and ACL'd by host) | libvips / libraw / lcms2 (RAW formats, colour management) |
| image→PDF writer (`openconvert-pdf`, in-process): JPEG verbatim DCTDecode, PNG re-flated with alpha composite disclosure; xref offsets asserted | pdfium/qpdf rendering (PDF→image direction needs oc-pdf worker; not yet built) |
| CSV⇄JSON tabular adapter (csv crate, RFC 4180 edge cases handled) | AVIF decode removed from route table (no AV1 decoder in engine set; returns when dav1d/aom linked) |
| Route-table honesty: Docx/Odt→Pdf removed (Office module), Class B video rows removed (FFmpeg module), Avif row removed (no decoder) | Docx→Pdf rendering (Office module territory) |

### Phase 4 — Containers · ✅ complete

| Done | Deferred |
|---|---|
| Bounded Matroska demuxer into AvGraph (depth ≤16, ≤100k elements, advancing cursor, byte budget, lacing refused) | Lossless trim/concat at keyframes (cue-index parsing is distinct work) |
| Two-pass deterministic MP4 muxer (stts coalescing, TimecodeScale-derived media timescale, no frame-rate guessing) | Chapter/tag/subtitle translation between containers (disclosed as dropped) |
| MKV→MP4 stream copy end-to-end via CLI, bit-identical samples verified by independent Python EBML+ISO-BMFF parser | |
| WebM⇄MKV surgery (elements splice verbatim), MKV→MKA extraction with disclosure | |
| Fuzz target: demux + both surgery modes + remux over arbitrary bytes | |

### Cross-phase

| Done | Deferred |
|---|---|
| ~340 tests across 31 suites, green Windows + Debian Docker | Real bare-metal Linux confinement re-verification (Docker/WSL2 caveat documented) |
| 11 CI steps green (`cargo xtask ci`) | arm64 confinement verification (emulation never validates confinement — TARGETS.md) |
| 7 fuzz targets | Mutation testing re-run after new code landed |

---

## How to use this

**Two engineers. 44 weeks. Both numbers are stated because a duration without a headcount is not a schedule.**

**Plan detail decays with distance.** Weeks 1–4 are planned to the commit. Everything after is planned to the *outcome*, because commit-level planning beyond a month is fiction that gets rewritten on contact.

Five rules govern the whole thing:

1. **Walking skeleton, not a spike.** Build the thinnest end-to-end slice that works, then thicken it. Nothing is written to be deleted.
2. **The CLI is the product.** The GUI is a client; the server is a later client.
3. **No AI, no modules, no daemon, no server, no shell extension in v1.** Each has an observable trigger in [03 §15.3](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires).
4. **A requirement without a test is not done** — and a test gated at a week it cannot pass is not a gate. Every `SR-*` in [09 §5](09-THREAT-MODEL.md#5-requirements-to-tests) names its test and a week that test can actually run.
5. **Every failure has a message, written before the code** ([03 §13](03-ARCHITECTURE.md#13-failure-is-designed)).

### What changed from v0.3

| Was | Now | Why |
|---|---|---|
| 4 crates | **5, plus four worker binaries** | `EngineBin` names programs, so those programs must exist and we must build them. And `unsafe` cannot share a crate with a compile-time invariant ([03 §5](03-ARCHITECTURE.md#5-five-crates-and-the-engine-workers)) |
| Week 0 measured a **linked stub** | Week 0 measures the **packaged worker set** | The linked configuration is the one the architecture forbids. Measuring it would have produced a number for a build we will never ship |
| 25 weeks, headcount unstated | **44 weeks, two engineers** | +4 on the boundary (6 → 10), +7 on breadth (5 → 12), +2 on containers, +1 on the CLI, +2 on the GUI, +3 on shipping — 25 + 19 = 44. The old plan put the macOS sandbox, the escape tests, the sandboxed probe, the overhead gates and the fuzz targets in one week |
| Sandboxing weeks 5–10, split disagreeing with [03](03-ARCHITECTURE.md) | **Weeks 5–14**, one split, stated once | v0.3's two documents disagreed about which OS was done in which week |
| Breadth = 10 adapters in 5 weeks | **Weeks 15–26**, first three on the worker template | With the artifact model resolved this is build engineering: four worker crates cross-compiled and signed for three platforms |
| `Step` = `Class` + `Limits` | **`Class` + `Limits` + `Isolation`** | SR-2 named an enforcement that did not exist |
| Windows shell extension in v1, on by default | **Cut to v1.1**, with [SR-17](09-THREAT-MODEL.md#5-requirements-to-tests) | It was our only code outside our own process, and it had no requirement, no test, no gate and no file |
| Prediction in `openconvert/src/predict.rs` | **`core/src/predict.rs`** | It lived in the CLI *binary*, which the GUI cannot link — and the GUI is where "the guess is the product" happens |
| Recipes: a fuzz target, no week | **Weeks 31–33**, with the state layer | A shareable file format was scheduled to be fuzzed before it was built |
| Prediction after the GUI | **Prediction wired at 39–40, scored from week 2** | The GUI at 20–22 listed "suggestion" as a deliverable and the signals arrived at 23–24 |
| 26 commits | **28 commits** over four weeks | Isolation, `Environment`, prediction scoring, the conflict policy and the identity handle all land in the skeleton |

---

## Contents

1. [Week 0 — before any code](#1-week-0--before-any-code)
2. [Weeks 1–4 — the walking skeleton, by commit](#2-weeks-14--the-walking-skeleton-by-commit)
3. [Weeks 5–44 — outcomes](#3-weeks-544--outcomes)
4. [The CI gates](#4-the-ci-gates)
5. [File schema](#5-file-schema)
6. [Where things live](#6-where-things-live)
7. [Parallel non-engineering tracks](#7-parallel-non-engineering-tracks)
8. [Weekly rhythm](#8-weekly-rhythm)
9. [Early warning signals](#9-early-warning-signals)

---

## 1. Week 0 — before any code

### Secure the name *(day 1 — gates everything)*

- [ ] Register **`openconvert.dev`**; `openconvert.dev` + `openfile.dev` → 301; `convertit.dev` defensively
- [ ] **USPTO + EUIPO search**, Nice classes **9** and **42**
- [ ] Reserve GitHub org · crates.io · npm · PyPI · Docker Hub

> ⚠️ A conflict in class 9 or 42 means **stop and revisit** — runners-up are in [01-VISION §10](01-VISION.md#10-the-name--decided). Changing a name in week 0 costs a day; in month 6 it costs a rebrand.

### Repository

- [ ] Apache-2.0 `LICENSE` + `NOTICE`
- [ ] `README.md` carrying the **permanent licence promise** — never relicensed, naming the HashiCorp/Redis precedent
- [ ] `SECURITY.md`, `.well-known/security.txt`, disclosure policy
- [ ] **Design docs into `/docs`**, including [09-THREAT-MODEL](09-THREAT-MODEL.md); public roadmap **and the non-goals list**

### Toolchain

- [ ] Five-crate workspace + an `engines/` member for the worker binaries, `rust-toolchain.toml`, MSRV declared
- [ ] CI: `fmt` · `clippy -D warnings` · `test` · **three licence gates** · **four source-scan lints** · **`depcheck`** · **`clippy.toml` purity** · **`cargo-audit` (`--target`-filtered)** · **`cargo-vet`**
- [ ] **`clippy.toml` checked in**, with `disallowed-types` and `disallowed-methods` covering `std::fs`, `std::net`, `SystemTime::now` and `std::env` — this, not a `wasm32` build, is what enforces the pure core ([03 §4](03-ARCHITECTURE.md#4-functional-core-imperative-shell)). A `wasm32` build stays as a **portability** gate and is labelled as one.
- [ ] `#![forbid(unsafe_code)]` in `openconvert-core`, `openconvert-sandbox` **and** `openconvert-run` — **as an audit property, not a linkage guarantee.** A crate with `forbid(unsafe_code)` and no `unsafe` token links a C library and calls into it perfectly happily (S2); linkage is enforced by `depcheck`. `openconvert-os` is the only *host-side* crate permitted `unsafe`, confined to syscalls **and process creation**. `engines/*` may use `unsafe` for FFI only — on the far side of the boundary, in a process that is already confined.
- [ ] `corpus/benign/` for ordinary test files
- [ ] **`corpus/src/evil.rs`** — the traversal, collision, ADS, device-name and normalisation cases, **generated in-repo from checked-in code**, so the SR-13 and SR-15 gates run on pull requests from forks. Real exploit samples never enter the repository and run in a separate nightly ([09 §9](09-THREAT-MODEL.md#9-security-engineering-practices)).

### Decide the target matrix — new in v0.6, because CPU architecture appeared in **zero** live documents

The build matrix was "three platforms", six times over. Architecture is not a packaging detail here: **every confinement mechanism in this design is architecture-specific**, and one of our own spikes compiled for `x86_64-unknown-linux-gnu` and **failed to compile** for `aarch64-unknown-linux-gnu` because C's `char` differs in signedness (S15).

- [ ] **Name the OS × architecture matrix**, and split it two ways: targets that get a *build*, and targets that get *confinement assurance*. They are not the same list and pretending otherwise is how a green CI matrix comes to mean nothing.
- [ ] **State the support floor.** Landlock ≥ 5.13 **plus runtime ABI negotiation** — the kernel we tested reported **ABI 3, not 4**, and passing an access bit it does not know is `EINVAL`, so a filter built for the newest ABI silently fails to apply on a slightly older kernel (S10b). Declare an MSRV and a Windows build floor at the same time.
- [ ] **Decide static vs dynamic linkage for the engines** — now settled as **dynamic**, and gated ([03 D19](03-ARCHITECTURE.md#16-decisions)). Recording it here because it is a week-0 decision that binds packaging, signing and the LGPL position all at once.
- [ ] **Write down the rule: emulation validates compilation and logic, never confinement.** `docker buildx` advertises `linux/arm64` while being unable to execute it until binfmt is registered, and under emulation the Landlock syscalls returned `ENOSYS` from a kernel that served them correctly to an amd64 container minutes earlier (S12b). A CI matrix that builds for a target it never executes reports green while testing nothing.

### Measure the three numbers that constrain everything

- [ ] **Build and package one worker binary** — a `oc-images` stub linking libvips, libheif, libavif, libjxl, libraw and lcms2 — cross-compiled and code-signed for all three platforms, and record the **installed size of the packaged set**, including shared libraries and signing overhead. The `<60 MB` budget is an unverified estimate; if the real number is 90 MB, say 90 MB now rather than breaking a published promise later. **Two reference points from spikes, neither of which replaces this measurement:** the engine `.so` closure installs at **42.7 MiB** on Debian x86_64, and a Rust binary that really links libvips is **300 KiB** alone, **15 MiB** with the 103 shared libraries it actually touches (S12e, S35). Those are one platform, one worker, dynamically linked against system copies — a shipped product bundles its own, adds three more workers, the Tauri shell and the CLI, and pays signing overhead.
  > This replaces v0.3's "link a stub binary against the intended core engine set." That measured a *linked* build — the configuration [D7](03-ARCHITECTURE.md#16-decisions) forbids and the early-warning table says to revert on sight. It would have produced a confident number for a build that will never ship.
- [ ] **Time a bare confined spawn** on all three OSes — the floor for the week-14 latency gates. Knowing it in week 0 tells you whether those gates are reachable before you build toward them.
- [ ] **Time a `postcard` round-trip over a pipe** for a 4 KB probe response. The worker protocol is on the critical path of every conversion; if it is not sub-millisecond the two-phase-detection design needs revisiting.

---

## 2. Weeks 1–4 — the walking skeleton, by commit

**Goal: `openconvert convert a.png -t jpg` works end to end with a receipt, limits and isolation on every step, nothing overwritten, and eleven new CI gates green.** Everything after this is breadth and confinement.

### Week 1 — the pure core, part 1

| # | Commit | Changes | Files |
|---|---|---|---|
| 1 | `chore: five-crate workspace` | Workspace + `engines/`, MSRV pin, Apache-2.0, `forbid(unsafe_code)` where it applies | `Cargo.toml`, `rust-toolchain.toml`, `LICENSE`, `NOTICE` |
| 2 | `chore(ci): gates and lints, live from commit two` | Matrix over three OSes. Lints: no `Command::new` outside `-os`; **no destructive filesystem API (eleven, not one)**; one outbound call site; **`Deserialize` only in `core/src/wire.rs`**. Gates: **`depcheck`** — no package in `-run`'s normal dependency graph has a non-null `links` key, which is the *working* enforcement of D7 that `forbid(unsafe_code)` was wrongly credited with; **`clippy.toml` purity** over `-core` plus a grep for `#[allow(clippy::disallowed`; **`cargo-audit --target`-filtered**; **`cargo-vet`** with a non-increasing exemption count | `.github/workflows/ci.yml`, `deny.toml`, `clippy.toml`, `engines.toml`, `models.toml`, `supply-chain/`, `xtask/{lint,depcheck}.rs` |
| 3 | `feat(core): format identity and table` | `FormatId`, `MediaKind`, formats as **data — including magic signatures**, so detection needs no code per format | `core/src/format.rs`, `core/data/formats.toml` |
| 4 | `feat(core): facts` | `Sniff`, `Properties` (**an enum per `MediaKind`**), `FileFacts` with **private fields and an opaque `InputToken`** — the handle itself lives in `-run` (commit 17), because a live OS resource in the pure core contradicts three claims this plan makes about it | `core/src/facts.rs` |
| 5 | `feat(core): target` | `Target` — a format, an operation, or a preset | `core/src/target.rs` |
| 6 | `feat(core): policy` | Three layers: defaults → user → flags. Carries `max_auto_class` (default `B`), **`on_conflict` (default `Suffix`)**, **`worker_reuse` (default `Balanced`; raisable to `Isolated`, never lowerable)**, and the **three-predicate floor** whose `network` field has no setter | `core/src/policy.rs` |
| 7 | **`feat(core): limits, budget, and a ceiling`** | Per-step `Limits` **with the measured defaults** ([03 §10.1](03-ARCHITECTURE.md#101-per-step)) — `archive_depth` 32, `archive_entries` 100_000, `decode_pixels` 256 Mpx, and `expansion_ratio` demoted to a ≥10,000 tripwire because it cannot separate a bomb from a disk image; per-job `Budget` so a legal 40,000-file batch cannot fill a disk; **`LimitCeiling`, which engine-reported facts may narrow and never widen** | `core/src/limits.rs` |

### Week 2 — the pure core, part 2

| # | Commit | Changes | Files |
|---|---|---|---|
| 8 | `feat(core): isolation vocabulary, sealed` | `Isolation`, `SandboxProfile` (**private fields; constructor behind `feature = "attest"`**), `Strength`, `IsolationFloor` | `core/src/isolation.rs` |
| 9 | **`feat(core): environment`** | The eighth noun: the route table, the engine registry with each engine's declared minimum isolation, the machine's profile | `core/src/environment.rs` |
| 10 | `feat(core): plan types` | `PlanRequest` (**deserialises**), `Plan` (**serialises only**), `Step` with **`Class`, `Limits` and `Isolation` all non-optional**, `Warning`, `version` | `core/src/plan.rs` |
| 11 | `feat(core): route table and router` | `Route`, `Requirement`, `RouteTable::routes_for()`, **`route(req, env)`** — filters on `max_auto_class`, the isolation floor, and network denial | `core/src/route.rs` |
| 12 | **`feat(core): prediction scoring`** | The five signals as a **pure** function over supplied values. No history I/O, no clock — which is what makes the p99 gate a criterion benchmark and gives every surface one implementation | `core/src/predict.rs` |
| 12b | **`feat(core): the wire boundary`** | `core/src/wire.rs` — **the only module in `-core` or `-sandbox` implementing `Deserialize`**, asserted by commit 2's lint. Every `wire::` value converts to its domain type through a fallible constructor that clamps against `LimitCeiling`; `OutputName` crosses as `String` and is re-parsed, never derived | `core/src/wire.rs` |
| 13 | **`test(core): the ten properties`** | `every_step_has_limits` · `every_step_has_isolation` · `no_unsafe_engine_inprocess` · `no_default_policy_arms_class_d` · `no_steps_without_network_denial` · `every_pair_in_kind_has_route_or_reason` · `below_floor_yields_no_steps` · **`worker_reuse_never_lowers`** · **`untrusted_never_shares_a_worker`** · **`engine_facts_cannot_widen_limits`**. Pure, no fixtures, no files. *(The last three are v0.6. Each guards a claim v0.6 introduced — and each was missing from the first draft of that revision, which is the same defect this plan has produced in every round: a feature and its enforcement claim written in one edit, with the test written in none.)* | `core/tests/properties.rs` |
| 14 | `fuzz: route table + predict` | Free, because the core is pure — and a panic in either is a DoS in every front end at once | `fuzz/fuzz_targets/{route,predict}.rs` |
| 14b | **`test(ci): mutation gate on the core`** | `cargo-mutants` at a rising threshold, starting 80%. **This audits commit 13, not the code** — measured against a comparable parser, 7.4M fuzz cases found nothing while 30 mutants exposed a test that passes when its own input is deleted (S29, S30) | `.github/workflows/mutants.yml` |

> **Gate:** the brain of the application exists, is pure, and is fully tested — with zero I/O written. `cargo test -p openconvert-core` runs in under a second and touches no file. **SR-1 and SR-2 have property gates from here**, ten weeks before their OS-level counterparts, because the structural half of each claim is testable now.

### Week 3 — the shell, in-process only

| # | Commit | Changes | Files |
|---|---|---|---|
| 15 | `feat(run): streaming sniff` | `sniff<R: Read + Seek>` — table-driven magic from `formats.toml`, `infer` → `tree_magic_mini` fallback. **Never loads the file into memory.** | `run/src/detect/sniff.rs` |
| 16 | `feat(run): polyglot and mismatch detection` | Multi-signature → quarantine recommendation; declared-vs-detected recorded | `run/src/detect/polyglot.rs` |
| 17 | **`feat(run): input identity, the handle table, and provenance`** | `detect()` opens once; **`-run`'s `HandleTable` owns the handle and `FileFacts` carries only its `InputToken`**; `content_id` computed through it; mark-of-the-web / quarantine read **at detect time and never re-derived from a copy** — because the mandated `create_new` path strips `Zone.Identifier` (S26); **copy-before-route for untrusted provenance and volatile volumes** | `run/src/detect/{mod,provenance}.rs`, `run/src/handles.rs` |
| 18 | `feat(run): engine registry` | A concrete registry over concrete types, **declaring each engine's minimum isolation** — which is what commit 13's `no_unsafe_engine_inprocess` reads. **No trait**: there is one engine, and a trait with one impl is a guess. It arrives in week 15 with two. | `run/src/engines/mod.rs` |
| 19 | **`feat(run): image-rs adapter with limits`** | PNG/JPEG/WebP/GIF/BMP/TIFF, in-process, no unsafe. **`decode_pixels` checked from the header before decode**; counting writer for `output_bytes` | `run/src/engines/image_rs.rs` |
| 20 | **`feat(run): executor, cancellation, and `O_EXCL` writes`** | Step dispatch, `CancelToken`, the **only** file-creation helper in the crate — `create_new`, no truncating variant — and `on_conflict` resolved before opening | `run/src/lib.rs`, `run/src/write.rs` |
| 20b | **`feat(run): JPEG metadata strip — the first Class A operation`** | Copy every JPEG segment except `APP1`/`APP13`/`COM`. **The image is never decoded**, which is what makes it trivially lossless and cheap to assert. Strips GPS, camera serial, author and edit history. **This exists so that commit 23's Class A gate has a subject** — and it gives **A8/SR-11**, which until now had a policy and no test, its first real one | `run/src/ops/strip_metadata.rs` | **A8/SR-11** |
| 21 | `feat(run): receipts` | Engines, versions, params, class, **limits, isolation, profile**, `content_id`, hashes, declared-vs-detected. A receipt that cannot be written **fails the step** | `run/src/receipt.rs` |

### Week 4 — the skeleton is alive

| # | Commit | Changes | Files | Requirement |
|---|---|---|---|---|
| 22 | `feat(cli): convert, inspect, plan, routes, formats` | `--json`, `--dry-run`, exit codes. **`openconvert routes mkv mp4` prints the table** — inspectability is a stated feature, so it gets a command | `openconvert/src/main.rs`, `.../cmd/*.rs` | |
| 23 | **`test: Class A round-trips byte-identically — metadata half`** | The lossless claim, enforced **against a subject that exists**: commit 20b's metadata strip. Every byte outside the stripped segments is identical, and the pixels are untouched because they are never decoded. **The re-container half of this gate is week 30**, when `AvGraph` lands — staged the way SR-5 and SR-13 already are. *v0.5 gated both halves at w4 and named as its subject two operations that do not exist until w15 and w27; the rule broken was this document's own.* | `tests/reproducibility.rs` | |
| 24 | `test: Class B — deterministic mode and perceptual floor` | **Under `--deterministic` (pinned encoder threads) Class B is byte-identical; otherwise SSIM ≥ a pinned reference.** Rebaselining needs `--rebaseline` and a reason | `corpus/`, `tests/golden.rs` | |
| 25 | **`test: limits and budget`** | A 4 KB PNG declaring 500 megapixels errors before allocating, with an actionable message; a batch projected past free space refuses in preflight | `run/tests/limits.rs` | **SR-5** |
| 26 | `test: detection routes by content` | A PostScript file named `.jpg` routes as PostScript; the receipt records both types | `run/tests/detect.rs` | **SR-4** |
| 27 | **`test: nothing is ever overwritten`** | Outputs colliding with existing files suffix, skip or fail per policy — never truncate. Source scan asserts no truncating open exists | `run/tests/conflict.rs` | **SR-15** |
| 28 | `test: one outbound call site, and none during conversion` | Source scan finds exactly one; a full conversion under a null route makes zero connection attempts. Plus the sniffer fuzz target | `tests/network.rs`, `fuzz/fuzz_targets/sniff.rs` | **SR-9** |

> **Gate: the walking skeleton is alive.** `openconvert convert a.png -t jpg --json` produces a real file and a real receipt, refuses a pixel bomb, routes by content, **never overwrites anything**, and phones nobody. **This is already shippable software** — and it cannot be crashed by a 4 KB file or made to destroy one.

---

## 3. Weeks 5–44 — outcomes

Planned to the outcome. Commits get planned one week ahead, in the week before.

### Weeks 5–14 — the security boundary

The one genuine architectural risk. **Windows first**, because it is the hardest target and the largest `heic→jpg` audience; discovering it doesn't work in month 4 would be fatal.

| Week | Outcome | Done when |
|---|---|---|
| **5** | `openconvert-sandbox` exists: `EngineBin` (closed enum), `Argv` (private fields), `BrokeredPath`, `OutputName`, `DestinationName`, `BrokeredOutput`, the `Worker` skeleton, and the **protocol types** | A compile-fail test proves `EngineBin` cannot be built from a string; a source scan proves `Command::new` appears nowhere else **(SR-3)** |
| **6** | The broker, both directions and **both paths**. Opaque names and pre-opened fds inbound; `OutputName::parse` with the full rejection set outbound; job-dir lock; startup sweep; the state-file parsers | The generated evil corpus all fails closed **(SR-13 names)**; SIGKILL mid-job leaves nothing **(SR-14)**; a rename-swap between detect and execute changes nothing **(SR-16)**; every state file survives corrupt, hostile and hand-edited variants **(SR-19)** |
| **7–9** | **Windows sandbox** — AppContainer, restricted token, Job Object, no-network capability SID, ACG/CIG/DEP/CFG, plus the `Reduced` tier and its honest reporting. **Week 9 also runs the Tauri sidecar spike** | `heic→jpg` converts inside AppContainer; a machine without ACG converts at `Reduced` *saying so*; a machine that cannot deny network is **refused with a message naming AppContainer**. **The AppContainer + Tauri sidecar kill-criterion gate is answered here**, while the answer can still change the architecture |
| **10–12** | **Linux sandbox** — Landlock, seccomp-bpf, cgroup v2, empty netns, user namespace, and the `Reduced` tier that **needs none of the last two** | Converts on stock Ubuntu (`Full`), a Debian with `unprivileged_userns_clone=0` (**`Reduced`, and it converts** — this is the case v0.3 refused), an unprivileged container (`Reduced`), and a kernel < 5.13 (`Minimal`, **refused, naming Landlock**) |
| **13** | **macOS sandbox** — App Sandbox, `sandbox_init` in the worker, no network entitlement, hardened runtime, per-worker signing | `heic→jpg` converts confined; the deprecation fallback in [03 §9.3](03-ARCHITECTURE.md#93-macos-honestly) is written down and the signing path proven |
| **14** | Escape tests **per reachable tier**, the sandboxed probe over the protocol, overhead gates, protocol fuzzing both directions | **All four kill-criterion gates green** ([§4](#4-the-ci-gates)); escape tests cover network, filesystem, process and traversal on every tier the prober can produce **(SR-1, SR-2)**; the receipt records a reduced profile correctly **(SR-11)** |

> **Kill criterion, checked at week 9 and week 14.** Miss the latency gates → move more formats to the in-process path before optimising the sandbox; never weaken the sandbox. Miss the Windows AppContainer + Tauri sidecar gate → **stop and rethink the architecture before any UI exists.** That gate moved from week 10 to week 9 because in v0.3 it was due before any Tauri application existed and therefore could not have been run.
>
> If it passes: publish the numbers. *"How we sandboxed media engines on three operating systems"* is the first content asset, and the work is already done.

### Weeks 15–44

| Step | Weeks | Outcome | Done when |
|---|---|---|---|
| **4a Worker template** | 15–17 | **One worker crate, done properly**: `oc-images` with libvips, cross-compiled and code-signed for three platforms in CI, speaking the probe and run protocol, with a fuzz target and a golden corpus. The `Engine` trait lands here, with image-rs and libvips as its two implementations. Installer-size gate goes live against the week-0 measurement | A contributor can add an engine to an existing worker by following one documented path, and CI produces signed artifacts for three platforms without manual steps |
| **4b Breadth** | 18–26 | `oc-images` completed (libheif, libavif, libjxl, libraw, lcms2, resvg) · `oc-pdf` (pdfium, qpdf) at 21–22 · `oc-archive` (libarchive, bomb caps) at 23–24 · `oc-audio` (LAME/Opus/FLAC; Symphonia decode stays in-process) at 25–26. **A fuzz target per adapter, each a gate** | Every core-and-no-AI conversion in [02-FEATURES](02-FEATURES.md) works, sandboxed, within limits. **SR-5's archive gates and SR-13's extraction gate go live at w24** |
| **5 Containers** | 27–30 | `AvGraph`, remux route, lossless trim/concat, audio extraction, metadata policy | **`mkv→mp4` selects stream copy and is bit-identical.** Asserted in CI |
| **6 CLI & state** | 31–33 | The state layer ([03 §12](03-ARCHITECTURE.md#12-state-on-disk)): config, history, journal, **recipes**, quarantine. Batch + resume, the **batch manifest**, the content-addressed cache (its trigger fires here), naming templates, `verify`, `doctor`, `config explain` | `openconvert plan --json` and `openconvert convert --json` agree on every field; a 40,000-file run produces one manifest and survives a mid-run kill |
| **7 GUI** | 34–38 | Tauri + Svelte on the same library. Design tokens → CSS · class chips (greyscale- and glyph-verified) · drop → suggestion → plan preview → progress → receipt · undo · mixed-drop grouping · `Panic` · settings · dark mode · **a11y and CSP gates in CI**. **The suggestion is real here, not a placeholder**: the scorer has existed since week 2, and signal 1 (the cold-start prior from the format table) needs no history — so the GUI shows a correct armed suggestion with its "why" line from day one of this phase, and step 8 adds personalisation on top of a surface that already works | Drop 12 HEIC → `Enter` → ~3 s, GPS stripped, receipt available, the plan preview names the sandbox profile, and **the packaged app makes zero connection attempts (SR-18, w35)** |
| **8 Prediction** | 39–40 | **Benchmark set first.** Then the history gatherer wired to the scorer built in week 2, confidence gating, alternates, fuzzy search. p99 < 10 ms as a CI gate | Ten drops of a type produce a correct armed suggestion with an honest explanation — and never a Class D one |
| **9 Ship** | 41–44 | Signing, **notarization of every worker binary**, installers, package managers, **the LGPL compliance artifacts** (notice, licence texts, source mirror for libvips/libheif/libraw/LAME), docs site, published threat model, SBOM, **"what we don't protect against"**, launch content | A stranger on a clean Windows machine converts a HEIC without reading anything |

---

## 4. The CI gates

Every invariant the project claims is here, with the week it starts failing the build. **If it is not in this table, it is prose.**

Two rules this table now obeys that v0.3's did not: **a gate's week is a week its test can actually pass** (v0.3 gated archive-bomb tests eight months before archives existed), and **every criterion the plan calls a kill criterion is a row** (v0.3 cited "all four" against a table containing three).

| Gate | Threshold | Requirement | Live from |
|---|---|---|---|
| `fmt` · `clippy -D warnings` · `test` | clean | — | w1 |
| Crate licence scan (`cargo-deny`) | no GPL/AGPL/non-commercial | — | w1 |
| **Engine licence scan** (`engines.toml`) | every native engine declared, with licence and linkage (subprocess vs linked) | — | w1 † |
| **Model licence scan** (`models.toml`) | every model declared with licence + commercial-use + sha256 | — | w1 † |
| **Lint: no `Command::new` outside `openconvert-sandbox`** | source scan; `EngineBin` not constructible from a string | SR-3 | w1 |
| **Lint: no truncating open anywhere** | source scan over `-sandbox` and `-run`; `create_new` is the only creation API | **SR-15** | **w1** |
| **Lint: exactly one outbound network call site** | source scan | SR-9 | w1 |
| **`depcheck`: `-run` links no native library** | no package in the normal dependency graph has a non-null `links` key | **D7** | **w1** |
| **Purity: `clippy.toml` over `-core`** | no disallowed type or method; no `#[allow(clippy::disallowed` | — | **w1** |
| **`cargo-audit`, `--target`-filtered** | no advisory on a target we ship | — | **w1** |
| **`cargo-vet` exemption count** | never increases | — | **w1** |
| **Lint: `Deserialize` only in `wire.rs`** | source scan over `-core` and `-sandbox` | **I14** | **w2** |
| **Mutation score, `openconvert-core`** | ≥ 80%, rising | — | **w2** |
| **Property: every step has limits** | — | SR-5 | w2 |
| **Property: every step has an isolation** | — | **SR-2** | **w2** |
| **Property: no memory-unsafe engine is `InProcess`** | — | **SR-2** | **w2** |
| **Property: no plan without network denial yields steps** | — | **SR-1** | **w2** |
| Property: no default policy arms Class D | — | — | w2 |
| Property: every in-kind pair has a route or a reason | — | — | w2 |
| Property: below the floor yields no steps | — | — | w2 |
| **Property: `worker_reuse` never lowers across policy layers** | — | **SR-20** | **w2** |
| **Property: an untrusted-provenance file never shares a worker** | — | **SR-20** | **w2** |
| **Property: engine-reported facts cannot widen a limit** | clamped against `LimitCeiling` | **SR-5, I14** | **w2** |
| Fuzz: route table · prediction | no panic | — | w2 |
| **Class A byte reproducibility — metadata edits** | bit-identical, over the w3 JPEG strip | **A8/SR-11** | **w4** |
| **Class A byte reproducibility — lossless re-container** | bit-identical | — | **w30** |
| **Class B: byte-identical under `--deterministic`; SSIM floor otherwise** | both | — | w4 |
| In-process limits | pixel bomb errors before allocating | SR-5 | w4 |
| **Per-job budget** | a batch projected past free space refuses in preflight | SR-5 | w4 |
| Detection by content | `.jpg`-named PostScript routes as PostScript | SR-4 | w4 |
| **Nothing is overwritten** | colliding outputs suffix/skip/fail; originals intact | **SR-15** | **w4** |
| Receipt completeness | every executed step appears, with limits and isolation | SR-11 | w4 |
| **Receipt is not optional** | a receipt that cannot be written fails the step | SR-11 | w4 |
| No outbound network during conversion | zero connection attempts | SR-9 | w4 |
| Fuzz: sniffer | no panic | — | w4 |
| **Path-traversal name corpus** (generated in-repo) | every evil name fails closed | SR-13 | w6 |
| Fuzz: `OutputName::parse` · `DestinationName::parse` | no panic, no accept | SR-13 | w6 |
| **Job lifecycle** | SIGKILL leaves no orphan; cancel completes < 500 ms | SR-14 | w6 |
| **Input identity** | a rename-swap between detect and execute changes nothing; the receipt matches the converted bytes | **SR-16** | **w6** |
| **State files degrade, never escalate** | corrupt / hostile / hand-edited variants of each | **SR-19** | **w6** |
| Fuzz: state-file parsers · worker protocol (both directions) | no panic | SR-19 | w6 |
| **Windows AppContainer + Tauri sidecar** | a spike proves it, or the GUI plan changes | — | **w9** |
| **Sandbox escape, per reachable tier** | network, filesystem, process escapes all fail at the OS | SR-1, SR-2 | w14 |
| **Added latency, 5 MB JPEG, p50 — Linux, macOS** | ≤ 25 ms | — | w14 |
| **Added latency, 5 MB JPEG, p50 — Windows, `worker_reuse: Balanced`** | ≤ 25 ms, amortised over a 12-file batch | — | w14 |
| **Added latency, 5 MB JPEG, p50 — Windows, `worker_reuse: Isolated`** | ≤ 60 ms per file, unamortised | — | w14 |
| **Added latency, 5 MB JPEG, p99** | ≤ 80 ms · Windows `Isolated` ≤ 140 ms | — | w14 |
| **Throughput, 200 MB TIFF stream** | ≤ 5% regression | — | w14 |
| **Receipt records a reduced profile** | correct mechanisms named | SR-11 | w14 |
| **Profile construction is sealed** | exactly one crate enables `feature = "attest"` | — | w14 |
| **The profile is read back, not requested** | request a mitigation known to be declined on the runner; the resulting profile must not claim it | **SR-11** | **w14** |
| Installer size | ≤ the week-0 measured budget | — | w15 |
| **Fuzz: one target per engine adapter** | no panic | SR-2 | w17, then per adapter |
| **Archive bombs** | zip bomb, nested archive stopped during extraction | SR-5 | **w24** |
| **Path-traversal through real extraction** | the same corpus, end to end | SR-13 | **w24** |
| **`mkv→mp4` stream copy** | bit-identical | — | w30 |
| Fuzz: receipt · recipe parsing | no panic | SR-19 | w33 |
| **GUI: strict CSP, no remote origin** | config lint | **SR-18** | **w35** |
| **GUI: zero outbound connections** | packaged app, full conversion | **SR-18** | **w35** |
| Contrast ratios, **as rendered component pairs** | ≥ 4.5:1 body, ≥ 3:1 non-text | — | w35 |
| **Shipped font covers every state glyph** | no fallback for `=` `≈` `⌇` `✦` `⛨` | — | w35 |
| **Prediction p99** | < 10 ms | — | w39 |
| Update chain | rejects unsigned, wrong-hash, downgrade | SR-7, SR-12 | w43 |
| **Revocation applies offline** | a cached list still disables a revoked engine | SR-12 | w43 |

† The two licence-table gates are live from week 1 and are **vacuous until week 15**, when the first native engine and its `engines.toml` row arrive. They are switched on early deliberately: the gate that fires the moment the first engine is added costs nothing to have waiting, and a gate added later is a gate someone has to remember.

**Eleven gates go live in week 4** (Class A metadata, Class B, in-process limits, budget, detection, no-overwrite, receipt completeness, receipt-not-optional, no-outbound, fuzz sniffer, plus the week-4 half of SR-5) on top of the **twenty-one** now live from weeks 1–2. v0.3 said "five" against a table containing thirteen.

**Six of those weeks-1–2 gates are new in v0.6, and every one replaces something that was assumed.** `depcheck` enforces D7, which `forbid(unsafe_code)` was wrongly credited with. The purity gate enforces [03 §4](03-ARCHITECTURE.md#4-functional-core-imperative-shell), which previously named no mechanism. The mutation gate audits the property tests, which nothing did. And the advisory and supply-chain gates cover vulnerabilities, which the licence gates never did.

> **The rule this table sets for itself — "a gate's week is a week its test can actually pass" — has now been broken in three consecutive revisions.** v0.4 broke it on SR-5, SR-11 and SR-13; v0.5 fixed those by staging and broke it on Class A. Before adding any row here, check that its subject exists in the commit list at or before that week.

---

## 5. File schema

```
openconvert/
├── Cargo.toml · rust-toolchain.toml · deny.toml
├── engines.toml            native engines: licence + linkage        ← CI gate
├── models.toml             model weights: licence + commercial + hash ← CI gate
├── LICENSE · NOTICE · README.md · SECURITY.md · CHANGELOG.md
├── .well-known/security.txt
├── .github/workflows/      ci · deny · lint · a11y · fuzz · bench · release · sign-* · nightly-hostile
├── docs/                   the design record + blog + threat model
├── xtask/                  the three source-scan lints, run in CI and locally
├── corpus/
│   ├── benign/             ordinary test files, in-repo
│   └── src/evil.rs         traversal · collision · ADS · device names · NFC/NFD
│                           — GENERATED IN-REPO so the gates run on fork PRs
├── fuzz/fuzz_targets/      route · predict · sniff · output_name · destination_name
│                           receipt · recipe · state · protocol · one per adapter
├── tokens/                 primitive + semantic design tokens (source of truth)
├── packaging/              installers, package-manager manifests, per-worker signing
├── site/                   docs + marketing site
│
├── crates/
│   ├── openconvert-core/           ← pure. No I/O, no clock, no env, no unsafe.
│   │   ├── data/formats.toml       the format table, incl. magic signatures
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── format.rs           FormatId, MediaKind, table lookup
│   │       ├── facts.rs            Sniff, Properties (enum/kind), FileFacts, InputHandle
│   │       ├── target.rs           Target
│   │       ├── policy.rs           defaults → user → flags; floor, max_auto_class, on_conflict
│   │       ├── limits.rs           Limits (per step) + Budget (per job)
│   │       ├── isolation.rs        Isolation, SandboxProfile (SEALED), Strength, IsolationFloor
│   │       ├── environment.rs      Environment — the machine, the engines, the tables
│   │       ├── plan.rs             PlanRequest, Plan, Step{class,limits,isolation}, Warning, version
│   │       ├── route.rs            Route, Requirement, RouteTable, route()
│   │       └── predict.rs          the five signals, scored — pure
│   │
│   ├── openconvert-sandbox/        ← THE BOUNDARY. #![forbid(unsafe_code)]. ~800 lines.
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── broker.rs           BrokeredPath · BrokeredOutput · OutputName · DestinationName
│   │       ├── engine_bin.rs       EngineBin — the closed set of executable programs
│   │       ├── argv.rs             Argv — no command strings exist
│   │       ├── worker.rs           Worker — one session per file; owns the job dir
│   │       ├── protocol.rs         the host↔worker wire format
│   │       └── sweep.rs            startup reaping of orphaned job dirs
│   │
│   ├── openconvert-os/             ← the syscalls. The only host-side `unsafe`.
│   │   └── src/                    Holds no invariants — which is the point.
│   │       ├── available.rs        probe the machine -> SandboxProfile (the only `attest` caller)
│   │       └── linux.rs · macos.rs · windows.rs
│   │
│   ├── openconvert-run/            ← the shell. #![forbid(unsafe_code)].
│   │   └── src/
│   │       ├── lib.rs              execute(plan, ctx, cancel) -> Outcome
│   │       ├── write.rs            create_new — the ONLY file-creation API in the crate
│   │       ├── receipt.rs          Receipt from an execution log
│   │       ├── detect/
│   │       │   ├── mod.rs          detect(src, hint) -> FileFacts   (opens once)
│   │       │   ├── sniff.rs        table-driven magic, Read + Seek
│   │       │   ├── polyglot.rs     multi-signature detection
│   │       │   ├── probe.rs        properties — in-process for Rust, brokered for C
│   │       │   └── provenance.rs   mark-of-the-web / quarantine · copy-before-route
│   │       ├── engines/
│   │       │   ├── mod.rs          registry + Engine trait (w15, with two impls)
│   │       │   ├── image_rs.rs     in-process images, limits before decode
│   │       │   ├── images.rs · pdf.rs · archive.rs · audio.rs   ← protocol clients
│   │       │   └── containers.rs   mp4/mkv remux — the stream-copy path
│   │       ├── ir/avgraph.rs       codec-preserving stream model
│   │       └── state/              ← everything that persists (03 §12)
│   │           ├── paths.rs        the per-user state directory
│   │           ├── config.rs · history.rs · journal.rs · recipe.rs · quarantine.rs
│   │
│   └── openconvert/                ← the CLI binary
│       └── src/
│           ├── main.rs
│           ├── cmd/               convert · inspect · plan · routes · formats
│           │                      verify · doctor · config · batch · recipe
│           └── output.rs          human + --json
│
├── engines/                      ← OUR WORKER BINARIES. FFI `unsafe` lives here,
│   ├── openconvert-worker/           on the far side of the boundary.
│   │   └── src/                    protocol · openat discipline · confinement entry
│   ├── oc-images/                  libvips · libheif · libavif · libjxl · libraw · lcms2 · resvg
│   ├── oc-pdf/                     pdfium · qpdf
│   ├── oc-archive/                 libarchive
│   └── oc-audio/                   LAME · libopus · libFLAC
│
└── apps/desktop/                 ← weeks 34–38. Tauri v2 + Svelte 5, links the library crates.
    ├── src-tauri/
    │   ├── src/main.rs
    │   └── tauri.conf.json       strict csp, no remote origin  ← CI gate
    └── src/
        ├── lib/
        │   ├── ipc.ts            typed client generated from core types
        │   ├── tokens/           generated CSS custom properties
        │   ├── components/       Chip · SuggestionCard · AlternateList · PlanTable
        │   │                     ProgressList · ReceiptView · GroupedDrop · ProfileBadge
        │   ├── stores/           prediction · job · undo · settings
        │   └── keys.ts · theme.ts
        └── routes/
            ├── +page.svelte      Idle → Prediction → Progress → Result
            └── settings/         general · safety · privacy · predictions
```

**No daemon. No IPC protocol between our own processes on the host. No module registry. No server, and no shell extension, in v1.** The CLI and the Tauri backend are two processes that each link the four library crates directly. The worker pipe is private to one job and is not a public protocol — see [03 §5.2](03-ARCHITECTURE.md#52-the-hostworker-protocol).

**`openconvert-server`** is deferred with an observable trigger: *the first design-partner contract, or 25 issues tagged `server`.* The CLI is kept at full `--json` parity precisely so the server is a thin wrapper over `route()` and `execute()` when that day comes.

---

## 6. Where things live

| Looking for… | It's here |
|---|---|
| The decision-making of the entire app | `core/src/` — eleven files |
| Why a route was chosen | `core/src/route.rs` → the `Requirement` that failed |
| Stream copy vs re-encode | `core/src/route.rs` (the table) + `run/src/ir/avgraph.rs` |
| Why JPEG can't hold alpha | `core/src/route.rs` as a `Requirement` — **never** in prediction |
| **The security boundary** | **`crates/openconvert-sandbox/` — the whole crate, and it contains no `unsafe`** |
| **Where `unsafe` is allowed at all** | `crates/openconvert-os/` (syscalls) and `engines/*` (FFI). Nowhere else, ever |
| **What guarantees this step runs sandboxed** | **`core/src/plan.rs` → `Step::isolation`, set by `route()` from the registry's declared minimum. Property-tested from week 2** |
| Opaque input names, the no-shell guarantee | `sandbox/src/broker.rs` + `sandbox/src/engine_bin.rs` |
| **Why an archive can't write outside its folder** | **`sandbox/src/broker.rs` → `OutputName::parse`** |
| **Why an archive can't destroy a file you already had** | **`run/src/write.rs` — `create_new` is the only creation API, and a lint proves it** |
| **Why the bytes converted are the bytes routed on** | `run/src/detect/mod.rs` → the handle owned by `FileFacts` |
| What isolation actually engaged on this machine | `os/src/available.rs` → `SandboxProfile`, rendered from `core/src/isolation.rs` |
| Why a job left no temp files behind | `sandbox/src/worker.rs` + `sandbox/src/sweep.rs` |
| Per-OS isolation | `os/src/{linux,macos,windows}.rs` |
| **Anything that persists between runs** | **`run/src/state/` — and [03 §12](03-ARCHITECTURE.md#12-state-on-disk) says why none of it can escalate** |
| **What happens when something fails** | **[03 §13](03-ARCHITECTURE.md#13-failure-is-designed) — every failure, its message, its test** |
| Receipt contents | `run/src/receipt.rs` ← `core/src/plan.rs` |
| Which colour means what | `tokens/semantic.*.json` — components never hard-code |
| The lossless guarantee, enforced | `tests/reproducibility.rs` |
| The architectural bet, asserted | `core/tests/properties.rs` |
| **Which threat a control answers to** | [09 §5](09-THREAT-MODEL.md#5-requirements-to-tests) — requirement → test → week |
| **What extension actually costs** | [03 §15.1](03-ARCHITECTURE.md#151-what-extension-actually-costs) — with real counts, including the expensive rows |
| Anything not in this table | **It is not designed yet. Say so, in writing, rather than inferring it.** |

---

## 7. Parallel non-engineering tracks

| Weeks | Track |
|---|---|
| **0** | Domains, registries, trademark. **Measure the packaged worker size, the confined spawn cost, and the protocol round-trip.** |
| **1–14** | **Build in public** on r/selfhosted — substantive progress, not marketing |
| **5–14** | Write the sandboxing deep-dive *as the work happens*, while detail is fresh. Ten weeks of it is a better article than one. |
| **15–26** | **GEO baseline** — run the 30 target prompts through the AI engines *before OpenConvert exists*, and log who's cited. Without a before, the after is unmeasurable. |
| **20–34** | Docs site · README as a product page with a GIF above the fold · publish the threat model |
| **30–40** | ~30 comparison pages · **the converter audit**, held for week 2 post-launch as a second spike |
| **38–44** | Line up 3–5 reviewers in the self-hosted niche |
| **41–44** | **LGPL compliance artifacts** — notice text, licence copies, and a source mirror for libvips, libheif, libraw and LAME. Not optional, and not free ([02-FEATURES §"Packaging model"](02-FEATURES.md#packaging-model)) |

---

## 8. Weekly rhythm

| | |
|---|---|
| **Monday** | Write the week's exit criterion in one sentence. If you can't, the week isn't planned. Plan next week's commits. |
| **Daily** | Trunk-based; CI green before merge; no long-lived branches |
| **Wednesday** | Check the gates ([§4](#4-the-ci-gates)) — sandbox latency and prediction p99 are *product* requirements, not optimisations |
| **Friday** | Did the exit criterion happen? One line if not. Ship a progress post if there's something real. |
| **Monthly** | Re-read the [non-goals list](02-FEATURES.md#14-explicitly-excluded), the [deferred list](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires), and **[09 §5](09-THREAT-MODEL.md#5-requirements-to-tests) — is every `SR-*` still gated, at a week it can still pass?** Scope creep enters through good ideas; security regressions enter through simplification. |

**On the two engineers:** weeks 7–13 parallelise cleanly (one OS each, then swap for review); weeks 18–26 parallelise by worker crate; weeks 1–4 and 34–38 do not, and are planned as such. A solo build of this plan is roughly 70 weeks, and if that is the reality the honest response is to cut the format catalogue, not the boundary.

---

## 9. Early warning signals

| Signal | Response |
|---|---|
| **A requirement in [09 §5](09-THREAT-MODEL.md#5-requirements-to-tests) loses its test** | Stop. This is exactly how v0.3 lost its resource limits and v0.4 lost SR-2's carrier. Restore the test or delete the requirement deliberately, in writing. |
| **A gate is moved to a later week** | Ask whether it was ever passable at the old one. If it was not, that is the v0.3 pattern — a gate written as an intention. Record which. |
| **A simplification removes a control** | The control had a reason. Find the `SR-*`, decide explicitly, and record the decision. |
| Sandbox latency climbing toward the gate | Move more formats to the in-process path. **Never weaken the sandbox to pass its own gate.** |
| **A sixth library crate being proposed** | Is there *pain*, a security boundary, an `unsafe` boundary, or an external consumer? Those four only. |
| A trait with one implementation appears | That's a guess unless the second impl is already in the build sequence. |
| The route table can't express something | Now A\* might earn its place. Not before. |
| Someone proposes the module system | Trigger: the third external PR adding a format adapter, or month 12. Count them; don't wait for a feeling. |
| Someone proposes the server | Trigger: a design-partner contract, or 25 issues tagged `server`. |
| Someone proposes AI before v1 ships | Zero-AI v1 already covers most measured demand |
| A daemon is proposed | Named trigger: watch folders. A per-job worker is not a daemon. |
| Format coverage becoming the focus | **Twenty formats done correctly beats a thousand done naively** |
| A sixth prediction signal proposed | Does it move top-1 acceptance on the frozen benchmark set? Measure or drop. |
| **An engine linked instead of subprocessed** | That's a licence boundary *and* a security boundary. Revert — and note that **`#![forbid(unsafe_code)]` does NOT catch this**: it is a lint on the `unsafe` keyword in a crate's own source, and the `unsafe` lives in the `-sys` dependency. **The `depcheck` gate is what fails the build** (S2). |
| **A function appears that takes a command string** | It cannot exist — `EngineBin` is closed. If one appears, someone widened the enum; that is the diff to revert. |
| **A path reaches a write without passing `OutputName`** | Revert. This is A12. |
| **A truncating open appears** | Revert. This is A13, and it is the bug the v0.4 design shipped with. The lint should have caught it; find out why it didn't. |
| **`feature = "attest"` appears in a second `Cargo.toml`** | Someone just gave a non-boundary crate the ability to fabricate a sandbox profile. This is a one-line diff and the gate catches it — treat it as a boundary change, not a build tweak. |
| **A ninth noun is proposed** | Stop and think, then write the thinking down — as [03 §3](03-ARCHITECTURE.md#3-the-whole-system-in-one-line) does for the eighth. The rule is not "never"; it is "justify it in public." |

---

## The one-paragraph version

Secure the name this week and measure three numbers — packaged worker size, confined spawn cost, protocol round-trip — because all three are gates you'll be held to later, and because the number the previous plan wanted to measure was for a build this architecture forbids. Build the pure core first: eleven files, no I/O, **class, limits and isolation all required on every step**, prediction scored as a pure function, fully tested by the end of week two. Make it real in weeks three and four with in-process image conversion that refuses a pixel bomb, **never overwrites a file you already had**, reads each input exactly once, writes a receipt, and prints its own route table: **that is already shippable software.** Then spend **ten weeks** on the only genuine architectural risk — sandboxing three operating systems, Windows first, with the Tauri sidecar question answered in week nine while the answer can still change anything. Then **three weeks building one worker binary properly** and nine repeating it, containers for four, CLI and state for three, the GUI for five, prediction for two, and ship in week forty-four. **Five crates, four worker binaries, two isolation variants plus a sealed profile, a floor whose network requirement has no setting, eight nouns, five prediction signals, no daemon, no modules, no server, no shell extension, no AI.** Everything deferred has a trigger someone can count; every threat has a requirement; every requirement has a test that fails the build in a week that test can pass.
