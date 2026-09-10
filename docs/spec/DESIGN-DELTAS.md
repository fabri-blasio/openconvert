# Design deltas — what 41 measurements say the record must change

**Status: APPLIED 2026-08-14.** All 28 deltas are in the record; `03`, `08` and `09` are at v0.6. Kept as the audit trail — every claim in the record that changed, and the spike that changed it.
**Evidence:** the private validation record · **Plan:** [PLAN-FORWARD.md](PLAN-FORWARD.md)

---

## What this is, and what it is not

The design record has been revised three times and scored 59–65/100 by four independent reviews. The reason the number never moved is that **each revision fixed the defects the last review named, and the fixes generated the next round's defects** — five of the current top findings were introduced by v0.5's own remedies.

So this is **not another revision round.** It is a single pass driven entirely by measurement. Every delta below cites the spike that produced it. Nothing here comes from reasoning about the design; where reasoning and measurement disagreed, the measurement won — including **four times where the measurement refuted something this project had already published.**

**Rule for applying this document:** if a delta has no spike citation, it does not belong here. Add it to a separate list and require evidence first.

### How to apply

Work **document by document** using the index in §2, not severity order — every delta names its exact section, and several touch the same paragraph. Apply, then delete the delta from this file (or mark it `APPLIED`). When the file is empty, the record matches what the machine says.

### What is deliberately NOT here

- Anything about macOS. Zero measurements exist; the private macOS validation runbook is out for execution. **Do not pre-emptively edit `03 §9.3`.**
- The Landlock-on-hardened-Debian and arm64 questions, for the same reason.
- Style, wording and structural preferences. Those are not deltas.

---

## 1. Summary, by severity

| # | Delta | Document | Spike | Kind |
|---|---|---|---|---|
| **D1** | `InputHandle` leaves `openconvert-core` | `03 §5.1, §11.3` · `08 c4, §5` | S1 | **False claim** |
| **D2** | `forbid(unsafe_code)` does not prevent linking — delete the claim ×4, add a real gate | `README` · `03 §1, §5` · `08 §9` | S2 | **False claim** |
| **D3** | `Limits` gets values | `03 §10.1` | S23 | **Missing** |
| **D4** | `expansion_ratio` cannot stop a bomb — rewrite the walkthrough | `09 §7` | S23 | **False claim** |
| **D5** | The job directory needs a container-SID ACL | `03 §5.2, §8.4, §12` | S9 | **Missing** |
| **D6** | A2 is false without `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` | `03 §5.2, §8.1` · `09 §6` | S3, S9 | **False claim** |
| **D7** | `-sandbox` cannot own the spawn | `03 §5, §5.1` | S3, S9, S12c | **Structural** |
| **D8** | `D11` becomes one worker per **batch** | `03 §16, §8.4` · `09 §8` | S16, S9×S16 | **Structural** |
| **D9** | The profile is built by **read-back**, not by request | `03 §7.1, §9` · `09 SR-11` | S14, S17, S17b, S21 | **Structural** |
| **D10** | The destructive-API lint widens from 1 API to 11 | `03 §7.4` · `08 §4` · `09 SR-15` | S4, S25 | **Incomplete** |
| **D11** | The NFC/NFD rule rejects ordinary filenames | `03 §8.2` | S5, S13, S24 | **False claim** |
| **D12** | Our own copy strips mark-of-the-web | `03 §11.3` · `09 §8` | S26 | **Conflict** |
| **D13** | The wire/domain split and protocol bounds | `03 §5.2, §7.1` | S6 | **Missing** |
| **D14** | `openat(fd, "..")` escapes on Linux — the platforms are opposite | `03 §5.2, §9.2` | S12c/d | **Missing** |
| **D15** | The latency gate must be per-platform | `03 §17` · `08 §4` | S7, S8, S12a | **False claim** |
| **D16** | A Job Object cap kills the protocol; an IOCP fixes it | `03 §10.1, §13` | S14, S18 | **Missing** |
| **D17** | Engines are *linked into* subprocesses — the LGPL conclusion is conditional | `02` · `09 §9` | (review F16) | **Unsupported** |
| **D18** | Target matrix, MSRV, Landlock ABI negotiation | `08 §1` | S10b, S12b, S15 | **Missing** |
| **D19** | SR-1's test asserts the wrong thing | `09 SR-1` | S9 | **False test** |
| **D20** | Reserved names: right rule, wrong justification | `03 §8.2, §13` | S13 | **Justification** |
| **D21** | Purity is enforced by clippy, not by a wasm32 build | `03 §4` · `08 §1, §4` | S1 | **Missing** |
| **D22** | Add a mutation-testing gate | `08 §4` | S29, S30 | **Missing** |
| **D23** | `O_EXCL` weakness — **downgrade our own finding** | (review F11) · `09 §8` | S27 | **Our error** |
| **D24** | Bound the job-directory path length | `03 §8.2, §13` | S28 | **Missing** |
| **D25** | Scope the reproducible-build claim | `09 §9` · `04 §10.2` | S31 | **Overclaim** |
| **D26** | `cargo-audit` / `cargo-vet` configuration | `09 §9` | S33, S33b | **Refinement** |
| **D27** | Real packaging numbers | `01 §5.8` · `02` | S12e, S35 | **Refinement** |
| **D28** | The Class A gate at w4 has no Class A operation | `08 §4, c23` | (review) | **Impossible gate** |

---

## 2. Index by document

| Document | Deltas |
|---|---|
| `README.md` | D2 |
| `01-VISION.md` | D27 |
| `02-FEATURES.md` | D17, D27 |
| **`03-ARCHITECTURE.md`** | **D1, D2, D3, D5, D6, D7, D8, D9, D10, D11, D12, D13, D14, D15, D16, D20, D21, D24** |
| `04-GROWTH.md` | D25 |
| **`08-EXECUTION-PLAN.md`** | **D1, D2, D10, D15, D18, D21, D22, D28** |
| **`09-THREAT-MODEL.md`** | **D4, D6, D8, D9, D10, D12, D17, D19, D23, D25, D26** |

---

## 3. The deltas

### D1 — `InputHandle` leaves `openconvert-core`

**Sections:** `03 §5.1`, `03 §11.3`, `08` commit 4, `08 §5`
**Evidence:** S1

**Current.** `03 §5.1` lists `facts.rs  FileFacts, Sniff, Properties, InputHandle` under `openconvert-core`, and `§11.3` defines `pub struct FileFacts { handle: InputHandle, … }` — *"owned, opened once, read-only, share-deny-write on Windows"*, i.e. a live OS resource.

**Against three claims in the same record:** `03 §4` says core I/O is **"None"** and its tests are *"no fixtures, no files"*; `03 §15.1` says core *"compiles to `wasm32` unchanged"*.

**Change.** `FileFacts` in core carries `content_id`, `sniff`, `props`, `provenance` and an opaque `InputToken(u64)`. `openconvert-run` owns `HandleTable: InputToken → File` and performs every read. `execute()` carries the `(FileFacts, InputHandle)` pair.

**I13/SR-16 survive** — one open, all reads through it. The guarantee comes from the pairing, not the field placement.

**If not applied:** the seven week-2 property tests cannot construct their inputs, because `detect()` — the only thing permitted to build a `FileFacts` — does not exist until week 3.

---

### D2 — `#![forbid(unsafe_code)]` does not prevent linking a C library

**Sections:** `README` line 57 · `03 §1` item 1 · `03 §5` · `08` "What changed from v0.3" · `08 §9`
**Evidence:** S2

**Current, in four places:** *"`-run` forbids `unsafe`, **so it cannot link libvips or pdfium**"*, and in the early-warning table: *"`#![forbid(unsafe_code)]` in `-run` makes it **a compile error, not a review item**."*

**Measured: false.** A crate with `#![forbid(unsafe_code)]` and no `unsafe` token compiled, linked and called into C through a safe wrapper. `forbid` is a crate-scoped lint on the keyword in *that crate's own source*; the `unsafe` lives in the `-sys` dependency.

**Change.**
1. Delete the causal *"so it cannot link"* in all four places. Replace with: *"`-run` forbids the `unsafe` keyword in its own source. That is an audit property, not a linkage guarantee — linkage is enforced by the `depcheck` gate."*
2. Add to `08 §4`, live from **w1**:

   | Gate | Threshold | Requirement | Live from |
   |---|---|---|---|
   | **Lint: `-run` links no native library** | no package in `-run`'s resolved **normal** dependency graph has a non-null `links` manifest key | **D7** | **w1** |

3. `08 §9`'s "An engine linked instead of subprocessed" row: replace *"makes it a compile error"* with *"the `depcheck` gate fails the build; `forbid(unsafe_code)` does **not** catch this."*

**Note.** `links` is the authoritative signal, not the `-sys` naming convention — `windows-sys` and `linux-raw-sys` link nothing. Follow **normal** dependency edges only; dev-dependencies produced false positives in testing.

**Why it matters twice:** D7 (the engine-artifact model) rests on this, and so does the LGPL boundary in `02` — see **D17**.

---

### D3 — `Limits` gets values

**Section:** `03 §10.1`
**Evidence:** S23

**Current.** Ten fields, and **the entire record specifies not one value.** The only number anywhere is `120 s` inside an example error string.

**Change.** Add a defaults table with the derivation, measured against libarchive 3.8.5:

| Field | Default | Basis |
|---|---|---|
| `archive_depth` | **32** | Legitimate content topped out at 8; the bomb was 41 |
| `archive_entries` | **100_000** | The bomb had 40,000 — and so does `01 §7`'s archivist workload. Cannot be tight |
| `expansion_ratio` | **≥ 10_000, tripwire only** | **Cannot be a primary control** — see D4 |
| `archive_total_bytes` | derived from `Budget` | With `Budget`, this is what actually bounds a bomb |
| `wall_time` | `base + k · input_bytes`, per `MediaKind` | A 4-hour remux and a 5 MB JPEG cannot share a constant |
| `memory_bytes`, `decode_pixels`, `output_bytes`, `temp_bytes`, `cpu_time` | **state a value and its rationale** | Still unspecified |

**If not applied:** someone invents them at the keyboard in week 1, and the w4 gate asserts a pixel bomb errors without ever asserting the threshold is right.

---

### D4 — `expansion_ratio` cannot separate a bomb from real data

**Section:** `09 §7` step 7
**Evidence:** S23

**Current.** *"Bomb attempt: 4 KB → 1 GB, nesting depth 40 | Running totals trip `expansion_ratio`, `archive_depth`, `archive_total_bytes` **during** extraction."* `expansion_ratio` is named first.

**Measured:**

| Archive | Ratio |
|---|---|
| 200 MB zeroed disk image — **legitimate** | **1029×** |
| 512 MB zip bomb | **1028×** |

**They are indistinguishable on that axis.** Any threshold catching the bomb also refuses compressed disk images, VM images, database dumps and sparse files.

**Change.** Rewrite step 7 to lead with `archive_depth` and `archive_total_bytes`, and state explicitly that `expansion_ratio` is a heuristic tripwire, not a discriminator. Add to `09 §8`: *"A highly compressible legitimate archive is indistinguishable from a bomb by ratio alone; we bound absolute size and depth instead."*

**Second site, same problem.** `03 §10.1`'s `InProcess` row also leans on it — *"`output_bytes` and `expansion_ratio` go through a counting writer that errors at the cap."* A 4 KB PNG of flat colour legitimately decodes to hundreds of megabytes, so the ratio is no more discriminating here. **`decode_pixels`, checked from the header before allocation, is the control that works** — and the same paragraph already says so. Demote `expansion_ratio` in that row too.

**Also record:** libarchive imposes **nothing** of its own — 512 MB extracted from a 522 KB archive with no error. Every limit comes from us.

---

### D5 — The job directory needs a container-SID ACL

**Sections:** `03 §5.2`, `03 §8.4`, `03 §12`
**Evidence:** S9

**Current.** The worker receives a pre-opened job-directory handle and writes through it. Nothing more.

**Measured.** Inside an AppContainer that is **necessary and not sufficient** — `NtCreateFile` performs a fresh access check for the *new file* against the child's token, and returns `STATUS_ACCESS_DENIED` (`0xC0000022`). With the job dir ACL'd for the container SID, it succeeds.

**Change.** Add to `§5.2` and to the `§8.4` worker-start sequence: *"On Windows the job directory is ACL'd for the AppContainer SID before the worker starts. The handed handle alone is not sufficient."* Add the failure to `§13` with a message. Add to `§12`: the startup sweep must also handle job dirs carrying container ACLs.

---

### D6 — "holds no descriptor it was not handed" is false as implemented

**Sections:** `03 §5.2`, `03 §8.1`, `09 §6` (A2)
**Evidence:** S3, S9

**Current.** `09 §6` A2: *"a compromised engine … **holds no descriptor it was not handed**."*

**Measured.** `std::process::Command` sets `bInheritHandles = TRUE`, and Windows then inherits **every inheritable handle in the parent**. The child enumerated **64 handles it was never given and wrote through two**, including one the parent deliberately withheld. With `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: unintended writes **0**.

**Change.** State that the guarantee requires an explicit inherit list, that `std::process::Command` cannot express one, and that **all host-side handles are opened non-inheritable by default** with inheritance granted per spawn.

---

### D7 — `openconvert-sandbox` cannot own the spawn

**Sections:** `03 §5`, `03 §5.1`
**Evidence:** S3, S9, S12c — **two platforms, two independent routes**

**Current.** `Worker` and the spawn live in `openconvert-sandbox`, which is `#![forbid(unsafe_code)]` and described as *"pure logic over `std::fs` and `std::process`"*.

**Measured.** Passing pre-opened descriptors requires `unsafe` on **both** platforms: `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` via raw `CreateProcessW` on Windows; a `pre_exec` hook on Linux, because Rust opens everything `CLOEXEC`.

**Change.**
1. Move **all** process creation into `openconvert-os`, behind one typed entry point — `spawn(EngineBin, Argv, Handles, Confinement) -> Worker` — that only `-sandbox` calls.
2. Restate what `-sandbox` guarantees: it holds the invariant *types*, it contains no `unsafe`, and it does not itself execute anything.
3. **Delete the address-space justification.** `03 §5` argues the split is needed because *"a memory-safety bug in a syscall wrapper **sharing its address space** and its crate can defeat every one of them"* — but `-os` is linked into the same process either way. The split buys audit focus and blast-radius clarity, which is reason enough. Say that instead.

---

### D8 — `D11` becomes one worker per batch

**Sections:** `03 §16` D11, `03 §8.4`, `09 §8`
**Evidence:** S16, S9×S16

**Current.** *"One process per file, for the duration of that file."*

**Measured.** Windows spawn is 40.15 ms against a 25 ms budget, of which the sandbox is 4.4 ms. A warm worker serving 12 files costs **2.19 ms/file** — 17.7×, and 91% headroom. A capability can be **granted** to a running worker (`DuplicateHandle`) and **revoked** from it (`DUPLICATE_CLOSE_SOURCE` → `STATUS_INVALID_HANDLE`), **including inside an AppContainer**.

**Change.**
1. `D11` → *"One worker per batch, one job directory at a time. Capabilities are granted per job and revoked when the job ends."*
2. `§8.4` currently defines cancellation, cleanup ownership and crash containment **per worker per file**. Restate all three in terms of a **job**: a crash now kills the batch, and *"three crashes on one format"* needs a new subject.
3. **Add to `09 §8`:** *"Revocation removes the handle, not the memory. A compromised engine can retain bytes from one file and write them into the output of another **within the same batch**. One process per file does not have this residual; we accept it in exchange for the latency budget."*

**This is a risk-appetite decision, not a measurement.** The measurement says it *can* be done safely; whether it *should* is the reader's call, and `09 §8` is where it gets recorded either way.

---

### D9 — The profile is built by read-back, not by request

**Sections:** `03 §7.1` (I11), `03 §9`, `09 SR-11`
**Evidence:** S14, S17, S17b, S20b, S21

**Current.** `I11` seals `SandboxProfile` so only the OS prober may construct one. `SR-11` requires the receipt to record *"the profile that engaged"*.

**Measured, and this one corrected itself mid-investigation:**

```
              no mitigation requested    ACG requested
  plain               0                        0
  AppContainer        1                        1
```

**ACG is a property of the AppContainer, not of the request.** An earlier write-up of this project claimed *"ACG is requested successfully and silently declines"* — that was measured on a **plain process**, which is not what ships. **CIG, re-tested the same way, does not engage in any of the four cells.**

**Change.**
1. Corrected statement for `03 §9.2`: **one** of the four `Full`-row mitigations (**CIG**) does not engage; ACG arrives via the container rather than the flag.
2. **New requirement.** The profile is constructed **in the child, after it starts**, from read-back queries — `GetProcessMitigationPolicy` on Windows, the applied Landlock ruleset on Linux — and returned over the protocol as a `wire::` type validated by D13's constructor.
3. **Requested-vs-engaged divergence is a first-class event**: it lowers the reported tier, appears in the plan preview, and is recorded in the receipt.
4. Add a test: request a mitigation known to be declined on the test machine and assert the resulting profile does **not** claim it.

**I11 guards against a dishonest constructor. It does not guard against an honest one recording a request the OS ignored** — and the measurement shows that error runs in **both** directions.

---

### D10 — The destructive-API lint widens from one API to eleven

**Sections:** `03 §7.4`, `08 §4`, `09 SR-15`
**Evidence:** S4, S25

**Current.** *"No truncating-open API exists in `-sandbox` or `-run`; **source scan bans `File::create`** outside two audited call sites."*

**Measured — five destructive routes on Windows, seven on Linux:**

| API | Windows | Linux |
|---|---|---|
| `File::create` | destroys | destroys |
| `File::create_new` | **safe** | **safe** |
| `OpenOptions` truncate | destroys | destroys |
| `fs::write` · `fs::copy` · `fs::rename` | destroys | destroys |
| `set_len(0)` | destroys | destroys |
| `fs::remove_file` | — | destroys |
| `symlink` after `unlink` | — | **destroys (POSIX-only)** |

**Change.** Rename the gate to **"no destructive filesystem API"** and enumerate: `File::create`, `OpenOptions` with `create`/`truncate`, `fs::write`, `fs::copy`, `fs::rename`, `fs::remove_file`, `fs::remove_dir_all`, `File::set_len`, `std::os::unix::fs::symlink`, plus `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` and `SetFileInformationByHandle(FileRenameInfo)`.

> **`set_len(0)` is the sharpest:** it truncates through a handle opened with plain `write(true)`. There is no truncating *open* for any scan to find.
>
> **`fs::rename` is the likeliest:** write-to-temp-then-rename is *the* idiomatic atomic write, and `03 §13`'s "partial output removed" rows push an implementer straight toward it.

**Pending:** macOS `copyfile(3)` and `clonefile(2)` — handoff T4. Add when measured.

---

### D11 — The NFC/NFD rule rejects ordinary filenames

**Section:** `03 §8.2`
**Evidence:** S5, S13, S24

**Current, per-name:** reject *"> 255 bytes, or a component that normalises to a different name under NFC/NFD"*.

**Measured.** As written it rejects **5 of 10** ordinary filenames — `café.jpg`, `Müller-Bericht.pdf`, Greek, Cyrillic, French. Every archive produced on macOS (APFS stores NFD) fails extraction with a security error.

**And the premise is wrong:** **NTFS does not fold NFC and NFD** — both files exist side by side. APFS does. ext4 does not.

**Change.**
1. Split the row. Keep `> 255 bytes` as a per-name rejection.
2. Replace the normalisation clause with: *"Within one extraction, two entries whose NFC forms are equal is a **collision**, resolved by `Policy::on_conflict`. Names are preserved byte-for-byte; normalisation is used only as a comparison key."*
3. Add **`CONIN$`** and **`CONOUT$`** to the reserved-device list.
4. Add a note: the same archive extracts to a **different set of files** on NTFS, APFS and ext4. That is a cross-platform determinism problem in a product whose second pillar is telling you exactly what happened.

---

### D12 — Our own copy strips mark-of-the-web

**Sections:** `03 §11.3`, `09 §8`
**Evidence:** S26

**Current.** `03 §9.2` raises the floor to `Full` for files carrying mark-of-the-web. `03 §11.3` **copies** untrusted-provenance inputs into the job dir before routing.

**Measured.** `fs::copy` and `fs::rename` **preserve** `Zone.Identifier`. **`read` + `create_new` + `write` strips it** — and that is the creation path I12 mandates, because `create_new` is *"the only creation API"*.

> **The copy taken *because* a file is untrusted destroys the evidence that it is untrusted.**

**Change.** In `§11.3`, state one of: (a) the copy explicitly duplicates the `Zone.Identifier` stream, or (b) **provenance is captured once at detect time, recorded in `FileFacts`, and never re-derived from the copy.** (b) is cheaper and the type already exists. Add to `09 §8`: *"Our own copy-before-route does not preserve the zone identifier; provenance is captured before the copy and is not re-checked afterwards."*

---

### D13 — The wire/domain split and protocol bounds

**Sections:** `03 §5.2`, `03 §7.1`
**Evidence:** S6

**Current.** *"Length-prefixed `postcard`"*, with no cap, and `Properties` crossing from a confined engine straight into `route()`.

**Change.**
1. **Every value crossing the protocol arrives as a `wire::` type** that is not the domain type, converted through a fallible clamping constructor. `Deserialize` appears **only** in `wire.rs` — gate it with a lint over `-core` and `-sandbox`.
2. **`Policy` carries a `LimitCeiling`; `route()` clamps.** Engine-supplied facts may only **narrow** limits, never widen them. *(Measured working: a compromised engine reporting `width = 4_000_000_000` is refused at the boundary.)*
3. **Host-side frame bounds** — `max_frame_bytes`, `max_messages_per_step` — checked **before allocation**. An attacker-chosen length prefix is a *correct* decode for an unbounded reader, so fuzzing will never find it; only a bound will.
4. `OutputName` and `DestinationName` cross the wire as `String` and are re-parsed, or implement `Deserialize` manually via `parse`. Never `#[derive(Deserialize)]`.

**Also record the measurement that makes this affordable:** round-trip is **348 µs p50** — `08 §1`'s sub-millisecond criterion is met.

---

### D14 — The platforms are safe in opposite places

**Sections:** `03 §5.2`, `03 §9.2`
**Evidence:** S12c/d, S3

**Measured:**

| | Windows | Linux |
|---|---|---|
| `..` through the job-dir handle/fd | **BLOCKED** by the NT object manager (`0xC0000033`) | **ESCAPES** — an ordinary component to `openat` |
| Descriptors not handed over | **64 visible, 2 writable** | **0** (`CLOEXEC` default) |

**Change.** `03 §5.2` says *"the `openat` discipline is defence in depth, not the guarantee."* That is **correct and now proven** — add that it is *more* than defence in depth on Linux, because with Landlock absent nothing else stops `..`. Consequences to state:

1. **`OutputName::parse`'s separator rejection is load-bearing on Linux and only defence-in-depth on Windows.**
2. **The Linux `Minimal` tier provides no filesystem containment at all** — which is the empirical justification for refusing it.
3. A worker can Landlock **itself** using only the fd it was handed (`landlock_add_rule` takes a `parent_fd`), so confinement composes with "no names at all" without the worker learning a path.

---

### D15 — The latency gate must be per-platform

**Sections:** `03 §17`, `08 §4`
**Evidence:** S7, S8, S12a

**Current.** One threshold — p50 ≤ 25 ms, p99 ≤ 80 ms — for three platforms.

**Measured:** Windows **40.15 ms** (161% of budget); Linux **1.96 ms** (7.9%). A **20× gap**, and signing does not help — a Microsoft-signed system binary costs 15.0 ms to create against our 15.8 ms.

**Change.** Either per-platform thresholds, or state the threshold on the **amortised per-batch** basis D8 creates (2.19 ms/file measured). Keep `03 §17`'s response rule verbatim — *"move more formats onto the in-process path; never weaken the sandbox"* — it is right, and now for a measured reason: **the sandbox is 4.4 ms of the 40.**

---

### D16 — A Job Object cap kills the protocol; an IOCP fixes it

**Sections:** `03 §10.1`, `03 §13`
**Evidence:** S14, S18, S22

**Measured.** A Job Object memory cap works — a capped worker dies where an uncapped one survives. But it **aborts** (`0xC0000409`) **without sending `Failed{..}`**, so `03 §13`'s "name the file, the step and the engine" has nothing to work with. On Linux, `RLIMIT_AS` gives a **recoverable** `Err` and the worker can still report.

**And the fix is measured:** an IO completion port on the job delivers `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` **before** the exit code.

**Change.** Add `JOBOBJECT_ASSOCIATE_COMPLETION_PORT` to the `§8.4` worker-start sequence. Add `§13` rows per message type, so the host synthesises *"exceeded its 512 MB memory limit"* rather than surfacing an abort. Note the platform asymmetry in `§10.1`.

---

### D17 — The LGPL conclusion is conditional on a fact nobody recorded

**Sections:** `02 §11`, `09 §9`, `08 §4` (engine-licence-scan row, w1), `08` commit 2
**Evidence:** review finding F16 *(analysis, not a spike — flagged as such)*

**Current.** `02 §11` lists under **Not owed**: *"Any relinking obligation on the user — **the engines are separate executables**."* `09 §9` records linkage as *"subprocess vs linked, because that distinction **is** the GPL boundary"*, and `08 §4` gates it from w1.

**The problem.** The engines are neither. They are **libraries linked into a subprocess**: `oc-images` links libvips (LGPL-2.1+), libheif (**LGPL-3.0**) and libraw. The subprocess boundary is between `-run` and `oc-images`, not between `oc-images` and libheif — so the relinking obligation attaches to `oc-images`.

**Change.** Make `linkage` three-valued — `subprocess` / `linked-into-worker` / `linked-into-host` — and add `link_mode = static | dynamic`. Restate `02`'s "not owed" row as **conditional on dynamic linkage**, or accept the obligation and schedule the object-code artefacts alongside the source mirror.

> Marked **unverified**: this is analysis, not measurement. The settling observation is the static/dynamic decision, which is `08 §1` work — see D18.

---

### D18 — Target matrix, MSRV, Landlock ABI negotiation

**Section:** `08 §1` (Week 0)
**Evidence:** S10b, S12b, S15

**Current.** CPU architecture appears in **zero** live documents. The build matrix is "three platforms", six times.

**Measured.**
- A spike compiled for `x86_64-unknown-linux-gnu` and **failed** for `aarch64-unknown-linux-gnu` (`c_char` signedness).
- Every syscall number in a seccomp filter is per-architecture (`__NR_socket` 41 vs 198), as is `AUDIT_ARCH`.
- The kernel reported **Landlock ABI 3, not 4** — passing an unknown access bit is `EINVAL`.
- `docker buildx` **advertises** `linux/arm64` while being unable to **execute** it until binfmt is registered.

**Change.** Add to Week 0:
1. **The target matrix** — which OS × architecture combinations ship at 1.0, and which get confinement *assurance* rather than only a build.
2. **MSRV and the supported-OS floor**: Landlock ≥ 5.13 **plus runtime ABI negotiation**; a Windows build floor.
3. **The static/dynamic linkage decision** (D17).
4. A stated rule: **emulation validates compilation and logic, never confinement.** A CI matrix that builds for a target it never executes reports green while testing nothing.

---

### D19 — SR-1's test asserts the wrong thing

**Section:** `09 SR-1`
**Evidence:** S9

**Current.** The test is *"TCP, UDP, DNS and a unix socket from inside an engine"*.

**Measured.** Socket **creation** is still permitted inside an AppContainer — restriction is enforced by the Windows Filtering Platform at connect/send. And a connect to nothing returns `TimedOut` in every configuration, confined or not.

**Change.** The test must **connect to a listener the test itself is running** and assert reachability. Never assert on socket creation; never connect to nothing.

> Both mistakes were made in this project's first attempt at that spike, and both produced a "denial" that was not there.

---

### D20 — Reserved device names: right rule, wrong justification

**Sections:** `03 §8.2`, `03 §13`
**Evidence:** S13

**Measured.** Of ten reserved names, **only `NUL`** vanished into a device. `CON`, `PRN`, `AUX`, `COM1`, `LPT1`, `CONIN$`, `CONOUT$` all became **ordinary files** — because Rust's `std::fs` uses verbatim (`\\?\`) paths, which bypass Win32 device-name parsing.

**Change.** Keep the predicate; fix the justification. It protects **the C engines opening ordinary Win32 paths, and downstream tools the user opens the output with** — not our own Rust code. And add to `§13`: where a name *does* vanish, **the write succeeds, no error is raised, and the conversion reports success while the output is silently discarded.**

---

### D21 — Purity is enforced by clippy, not a wasm32 build

**Sections:** `03 §4`, `08 §1`, `08 §4`
**Evidence:** S1

**Current.** `03 §4` states core has no I/O, no clock, no env — with **no mechanism named**.

**Measured.** `std` is fully available on `wasm32-unknown-unknown`: a fixture holding `std::fs::File`, `SystemTime::now()` and `std::env::var` **compiled and produced an rlib**. `#![no_std]` rejects it (`cannot find module or crate 'std'`). A checked-in `clippy.toml` with `disallowed-types`/`disallowed-methods` catches all four leaks while keeping `Vec`, `String` and `HashMap`.

**Change.** Add to Week 0 and to the `08 §4` gate table at **w1**: a `clippy.toml` purity gate, plus a grep for `#[allow(clippy::disallowed` so the bypass is itself gated. Class it honestly — **Hard**, per `03 §7.4`'s own taxonomy. Keep a wasm32 build as a **portability** gate, labelled as such, since `04 §5`'s tool-page channel depends on it.

---

### D22 — Add a mutation-testing gate

**Sections:** `08 §4`, `09 §9` (first bullet)
**Evidence:** S29, S30

**Measured.** `cargo-mutants` on a parser that already passed **24 evil-corpus cases and 4000 property cases**: **10 of 30 mutants survived (64%)**. Among them a corpus test that **passes vacuously when its data is emptied**, and an **unreachable branch**. Six targeted tests took it to 100%.

Separately, `cargo-fuzz` ran **7,413,919 cases** against the same parser and found **nothing**.

> **Fuzzing tests the code. Mutation testing tests the tests.** `09 §9` opens with a ten-target fuzzing list and `08 §4` gates them from week 2. **Neither document mentions mutation testing** — so nothing in the record checks whether any of those tests would fail if the thing they cover broke.

**Change.** Add `cargo-mutants` on `openconvert-core` as a gate from **w2**, at a threshold (start 80%, raise it). Keep every fuzz gate — they answer different questions.

> Worth stating in `09 §9` explicitly, because the fuzzing list is long enough to look like sufficient coverage: **7.4M fuzz cases and 4000 property cases did not detect a test that passes when its own input is deleted. Thirty mutants did.**

---

### D23 — Downgrade our own `O_EXCL` finding

**Sections:** review finding F11, `09 §8`
**Evidence:** S27

**Current.** This project's own review rated *"`O_EXCL` atomicity is filesystem-dependent and degrades on network/sync volumes"* at **MEDIUM-HIGH**.

**Measured: refuted.** Across a 9p/drvfs boundary to NTFS — a genuinely non-local path — `O_EXCL` **held**, refusing with `EEXIST` exactly as on native ext4. `O_NOFOLLOW` held too.

**Change.** Downgrade F11 to **LOW, unverified**. **Do not** add a `09 §8` row claiming the guarantee is weak on network volumes — there is no evidence for it. Keep the real finding instead: `rename` replaced its target silently on **both** filesystems, so the threat to I12 is the APIs that never use `O_EXCL` (**D10**), not a weak `O_EXCL`.

---

### D24 — Bound the job-directory path length

**Sections:** `03 §8.2`, `03 §13`
**Evidence:** S28

**Measured.** An **829-character** path with a 255-char leaf was created without complaint, because Rust uses verbatim paths. `03 §8.2` bounds the **name** at 255 bytes and **nothing bounds the path**.

**Change.** Bound the job-directory path at creation time so that job-dir + longest legal member stays inside what a C engine can open; set `longPathAware` in the worker manifests. Add a `§13` row — otherwise the failure surfaces as an unexplained engine error on deep archives only.

---

### D25 — Scope the reproducible-build claim

**Sections:** `09 §9`, `04 §10.2`
**Evidence:** S31

**Current.** `04 §10.2` sells **"Attested builds: reproducible-build attestation, SBOM, FIPS packaging"** as a **paid** tier. `09 §9` backs it with one bullet — *"Reproducible builds so a third party can verify the shipped binary matches the source"* — **no week, no gate, no scope.**

**Measured.** Same source, `SOURCE_DATE_EPOCH` pinned, `--remap-path-prefix` applied: a rebuild **in the same directory** is byte-identical; a build **from a different root** is not.

**Change.** Scope the claim to **a fixed container image and a fixed build path** — achievable now, and what the attestation should promise. State that bit-identical output across arbitrary build roots needs more than the two flags everyone quotes, and that the **C engine builds are a separate problem** — pinned-and-hashed, not bit-reproducible. Give it a week and a gate, or stop selling it.

---

### D26 — There is no advisory scanner and no supply-chain review

**Section:** `09 §9`, `08 §4`
**Evidence:** S33, S33b

**Current.** `09 §9`'s "three licence surfaces" bullet names **`cargo-deny`** — a licence and ban-list gate. **No advisory scanner and no dependency-review tool appears anywhere in the record.** `cargo-deny advisories` is not configured, and neither `cargo-audit` nor `cargo-vet` is mentioned. This is an addition, not a correction.

**Measured.**
- `cargo-audit` found a real advisory (`atomic-polyfill` unmaintained) via `postcard → heapless` — **`postcard` being the crate `03 §5.2` names for the worker protocol.** But `cargo tree -i` shows nothing on the **host** target; it appears only under `--target all`, because audit reads `Cargo.lock`, which is target-independent.
- `cargo vet init` **grandfathers all 67 existing crates.** A green first run is green by construction.

**Change.** Add an advisory gate to `09 §9` and `08 §4` — `cargo-audit`, or `cargo-deny advisories` alongside the licence check that is already configured — with **`--target` filtering**, or an ignore list that records *why* each entry is safe. Add `cargo-vet` with the `exemptions` file treated as a **debt list**, and a CI check that the exemption count only ever decreases.

> Without the target filter this gate cries wolf on day one, and a gate that cries wolf gets an `--ignore` flag added to it within a month.

---

### D27 — Real packaging numbers

**Sections:** `01 §5.8`, `02` "Packaging model"
**Evidence:** S12e, S35

**Measured on Debian/Ubuntu, x86_64:** engine `.so` closure installed **42.7 MiB**; a Rust binary that really links libvips **300 KiB**, and **15 MiB** including the 103 libraries it actually touches.

**Change.** Keep the ⚠️ and the `<60 MB` target, but cite these rather than nothing. State plainly: **one worker, one target, dynamically linked against system copies** — a shipped product bundles its own, adds three more workers, the Tauri shell, Inter and the CLI, and pays signing overhead. `08 §1`'s week-0 measurement (the *packaged* set, per target) remains the number that matters.

---

### D28 — The Class A gate at w4 has no Class A operation

**Sections:** `08 §4`, `08` commit 23
**Evidence:** review analysis of the commit list *(not a spike)*

**Current.** Commit 23 gates *"Class A round-trips byte-identically"* at w4, naming its subject as *"metadata-only edits and lossless re-container — the operations that exist in week 4."*

**Neither exists.** Commits 15–28 build sniff, polyglot, identity, registry, the image-rs adapter (PNG/JPEG/WebP/GIF/BMP/TIFF), executor, receipts and CLI. Lossless re-container is weeks 27–30; JPEG XL needs libjxl at week 18+. The week-4 product is `a.png → jpg`, which is **Class B**.

**Change.** Either add a JPEG metadata-strip commit in week 3 — small, already Class A in the `02` catalogue, and it gives the gate a subject — or stage the row `w4 (metadata) / w30 (re-container)` the way SR-5 and SR-13 already are. Move the SSIM half to w15+, where a reference corpus exists.

> `08 §4` opens by claiming *"a gate's week is a week its test can actually pass."* This is the third revision in which that rule has been broken.

---

## 4. After applying — done

1. ~~Bump each touched document's version and add its "what changed" row.~~ **Done** — `03` and `09` carry v0.6 headers citing spikes rather than review rounds.
2. ~~**Check every anchor still resolves.**~~ **Done** — a checker was written for the pass and found two real breaks: a heading this change order itself renamed (`03 §8.4`), and 24 stale links into the private validation record. Both fixed; all public cross-document anchors and file paths now resolve.
3. ~~**Delta D9 changes `09 §9`'s audit scope.**~~ **Done** — the auditor brief no longer says `-os` holds no invariants.
4. **Next: Phase 1.** The record is now the input to the skeleton.

<details><summary>Original checklist</summary>

1. Bump each touched document's version and add its "what changed" row — citing **the spike**, not a review round.
2. **Check every anchor still resolves.** These deltas rewrite headings (D10 renames a gate) and table rows that other documents deep-link into; `03`, `08` and `09` cross-reference each other heavily. There is no link checker in this repo — write one, or verify by hand.
3. **Delta D9 changes `09 §9`'s audit scope.** It tells the third-party auditor that `-os` "holds no invariants". After D7 and D9, `-os` constructs the `SandboxProfile` from read-back — which *is* an invariant. Update that sentence or the auditor is briefed wrong.
4. **Then start Phase 1.** The record is the input to the skeleton, and D1, D3, D7, D8, D9 and D13 all change what gets written in week 1.

</details>

---

## 5. The three that needed a decision — **DECIDED 2026-08-14**

All three are settled. The record has been edited to match.

### D8 → **a user-facing setting, defaulting to provenance-keyed reuse**

Neither option on the table. The call was to **make the tradeoff the user's** — which is better than either, because it is the one question in this document whose right answer genuinely depends on who is asking.

| `Policy::worker_reuse` | Behaviour |
|---|---|
| **`Balanced`** *(default)* | A worker is reused across files sharing a **trusted** provenance. Any untrusted-provenance file gets a worker of its own |
| **`Isolated`** *(user toggle)* | One worker per file, always — the original D11. Costs ~38 ms/file on Windows, and the UI says so |

Surfaced in the UI and the CLI, recorded in the receipt, and **`Isolated` is a floor a policy may raise to but never lower from**, exactly like `IsolationFloor`. Managed deployments pin it; individuals choose.

> **Why this beats either original option.** The residual is *cross-file*, so whether it matters is a property of the user's threat model, not ours. A photographer batching 400 of their own RAWs and a lawyer processing discovery from opposing counsel want opposite things, and we have no basis for choosing between them. It also converts a hidden architectural compromise into a visible, explicable product feature.

### D17 → **dynamic linking, enforced by CI gate**

`engines.toml` gains `link_mode`; any engine with an LGPL licence must be `dynamic` or the build fails. `02 §11`'s "not owed" row is rewritten to state the real premise.

**Additional instruction:** the AppContainer-SID permission requirement (S9 — now applying to the shared libraries too) goes into the **implementation documentation**, not only the threat model. It is something the person writing the code needs in front of them, not something an auditor discovers later. See **D5** and the new `03 §5.3`.

### D28 → **split the gate, add the week-3 feature**

`Class A byte-identical (metadata edits)` at w4, with a new week-3 JPEG metadata-strip commit as its subject. `Class A byte-identical (lossless re-container)` at w30. The strip also gives **A8/SR-11** its first real test.
