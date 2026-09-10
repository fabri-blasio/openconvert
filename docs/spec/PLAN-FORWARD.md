# OpenConvert — Forward Plan

**How to get from a design record to shipped software that is safe, professional, and low-defect.**

Written 2026-08-13, after two independent reviews of the v0.5 record ([mine](REVIEW-2026-08-13-independent.md), [the other](REVIEW-2026-08-13.md)) both landed at 64–65/100 and both concluded the same thing: the documents have saturated what more writing can buy.

---

## 0. The strategy in one paragraph

**Stop revising documents. The next artifact is a measurement, and the one after that is code.** Four independent reviews scored this design 59–65 across three revisions, and the reason the number never moves is that each round fixes the defects the last round named while the fixes generate the next round's defects. So this plan does three things in order: **measure the things that can invalidate the architecture**, **convert the ~40 outstanding findings into enforceable properties** rather than forty patches, and **build a narrower v1.0 than the record describes**.

> ### ⚑ Revised 2026-08-14, after running it
>
> **Phase 0 has been done**, as far as one Windows machine with Docker and WSL allows. **42 spikes designed, 41 run** — results in the private validation record, what is left in [§8](#8-what-still-needs-to-be-tested) and the private validation backlog.
>
> **19 predictions were refuted, five of them mine** — including S27, which refutes finding F11 from my own review of this design. Everything below is written against measurements rather than reasoning, and where a measurement contradicted an earlier recommendation *in this document*, the correction is stated rather than quietly applied.
>
> **The single biggest thing the measurements changed:** Windows and Linux are safe in *opposite places*, and the design record has one model for both. That changes which platform to ship first, what `OutputName::parse` is actually for, and what the `Minimal` tier is worth.
>
> ### ✓ Applied to the record, 2026-08-14
>
> The measurements below are no longer only in this document. **All 28 deltas in [DESIGN-DELTAS.md](DESIGN-DELTAS.md) have been applied**; `03-ARCHITECTURE`, `08-EXECUTION-PLAN` and `09-THREAT-MODEL` are at **v0.6**, and `README`, `01`, `02` and `04` carry their corrections. The three open decisions were settled: **`worker_reuse` became a user setting** (default `Balanced`, `Isolated` available and never lowerable by policy), **engines link dynamically under a CI gate**, and **the Class A gate was split with a week-3 metadata strip added to give it a subject**.
>
> **Read [§8](#8-what-still-needs-to-be-tested) before trusting any number here.** **Only S11 (macOS) is unrun.** Three things can still invalidate part of the architecture, and all of macOS is among them.

---

## 1. Phase 0 — what it measured

The record's Week 0 listed three measurements. This plan called for six. **Forty-two spikes were designed and forty-one ran** (RESULTS), across bare Windows, Docker Linux, WSL Ubuntu and emulated arm64.

### Performance and packaging

| Question | Answer | Consequence |
|---|---|---|
| Protocol round-trip, 4 KB | **348 µs p50, 614 µs p99** | ✅ `08 §1`'s sub-millisecond criterion is **met**. Two-phase detection is affordable |
| Bare spawn, Windows | **40.15 ms p50** (creation 15.8, teardown ~24) | ❌ **161% of the whole 25 ms budget**, before any confinement |
| Bare spawn, Linux | **1.96 ms p50** | ✅ **7.9%** of the budget. A **20× platform gap** |
| Would signing help? | **No.** A Microsoft-signed binary costs 15.0 ms to create; ours 15.8 | Not a signing or packaging problem |
| AppContainer marginal cost | **4.43 ms** | The sandbox is nearly free. **The spawn is the problem** |
| **Warm worker, 12 files** | **2.19 ms/file** vs 38.80 cold — **17.7×** | ✅ **`D11` can be amortised. The gate is met with 91% headroom** |
| Engine closure size, Linux | **42.7 MiB** (everything installed) | Upper bound for the engine set |
| **A worker that really links libvips** | **300 KiB binary; 15 MiB with its 103 shared libs** | One worker, one target, system copies. `<60 MB` is plausible for one platform, tight for three |
| ASan on a C-linking binary | **Builds and runs** (1286 KiB vs 300) | The nightly sanitiser job is demonstrated, not assumed |

### Confinement — Windows

| Question | Answer | Consequence |
|---|---|---|
| Worker with no path? | **Yes** — `NtCreateFile` + `RootDirectory`; `FILE_CREATE` is a real `O_EXCL`; **the kernel refuses `..`** | The model works; needs `unsafe` and an NT API |
| Does the handle list close the leak? | **Yes** — unintended writes 1 → 0 | `std::process::Command` leaks **64 handles**; only `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` fixes it |
| Is the handed handle sufficient in an AppContainer? | **No** — `STATUS_ACCESS_DENIED` | **Every job dir must be ACL'd for the container SID.** Not in the record |
| Can a capability be granted/revoked on a **running** confined worker? | **Yes, both** — `DuplicateHandle` / `DUPLICATE_CLOSE_SOURCE` | The warm pool is safe under the `Full` tier |
| Job Object enforces `Limits`? | **Yes** — but the worker **aborts** with no `Failed{..}` | See the next row |
| Can the host learn *which* limit tripped? | **Yes** — `PROCESS_MEMORY_LIMIT` via an IO completion port | ✅ **Fixes the gap.** `03 §13` can name the limit |
| **Does ACG engage?** | **Yes, in an AppContainer** (0 plain → 1 in container). The explicit request does nothing | ⚠️ **Corrected — I published the opposite.** See §1.5 |
| **Does CIG engage?** | **No** — a non-Microsoft DLL loaded in **all four** cells | ❌ **One of four `Full`-row mitigations does not engage** |
| Do a restricted token and AppContainer compose? | **Yes** — `CreateProcessAsUser` takes the capabilities attribute too | The `Full` row is constructible as written |

### Confinement — Linux

| Question | Answer | Consequence |
|---|---|---|
| seccomp vs libvips? | **Survives** — deny `AF_INET`/`AF_INET6`, permit `AF_UNIX` | ✅ The `Reduced` tier's premise holds |
| Landlock without a userns? | **Confirmed**, unprivileged, **ABI 3 not 4** | ✅ The v0.5 ladder rewrite's central claim. Requires runtime ABI masking |
| Is the `Full` tier reachable unprivileged? | **Yes** — userns + empty netns as uid 1000 | Network denial can come from the netns, not only seccomp |
| Does `openat(fd, "..")` escape? | **YES** — and Landlock blocks it (`EACCES`) | **The platforms are safe in opposite places** |
| Do limits fail usefully? | **Yes** — `RLIMIT_AS` gives a recoverable `Err` | Linux can send `Failed{..}`; Windows cannot without the IOCP |

### Correctness, filenames and data safety

| Question | Answer | Consequence |
|---|---|---|
| Destructive `std` APIs | **5 on Windows, 7 on Linux** | `symlink`-after-`unlink` is a POSIX route no truncating-open scan sees |
| **Can `expansion_ratio` separate a bomb?** | **No.** Legit disk image **1029×**, zip bomb **1028×** | ❌ **The control `09 §7` names first cannot work.** Depth and `archive_total_bytes` can |
| Does libarchive impose any limit? | **None** — 512 MB from a 522 KB archive, no error | Every limit must come from us |
| Do the NTFS premises hold? | **5 of 6.** **NTFS does not fold NFC/NFD**; APFS does; ext4 is case-sensitive | Three filesystems, three outcomes for one archive |
| Does our own copy preserve mark-of-the-web? | **`read`+`create_new` STRIPS it** — the API I12 mandates | ❌ **Two controls conflict.** New finding |
| Is `O_EXCL` weaker on a non-local filesystem? | **No** — held across 9p to NTFS | ❌ **Refutes my own review finding F11** |
| Long paths | **829 chars fine** for our Rust; **`MAX_PATH` binds the C engines** | Bound the job-dir path; set `longPathAware` |
| Containment property | **4000 cases, no escapes** | ✅ The parser design is sound |

### Practice

| Question | Answer | Consequence |
|---|---|---|
| **Would the tests fail if the thing broke?** | **64%** — 10 of 30 mutants survived, incl. a **vacuous test** and **dead code** | ❌ Fixed to 100%. **Mutation testing is the only tool that audits the tests** |
| Reproducible across build roots? | **No** — same source, different root, different bytes | Scope the attestation to a fixed image and path |
| Does `depcheck` replace the false `forbid` claim? | **Yes** — fires on `libz-sys [links = z]` | The working gate for D7 |
| Does `cargo-audit` earn its place? | **Yes, with care** — real advisory in `postcard`, **for a target we never build** | Needs `--target` filtering or it cries wolf |
| Can Miri check this project's `unsafe`? | **No** — it cannot cross FFI | Pure core only; ASan/UBSan + fuzzing for the workers |

### What still belongs in Phase 0 — writing, not engineering

- **State the target matrix.** Still absent from every live document, and now with evidence: the S10 spike compiled for x86_64-linux and **failed** for aarch64-linux (`c_char` signedness), every syscall number in a seccomp filter is per-architecture, and `docker buildx` **advertises** `linux/arm64` while being unable to **execute** it until binfmt is registered. **A CI matrix that builds for a target it never runs reports green while testing nothing.**
- **Declare MSRV and the supported-OS floor.** Landlock needs ≥ 5.13 *and* runtime ABI negotiation — the test kernel reported **ABI 3, not 4**, and passing a bit the kernel does not know is `EINVAL`.
- **Decide static vs dynamic linkage of the LGPL engines.** Never a measurement; still unanswered. Determines an obligation `02` declares "not owed".
- **Write down the `Limits` defaults.** S23 supplies the data (below); the corpus supplies none.

### The `Limits` values, derived from measurement

`03 §10.1` gives `Limits` ten fields and **the entire corpus specifies not one value**. A control whose threshold is unknown is not a control. From S23, measured against real libarchive:

| Field | Starting value | Why |
|---|---|---|
| `archive_depth` | **32** | Legitimate content topped out at 8; the bomb was 41. Clean separation |
| `archive_entries` | **100,000** | The bomb had 40,000 — and so does `01 §7`'s archivist workload. Cannot be a tight bound |
| `expansion_ratio` | **≥ 10,000, tripwire only** | A legitimate zeroed disk image is **1029×** and the bomb **1028×**. **This field cannot be a primary control** |
| `archive_total_bytes` | from `Budget` | With the per-job `Budget`, this is what actually bounds a bomb |

**`09 §7` step 7 must be rewritten.** It stops the bomb with *"running totals trip `expansion_ratio`, `archive_depth`, `archive_total_bytes`"* and names ratio first. Ratio is the one that cannot do it.

---

## 1.5 What the measurements changed

Seven findings that alter the plan, in order of how much they alter it.

### 1. The platforms are safe in opposite places

| | Windows | Linux |
|---|---|---|
| `..` through the job-dir handle/fd | **BLOCKED** by the NT object manager (`0xC0000033`) | **ESCAPES** — `..` is an ordinary component to `openat` |
| Descriptors the worker was not handed | **64 visible, 2 writable** | **0** — Rust sets `CLOEXEC` by default |
| Process creation | 40 ms | 2 ms |
| Filesystem confinement | AppContainer + a **job-dir ACL** | Landlock, unprivileged |

**Consequences.** `OutputName::parse`'s separator rejection is **load-bearing on Linux and merely defence-in-depth on Windows** — the reverse of the intuitive reading, and `03 §5.2` states one rule for both. The Linux `Minimal` tier (no Landlock) provides **no filesystem containment at all**, which is the empirical case for refusing it. And `03 §5.2`'s sentence — *"containment is enforced by the OS; the `openat` discipline is defence in depth, not the guarantee"* — is exactly right and now proven, in both directions.

### 2. The latency gate is a Windows problem, and the sandbox is not the cause

25 ms p50 for a whole sandboxed job. Windows spends 40 ms before confinement and 4.4 ms on it. Linux spends 2 ms. `03 §17`'s instruction — *"move more formats onto the in-process path; never weaken the sandbox to pass its own gate"* — is right, and now has a sharper form: **one threshold for three platforms cannot be correct when the platforms differ by 20×.**

**S16 has now settled how to fix it.** A warm worker serving 12 files costs **2.19 ms/file against 38.80 ms** for one process each — a 17.7× speedup that lands Windows at 91% headroom under the gate, with nothing weakened. And the security question a warm pool raises is answered: `DuplicateHandle` hands a **new** job directory to a **running** worker (which still never learns a path), and `DUPLICATE_CLOSE_SOURCE` **revokes** it — after revocation the worker gets `STATUS_INVALID_HANDLE`. A warm worker does not accumulate access to every job directory it has served.

**So `D11` changes from "one process per file" to "one worker per batch, one job directory at a time."** The cost is real and must be taken deliberately: `03 §8.4` defines cancellation, cleanup ownership and crash containment per-worker-per-file, and all three need restating in terms of a job; and **revocation removes the handle, not the memory**, so a compromised engine can carry bytes from one file to the next *within a batch*. That residual belongs in `09 §8`, and it is the argument for a fresh worker per batch rather than a long-lived pool.

### 3. A mechanism can report success and not engage

> **⚠️ This finding was published wrong once, and the correction is the better story.**
>
> S14/S17 requested ACG on a **plain process**, read back `ProhibitDynamicCode = 0`, and I wrote *"ACG silently declines"* and generalised to *"two of four `Full`-row mitigations do not engage."* S20b then got `ProhibitDynamicCode = 1` from an AppContainer process that requested **no mitigation at all**. A 2×2 settled it:
>
> ```
>   plain     / no request    0        AppCont.  / no request    1
>   plain     / ACG asked     0        AppCont.  / ACG asked     1
> ```
>
> **ACG is a property of the AppContainer, not of the request** — and every worker ships inside one. My original measurement was of a configuration the product never uses. **CIG, re-tested the same way, does not engage in any of the four cells**, so that half stands.

**The corrected statement:** one of the four mitigations in `03 §9.2`'s Windows `Full` row (**CIG**) does not engage; ACG arrives via the container rather than the flag.

**What survives, and is the reason Property 7 exists:** the explicit `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY` request does nothing on a plain process while **every API reports success** — `UpdateProcThreadAttribute` returns success, `CreateProcessW` succeeds, and `ProhibitDynamicCode` stays 0. A `SandboxProfile` assembled from *requests* is untrustworthy in **both** directions: it would have over-reported ACG on the plain process, and it would have *under*-reported it in the container, where ACG is present without being asked for.

> **You cannot know what engaged without reading it back.** That is Property 7, and it nearly failed in the direction I did not anticipate.

### 4. Two independent confirmations that `-sandbox` cannot own the spawn

Passing pre-opened descriptors requires `unsafe` on **both** platforms — `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` via raw `CreateProcessW` on Windows, a `pre_exec` hook on Linux. `std::process::Command` can express neither. `03 §5` puts `Worker` and the spawn in `openconvert-sandbox`, which is `#![forbid(unsafe_code)]`. **This is now measured on two platforms by two different routes and is not going away.**

### 5. The job directory needs an ACL, and the record does not say so

Inside an AppContainer the handed handle is **necessary and not sufficient** — `NtCreateFile` re-checks access for the new file against the child's token. Every job directory must be ACL'd for the container SID before the worker starts: an extra per-job step, an extra failure mode, an extra sweep obligation.

### 6. Filesystems disagree about filenames, and the record assumes they agree

**NTFS does not fold NFC and NFD; APFS does.** So the same archive extracts as two files on Windows and one on macOS. The software-side collision check is **required**, not optional. Separately, only `NUL` vanishes into a device on modern Windows via Rust's verbatim paths — the reserved-name predicate is still right, but for the **C engines and downstream tools**, not for our own code. And where a name does vanish, the write succeeds and **the conversion reports success while the output is silently discarded**, which is not in `03 §13`.

### 6b. Two of the design's own controls conflict over mark-of-the-web

`03 §9.2` raises the floor to `Full` for files carrying mark-of-the-web. `03 §11.3` **copies** untrusted-provenance inputs into the job directory before routing. And measurement says `fs::copy` and `fs::rename` **preserve** the `Zone.Identifier` stream while **`read` + `create_new` + `write` strips it** — which is the creation path I12 mandates, because `create_new` is *"the only creation API"*.

> **The copy taken *because* a file is untrusted destroys the evidence that it is untrusted, if it uses the only API the design permits.**

The design is partly saved by accident: `Provenance` is already a field of `FileFacts`, captured before the copy. But any later re-check, any resumed batch reading a journalled `PlanRequest`, or any verification against the copied file sees a clean file. `09 §8` says MOTW "fails open"; it does not say **our own copy is one of the things that opens it**. Either copy the stream explicitly, or state that provenance is captured once and never re-derived.

### 6c. Windows and Linux fail limits differently, and only one can explain itself

`RLIMIT_AS` on Linux gives the worker a **recoverable** `Err` — it can still send `Failed{..}` and `03 §13` gets its actionable message. A Windows Job Object cap **aborts** the process. The fix exists and is cheap: an IO completion port on the job delivers `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` *before* the exit code, so the host can synthesise the message. **Add `JOBOBJECT_ASSOCIATE_COMPLETION_PORT` to the worker-start sequence and give `03 §13` a row per message type.** Without it the limit is enforced and unexplainable.

### 7. Limits enforce, but they break the protocol

A Job Object memory cap works — a capped worker dies where an uncapped one succeeds. But it **aborts** (`0xC0000409`) without sending `Failed{..}`, so `03 §13`'s "engine returns `Failed`, message names the file, step and engine" cannot cover it. The host has to synthesise the message from an abnormal exit code, and that needs its own row.

---

## 2. The six properties

Both reviews produced ~40 findings between them. Do **not** work a 40-item list. Nearly all of them are instances of six properties that are currently enforced by intention. Make each one enforced by a build, and the instances retire together — and, more importantly, so do the ones nobody has found yet.

### Property 1 — The pure core is pure, and a machine checks it

> **⚠️ CORRECTED 2026-08-13 after running S1.** The first version of this section recommended a `wasm32-unknown-unknown` build as the purity gate and called it "the single best move in the whole plan." **That was wrong.** `std` is fully available on `wasm32-unknown-unknown`: `std::fs::File`, `SystemTime::now()` and `std::env::var` all compile and fail only at runtime. The spike built a fixture containing all three leaks and produced a wasm32 `.rlib`. The gate would not have caught the `InputHandle` defect. What follows is the corrected mechanism, measured rather than assumed.

**Enforce it with two things, from commit 1.**

**(a) A checked-in `clippy.toml`,** gated by `cargo clippy -- -D warnings`:

```toml
disallowed-types = [
  { path = "std::fs::File",         reason = "openconvert-core is pure: no I/O (03 §4)" },
  { path = "std::fs::OpenOptions",  reason = "openconvert-core is pure: no I/O (03 §4)" },
  { path = "std::time::SystemTime", reason = "time is an input, never a read (03 §4)" },
  { path = "std::time::Instant",    reason = "core reads no clock (03 §4)" },
  { path = "std::process::Command", reason = "core spawns nothing (03 §4)" },
]
disallowed-methods = [
  { path = "std::env::var",              reason = "core reads no environment (03 §4)" },
  { path = "std::time::SystemTime::now", reason = "core reads no clock (03 §4)" },
  { path = "std::fs::read",              reason = "core does no I/O (03 §4)" },
  { path = "std::fs::write",             reason = "core does no I/O (03 §4)" },
]
```

Measured: this rejects all four leaks, by name, with the reason in the error message, while leaving `Vec`, `String` and `HashMap` available — which `openconvert-core` needs. Honest class in the record's own taxonomy: **Hard**, because `#[allow(clippy::disallowed_types)]` bypasses it. So pair it with a CI grep for `#[allow(clippy::disallowed` — a bypass then costs a visible diff on a gated file, which is the same shape as the `feature = "attest"` control.

**(b) `#![no_std]` + `extern crate alloc`, if the core's dependency list allows it.** Measured: the leak fixture fails with `error[E0433]: cannot find module or crate 'std'`. That is **Impossible** class — the strongest available — and it is worth real effort to reach, because §7.4's whole value is the Impossible/Hard distinction. Cost: `std::collections::HashMap` becomes `hashbrown`, and `std::error::Error` is unavailable. Evaluate it in week 1 against the real type list rather than deciding now.

**Keep the wasm32 build as a CI job anyway** — just not as a *purity* gate. It genuinely verifies that `03 §15.1`'s "compiles to `wasm32` unchanged" claim and the whole `04 §5` tool-page channel still hold, which is currently asserted and never checked. Label it a portability gate so nobody mistakes it for the purity one again.

**What this retires:** `InputHandle` in `openconvert-core` (clippy flags `std::fs::File` at commit 4); every future clock, env or filesystem leak into the core; and the unverified wasm32 portability claim.

**What you must change to make it pass:** move `InputHandle` to `openconvert-run`. `FileFacts` keeps `content_id`, `Sniff`, `Properties`, `Provenance` and becomes a plain value; `-run` holds `HandleTable: InputToken → File` and does every read. **The identity guarantee (SR-16/I13) is preserved by the pairing, not by the field placement** — `execute()` carries `(FileFacts, InputHandle)` and reads only through the handle it was given.

While you are there, fix the signature that currently cannot work: `route(req: &PlanRequest, env: &Environment)` has no way to see the `FileFacts` it must evaluate `Requirement`s over. Make it `route(req: &PlanRequest, facts: &FileFacts, env: &Environment) -> Plan`, with `PlanRequest = Target + Policy + parameters`. That also makes recipes portable, because a recipe *should* re-derive facts from the new file.

---

### Property 2 — No untrusted value reaches a decision the receipt attests to

This is the deepest defect in the record and it is one design decision, not five findings.

Today: `probe()` runs the memory-unsafe engine **inside** the sandbox and returns `Properties`; `route()` — pure, on the host — evaluates requirements over those `Properties` and attaches the `Class` the receipt attests to, plus the `Limits` for every later step. A compromised engine that never escapes the sandbox chooses its own fidelity class. The machine is safe; the receipt — the product's second pillar — is not.

**Enforce it with three mechanisms:**

**(a) A wire/domain type split, with a lint.** Every value crossing the protocol arrives as a `wire::` type that is *not* the domain type, and must be converted through a fallible, clamping constructor:

```rust
// crates/openconvert-proto/src/wire.rs — the ONLY place Deserialize appears
#[derive(Deserialize)]
pub struct RawProperties { /* plain fields, no invariants */ }

#[derive(Deserialize)]
pub struct RawOutputName(String);       // NOT OutputName

// host side
impl RawProperties {
    pub fn validate(self, sniff: &Sniff, ceiling: &LimitCeiling)
        -> Result<Properties, WireError>;
}
impl RawOutputName {
    pub fn parse(self) -> Result<OutputName, NameError>;   // the same parse, no bypass
}
```

Gate it: **a CI lint asserting `derive(Deserialize)` appears nowhere in `openconvert-core` or `openconvert-sandbox` outside `wire.rs`.** The design already reasons this way for `Plan` ("private fields and no `Deserialize`") and simply did not extend it to the types that actually travel. One lint closes the whole class, including the `OutputName`-arrives-pre-validated hole.

**(b) A limits ceiling that engine facts cannot widen.** `Policy` carries `LimitCeiling`; `route()` clamps. Engine-supplied properties may only *narrow* limits, never raise them. Today, inflated declared dimensions inflate `decode_pixels`, `memory_bytes` and `wall_time` for later steps — a contained compromise becomes a host resource amplifier that stays within policy.

**(c) Host-side protocol bounds, checked before allocation.** `max_frame_bytes`, `max_messages_per_step`, `max_progress_rate`. The `Limits` vocabulary protects the sandboxed process from the file; nothing protects the unconfined host from the sandboxed process. A hostile length prefix is a *correct* decode of a hostile value, so fuzzing will never find it — only a bound will.

**And the design decision underneath all three:** route on **phase-1 `Sniff` only** to a provisional plan; let the engine **reject** a step it cannot perform rather than **shape** the step it will perform; re-route on rejection. Your own design already protects `Isolation` this way (derived from phase-1 facts, precisely so an engine cannot demote itself). `Class` deserves identical treatment and does not get it. Where a probe-derived property genuinely must influence the class, **mark that class provisional in the receipt** — the vocabulary already exists (`⌇`).

---

### Property 3 — Nothing this program does can destroy or replace a file

You have this for `File::create` and `O_TRUNC`. The class is wider — **measured**, not guessed. S4 ran every `std` write API against a file containing known bytes:

```
  File::create                 DESTROYED   caught by the stated lint
  File::create_new             PRESERVED   the sanctioned API
  OpenOptions::write+truncate  DESTROYED   caught
  fs::write                    DESTROYED   *** GAP ***
  fs::copy                     DESTROYED   *** GAP ***
  fs::rename                   DESTROYED   *** GAP ***
  OpenOptions::append          MODIFIED    *** GAP ***
  set_len(0)                   DESTROYED   *** GAP ***
```

**Five gaps, not the three I predicted.** `set_len(0)` is the sharpest: it truncates through a handle opened with plain `write(true)`, so there is no truncating *open* for any scan to find.

**Enforce it with:** one module — `openconvert-fs` — that is the only code in the workspace permitted to mutate the filesystem, exposing exactly:

```rust
pub fn create_new(dir: &DirHandle, name: &OutputName) -> io::Result<File>;   // O_EXCL / CREATE_NEW
pub fn unlink_verified(dir: &DirHandle, name: &OutputName, expect: ContentId) -> io::Result<()>;
```

and a lint banning, workspace-wide outside that module: `File::create`, `OpenOptions` with `truncate` or `create`, `fs::write`, `fs::copy`, `fs::rename`, `fs::remove_file`, `fs::remove_dir_all`, and the Windows equivalents (`MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`, `SetFileInformationByHandle` with `FileRenameInfo`).

**Why the widening matters:** `fs::rename` silently replaces its destination, `fs::write` and `fs::copy` truncate — and write-to-temp-then-rename is the *most idiomatic* way to make a write atomic. It is exactly what a careful contributor reaches for after reading your own §13 rows that say "partial output removed." Your lint as specified catches the careless act and not the conscientious one.

**Two consequences to fix while you are here:**

- **Deletion must go through a handle, not a path.** `unlink_verified` above. Today the only code that deletes user-visible files (receipt-unwritable rollback, disk-full cleanup, undo) does it by path in an error path in a directory you do not control.
- **Apply `on_conflict` to the receipt sidecar.** Right now a pre-planted `photo.jpg.receipt.json` makes every conversion producing `photo.jpg` fail *and delete its own output* — two correct controls composing into an attacker-triggered DoS. And **stop deleting a successful output because a sidecar failed**: write the receipt to the state directory, mark the output unattested in history and the manifest, and tell the user. The second pillar survives an unattested output better than the user survives losing work that succeeded.
- **Add the Linux tail: `symlink` after `unlink`.** S25 found **seven** destructive routes on Linux, not five — `fs::remove_file` and `symlink`-over-a-path are both POSIX ways to replace a file that no "truncating open" scan can see.

> **⚠️ CORRECTED — I was wrong about `O_EXCL`.** The first draft of this section said *"`O_EXCL` is not reliably atomic over NFS or on sync/offline-capable volumes"* and my review rated it MEDIUM-HIGH (finding F11). S27 tested it across a 9p/drvfs boundary to NTFS — a genuinely non-local filesystem — and **`O_EXCL` held, refusing with `EEXIST`, exactly as on native ext4.** `O_NOFOLLOW` held too.
>
> **F11 should be downgraded to LOW and marked unverified.** It remains a theoretical concern for NFSv3 specifically; I have no evidence for it and the one non-local filesystem available upheld the guarantee.
>
> **What the test found instead is more useful:** `rename` replaced its target silently on *both* filesystems. So the threat to I12 was never a weak `O_EXCL` — it is the six other APIs that never use `O_EXCL` at all. That is the version of this argument to keep.

---

### Property 4 — Every process boundary is deny-by-default

Three boundaries ship or will ship. Two have no design.

| Boundary | Status after the spikes | What to do |
|---|---|---|
| Host ↔ worker, **Windows** | **Measured and working**, with two additions the record lacks | `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` **and** a per-job ACL for the container SID |
| Host ↔ worker, **Linux** | **Measured; `openat` alone does not contain** | Landlock is mandatory, keyed off the handed fd |
| **Webview ↔ Rust backend** | **Still nothing** | See below |
| Shell extension (v1.1) | Designed well, cut from v1 | Keep it cut |

**What the host↔worker measurements add to the record:**

1. **`std::process::Command` cannot express either platform's requirement.** On Windows it inherits *every* inheritable handle — S3 found 64 the worker was never given and **wrote through two, including one deliberately withheld**. `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` closes it (S9: unintended writes 1 → 0). On Linux, Rust's `CLOEXEC` default means fd 5 only arrives through an `unsafe` `pre_exec` hook.
2. **The job directory must be ACL'd for the AppContainer SID.** The handed handle is not sufficient; `NtCreateFile` re-checks. Add it to the worker-start sequence and to the sweep.
3. **On Linux, Landlock is not optional.** `openat(5, "../secret/x")` escaped; with Landlock it returned `EACCES`. And the rule can be keyed off `parent_fd: 5`, so the worker confines itself **without ever learning a path** — which is exactly what `03 §5.2` wants and does not say.
4. **Socket *creation* stays permitted inside an AppContainer.** `09 SR-1`'s test must assert on **reachability against a listener the test itself runs**, never on socket creation and never on a connect to nothing. My first version of that spike made both mistakes and reported a denial that was not there.

The webview is the one boundary that ships in v1 and still has no controls beyond a CSP. Concretely:

1. **`apps/desktop/src-tauri/capabilities/main.json`**, checked in with the first GUI commit, listing every permitted command explicitly, scoped to the main window, with `shell`, `fs`, `process`, `http` and `updater` plugins **absent**. Deny-by-default is Tauri v2's model; you simply have not used it.
2. **A lint asserting no `@tauri-apps/plugin-{shell,fs,process,http}` in `package.json`.** Your `no Command::new outside -sandbox` scan cannot see a `Command::new` inside a Tauri plugin dependency — which means I7 ("shell injection is unrepresentable", classed **Impossible**) is Impossible in Rust and merely Forbidden across the IPC boundary until this gate exists.
3. **One `DisplayName` newtype** that strips control characters, ANSI CSI/OSC sequences and bidi overrides, applied to every untrusted string before it is rendered — in the webview *and* in the terminal. Your threat model names "a crafted filename rendered into the UI" as an example of A15 and gives it no control; input names arrive as `hint: Option<&OsStr>` and are never parsed at all. An archive member whose name contains OSC sequences rewrites the terminal on `openconvert convert`.
4. **A lint banning `{@html}` in Svelte components.** Two lines of grep.

---

### Property 5 — Every claim has a mechanism, at a week the mechanism can run

You have a gate table and a rule that "if it is not in this table, it is prose." The rule is right and the table has drifted three times: SR-5/11/13 were gated at impossible weeks in v0.4 and fixed; SR-15 and SR-19 are gated at impossible weeks now; the Class A gate at w4 has no Class A operation in commits 1–28 to protect; SR-2's w2 property test quantifies over arbitrary registries while the shipped registry is never checked by anything.

**This is a recurring class, so fix it with a machine and not with vigilance:**

```toml
# requirements.toml — the single source of truth, checked by CI
[[requirement]]
id       = "SR-15"
claim    = "A conversion cannot destroy a file that already exists"
tests    = ["lint::no_destructive_fs_api", "conflict::suffix_skip_fail"]
live_from = 4
[[requirement.staged]]
tests    = ["traversal::collision"]
live_from = 24            # oc-archive exists
depends_on_commit = "oc-archive:first-extraction"
```

Then a CI job that, at every commit, asserts: **every test named in `requirements.toml` whose `live_from` has passed exists, is not `#[ignore]`d, and ran.** A gate that cannot be found is a build failure. This converts your gate table from a document into a build artifact, and it is the only thing that will stop the drift recurring — it has now recurred three revisions running.

Generate `09 §5` and `08 §4` *from* this file so the two tables cannot disagree again.

**Two specific repairs while you build it:** SR-2's real enforcement is a hand-maintained registry table with no automated check (add `memory_safety = "rust" | "ffi"` to `engines.toml`, derive the minimum isolation from it, gate that every `EngineBin` variant has a row and that `ffi ⇒ Sandboxed`). And give the Class A gate a subject — a JPEG metadata-strip in week 3 is small, is already Class A in your catalogue, and makes the gate real.

---

### Property 6 — The tests would fail if the thing they protect broke

This is the question nobody in three review rounds could answer, because it cannot be answered by reading. Answer it empirically.

**`cargo-mutants` on `openconvert-core`, as a CI gate, from week 2.**

Mutation testing changes your source (`>` → `>=`, `&&` → `||`, returns a default) and reports which mutants your test suite fails to kill. A surviving mutant is a line of code your tests do not actually test. It is normally too slow to be practical — but your core is **pure, has no I/O, and its whole test suite runs in under a second**, which is exactly the configuration where mutation testing is cheap and devastating.

Run it on `route.rs`, `predict.rs`, `policy.rs` and `isolation.rs`. Set a threshold (start at 80% mutants killed, raise it). This is the single highest-value testing practice available to this specific codebase and no document mentions it.

> **Now measured — and it justified itself immediately.** S29 ran 30 mutants against the `OutputName::parse` spike, which already had **24 hand-written evil-corpus cases and 4000 property-test cases, all green**. Result: **10 survived — a 64% score.** Among them:
>
> - `evil_corpus() -> vec![]` survived, because the corpus test **passes vacuously when its data disappears**. That is precisely the "gate that green-lights while testing nothing" failure `08 §4` exists to prevent, found in a test I had written twenty minutes earlier.
> - `||` → `&&` survived on the UNC check, because that branch was **dead code** — the separator check above it already rejected everything containing `\` or `/`. Neither the corpus nor 4000 property cases noticed an unreachable branch in a security parser.
> - `collision_key -> "xyzzy"` survived, because the tests asserted two derived values *agreed* without asserting what either one *was*.
>
> Six targeted tests and one source reorder took it to **27 caught, 3 unviable, 0 missed**. **The record's testing strategy is property tests + fuzzing + a hand-written corpus. This parser had all three, and all three missed a vacuous test and an unreachable branch.** Mutation testing is the only tool in the stack that audits the *tests* rather than the code, and it cost five minutes.

---

### Property 7 — The profile records what engaged, not what was asked for

**New, and it exists because a measurement created it.** S14 requested ACG with the correct bit; `UpdateProcThreadAttribute` returned success, `CreateProcessW` succeeded, the child started — and `GetProcessMitigationPolicy` in the child reports `ProhibitDynamicCode=0`. Strict handle checks at bit 32 engaged in the same call, so the mechanism and the encoding both work. **ACG was declined silently, with every API reporting success.**

This is a direct hit on the two claims the product is sold on. `03 §7.1` I11 says only the OS prober may assert what confinement engaged; `09 SR-11` requires the receipt to record "the profile that engaged". The natural implementation — record the flags you passed to `CreateProcessW` — produces a receipt claiming `Full` with ACG on a machine that delivered neither.

**Enforce it with:**

1. **The profile is constructed in the child, after it starts, from read-back queries** — `GetProcessMitigationPolicy` on Windows, the Landlock ABI and applied-ruleset state on Linux, `sandbox_check` on macOS — and returned to the host over the protocol as a `wire::` type that goes through Property 2's validating constructor.
2. **Requested-vs-engaged is compared, and a divergence is a first-class event.** It downgrades the tier, it appears in the plan preview, and it is recorded in the receipt. Silent divergence is the failure this property exists to prevent.
3. **A test that asserts divergence is detected**: request a mitigation known to be declined on the test machine, and assert the resulting profile does **not** claim it.

The record already has the right instinct — I11 exists precisely so nobody can fabricate a profile. It guards against a *dishonest* constructor and not against an *honest* one recording a request the OS ignored.

---

## 3. Where the bugs will actually come from

Generic advice is worthless. In *this* project, defects will cluster in four places, in this order:

### 3.1 FFI to the C libraries (`engines/*`) — the largest source, by a wide margin

You are wrapping libvips, libheif, libavif, libjxl, libraw, lcms2, pdfium, qpdf, libarchive, LAME, libopus and libFLAC. Every one is a C or C++ library with manual lifetime rules, and the `unsafe` blocks that call them are where your memory bugs will live — not in the sandbox, not in the core.

**What to do:**
- **ASan + UBSan builds of every worker binary, in nightly CI**, run over the golden corpus and the fuzz corpus. This is where you will actually find memory bugs, and it costs one workflow file.
- **`cargo-fuzz` per adapter from the day that adapter exists** — your plan already says this; the discipline is to never let an adapter merge without its target.
- **OSS-Fuzz enrollment.** Free for open-source projects, runs continuously on Google's infrastructure, and it is a credibility asset for a product whose pitch is "we parse hostile input safely." Apply the week `oc-images` exists.
- **Wrap each C library once, in one module, with the lifetime rules written above it as a comment.** Never call the raw binding from two places.
- **`Miri`** on any pure-Rust `unsafe` — it cannot cross FFI, so it will not help with the C, but it catches the Rust-side pointer mistakes.

### 3.2 Per-OS sandbox code (`openconvert-os`) — the highest-consequence source

Ten weeks of the schedule live in a crate the architecture document describes as three filenames. A bug here is a confinement failure, and it will be silent — the conversion works, the sandbox just is not there.

**What to do:**
- **Escape tests are the unit tests of this crate, not an integration afterthought.** Attempt TCP, UDP, DNS, an AF_UNIX connect, a write outside the job dir, a `fork`, a `ptrace` — from inside a real worker, per tier, **per architecture**. Every one must fail. Write them the same week you write the confinement, not at week 14.
- **Assert the profile matches reality.** After applying confinement, the worker should *verify* it (attempt one forbidden operation, confirm failure) and report that back. A `SandboxProfile` that claims Landlock engaged when the syscall silently returned `EINVAL` is the worst failure this design can have, because it is invisible and it goes in the receipt.
- **Test the failure paths, not just the success path.** Kernel too old, `unprivileged_userns_clone=0`, AppContainer blocked by policy, ACG rejected by an injected DLL. Each should produce the right tier and the right message. These are the paths users on real machines will hit, and they are the ones nobody tests.

### 3.3 Cross-platform path and filename handling — the sneakiest source

`OutputName::parse` has thirteen predicates and runs on three filesystems with different case sensitivity, different Unicode normalisation, different reserved names and different length limits.

**What to do:**
- **Property-test it with `proptest`**, not just the evil corpus. Generate arbitrary strings; assert the parse either rejects or produces a name that, when created under a temp dir and canonicalised, is still under it. That is the invariant; test the invariant, not the examples.
- **Fix the NFC/NFD rule before you implement it.** As written — "reject a component that normalises to a different name under NFC/NFD" — it rejects `café.jpg`, `Müller.pdf` and essentially every archive produced on macOS. The intent is *collision detection within one extraction*; write it that way (compare NFC forms within an archive, preserve original bytes for the write).
- **Add `CONIN$` and `CONOUT$`** to the reserved-name list.
- **Run the evil corpus on all three OSes.** A name that is safe on Linux and dangerous on Windows is the whole point.

### 3.4 The GUI state machine — the most annoying source

Drop → predict → preview → progress → result → undo, with cancellation, mixed drops, batch grouping and a 60-second undo window. This is where you will spend more debugging time than you expect.

**What to do:** keep the state machine in **Rust**, not in Svelte. The frontend renders a state it is given and sends events; it does not own the transition table. That makes the interesting logic testable without a browser, keeps one implementation for CLI and GUI, and is the same argument that correctly moved `predict.rs` into the core.

---

## 4. The practice stack

Everything below is set up once, in Phase 0 or week 1, and pays for the project's lifetime. Items marked **★** are not in the current plan and I would not start without them.

**Correctness**
- `#![forbid(unsafe_code)]` in core, sandbox, run — you have this. **Understand what it does not do:** it forbids the `unsafe` *keyword in that crate's source*. It does **not** prevent `-run` from linking libvips through a safe wrapper crate, which four documents claim it does, once as "a compile error, not a review item." ★ **Replace that claim with a real gate:** a `cargo metadata` check that no `*-sys` crate appears in `-run`'s dependency graph.
- `clippy -D warnings`, `fmt`, on every commit — you have this.
- ★ **`cargo-mutants` on the core** (Property 6). **Measured (S29): 64% on a parser with 24 corpus cases and 4000 property cases already green.** It found a vacuously-passing test and a dead branch. Highest-value item in this list.
- ★ **`proptest` alongside your hand-written property tests** — you have the right idea and the pure core makes generated inputs nearly free.
- ★ **A checked-in `clippy.toml` with `disallowed-types`/`disallowed-methods`, plus a grep for `#[allow(clippy::disallowed`** (Property 1, corrected by S1). The wasm32 build stays as a *portability* gate, not a purity one.

**Memory safety**
- ★ **ASan/UBSan nightly builds of the workers** (§3.1).
- ★ **OSS-Fuzz enrollment** once the first worker exists.
- `cargo-fuzz` per parser — you have this; start it at week 2 for `route`/`predict`/`sniff`, which cost nothing.
- ★ **Miri** on `openconvert-core` and pure-Rust parsers only. **Measured (S34):** Miri cannot cross an FFI boundary, so it can say nothing about `crates/engines/*` or `openconvert-os` — which is where every raw pointer in this project lives. ASan/UBSan plus a fuzzer are the tools there.

**Supply chain**
- `cargo-deny` — you have this.
- ★ **`cargo-audit` in CI and a weekly scheduled run** — advisories land between commits. **Measured (S33):** it found a real advisory in `postcard`, the crate `03 §5.2` names for the worker protocol — but only under `--target all`, because `cargo-audit` reads `Cargo.lock`, which is target-independent. **Configure `--target` filtering or an ignore list that records why each entry is safe**, or the gate fails over a crate that is never compiled and gets switched off.
- ★ **`cargo-vet`** — the one that actually matters for a security product. It records human audits of your dependencies and imports shared audit sets from Mozilla and Google, so you inherit most of the work. For a project selling "we know what is in our binary," this is the artifact.
- ★ **Pin every GitHub Action by commit SHA**, not by tag. A tag is mutable.
- ★ **Signing in a separate workflow that no pull request can trigger**, using OIDC rather than long-lived secrets, with the signing key in a hardware-backed store. **Your build pipeline is a softer target than AppContainer** — it holds three platforms' signing material, accepts fork PRs, and has no key custody, rotation or compromise-recovery story anywhere in the corpus. An attacker who wants your users will choose it over a kernel bug.
- ★ **`actions/attest-build-provenance`** — SLSA provenance is close to free now and it is literally the "attested builds" line you plan to sell.

**Release**
- ★ **Scope the reproducible-build claim honestly.** You sell "reproducible-build attestation" in two paid tiers and gate it nowhere. Bit-reproducible C toolchain output across three OSes has consumed dedicated teams at Debian for years. **Claim it for the Rust binaries** (`SOURCE_DATE_EPOCH`, `--remap-path-prefix`, a locked toolchain, a pinned vendored dependency tree) and say plainly that the C engine builds are pinned-and-hashed rather than bit-reproducible, until they are.
- ★ **Signed, versioned SBOM per release** (`cargo-sbom` / CycloneDX), including the native engines your `cargo-deny` cannot see.
- **`engines.toml` with a three-valued linkage field** — `subprocess` / `linked-into-worker` / `linked-into-host`, plus `link_mode = static | dynamic`. Your current two-valued field cannot express your actual situation (libraries linked into subprocesses), which is why the LGPL conclusion is unsupported.

**Operability without telemetry**
- ★ **`tracing` with a local-only subscriber**, structured logs the user can read, redact and attach to an issue. "Zero telemetry" should not mean "zero diagnosability" — it means the user chooses what to send, and can see exactly what it is. `openconvert doctor --report` producing a shareable, previewable bundle is the version of this that respects the principle.
- **Stable machine-readable error codes** on every error, surfaced in `--json`. Your error messages are excellent prose; give them identifiers so scripts and support can key on them.

**Review discipline**
- ★ **CODEOWNERS on `crates/openconvert-sandbox`, `crates/openconvert-os`, `engines.toml`, `models.toml`, every `Cargo.toml`, and `capabilities/`.** Two-person review for anything touching them. The `feature = "attest"` seal is a one-line diff; so is widening `EngineBin`; so is adding a `*-sys` dependency to `-run`.
- ★ **A `SECURITY.md` decision log.** When a control is removed or weakened, the diff must say which `SR-*` it touches and why. Your own early-warning table names "a simplification removes a control" as the top signal — give it a place to be recorded.

---

## 5. What I would cut from v1.0

This is the largest defect-reduction lever you have and it is not close.

Your own rule is *"twenty formats done correctly beats a thousand done naively."* I would apply it much harder: **four formats, one platform, shipped, beats twenty formats on three planned.**

**Ship v1.0 as:**
- **One platform, and the measurements say it should be Linux.** The first draft of this plan left the choice open. It is no longer open:

  | | Windows | Linux |
  |---|---|---|
  | Spawn vs the 25 ms budget | **161%** | **7.9%** |
  | Filesystem confinement | AppContainer + per-job ACL + read-back | Landlock, unprivileged, keyed off the handed fd |
  | Descriptor leaks | 64 found, 2 writable — needs `HANDLE_LIST` | none |
  | A mitigation silently declining | **yes (ACG)** | not observed |
  | Extra work the record does not budget | job-dir ACL, profile read-back, `HANDLE_LIST`, warm-pool decision | `..` must be rejected in software |

  **`08 §3` puts Windows first to retire the biggest risk** — *"discovering it doesn't work in month 4 would be fatal."* That reasoning was sound and **the risk is now retired**: S9 and S14 show AppContainer, the handle list and Job Objects all work, at Phase-0 cost instead of month-4 cost. With the risk retired, the argument for Windows-first is spent, and Linux is measurably the cheaper road to a shipped, fully-confined v1.0. **Do Windows second, with the ACL, read-back and warm-pool work sized from what the spikes already found.**
- **One worker** (`oc-images`: libvips + libheif). Not four.
- **The two flagship conversions**: `heic→jpg` and `mkv→mp4` stream copy. They are the entire product thesis and they exercise every part of the architecture — sniff, probe, route, sandbox, execute, receipt, and the Class A/Class B distinction.
- **CLI only.** No GUI.
- Full safety stack, no compromises: broker, limits, `O_EXCL`, receipts, escape tests, fuzzing.

**Then widen along one axis at a time:** second platform → second worker → GUI → remaining formats. Each widening is a known quantity because the one before it shipped.

**Why this is the right call:** the current plan reaches "shippable" at week 44 having never shipped anything, on three platforms simultaneously, with four workers and a GUI, built by two people who — as far as the record says — have not shipped an OS sandbox before. Every unknown compounds with every other unknown. Shipping a narrow v1.0 at roughly week 20 converts a pile of correlated risks into a series of independent ones, gets you real users finding real bugs a year earlier, and gives the build-in-public audience something to actually use.

**What this costs:** the "convert any file into anything" headline is not true at 1.0. I think that is fine — it is not true at week 44 either (no PDF/A, no LibreOffice, no FFmpeg, no AI), and the record already handles that gracefully for the archivist persona. Say what 1.0 does and ship it.

---

## 6. Revised sequencing

Against my own estimate of 55–70 weeks for the full scope at two engineers, versus the plan's 44. This sequence targets a narrow 1.0 much earlier and reaches full scope at roughly the same place.

| Phase | Weeks | Content | Exit criterion |
|---|---|---|---|
| **0 De-risk** | 0–2 | **Mostly done** — 14 spikes ran. Remaining: macOS (S11), arm64 execution (S15), the warm-pool decision (S16), target matrix, MSRV, linkage | Every number published, including the ones that failed. The Windows risk `08 §3` front-loads is **already retired** |
| **1 Core** | 3–5 | The eleven pure files. Property 1's clippy purity gate, Property 6's mutation gate, `proptest`, fuzz on `route`/`predict` — all live | `cargo test -p openconvert-core` under 1 s, 80% mutants killed, clippy purity gate green |
| **2 Skeleton** | 6–8 | `openconvert convert a.png -t jpg` end to end, in-process, `openconvert-fs` and Property 3's lint from day one | Ships. Refuses a pixel bomb, overwrites nothing, writes a receipt, phones nobody |
| **3 Boundary, Linux** | 9–16 | `-sandbox` + `-os` for **Linux**, per §5. Landlock + seccomp + rlimit; escape tests written the same week as the confinement; Property 2's wire split and bounds; Property 7's read-back | `heic→jpg` converts confined; every escape test fails; **latency gate met with 92% headroom, measured** |
| **4 First worker** | 17–22 | `oc-images` (libvips + libheif) properly: cross-compiled, signed, fuzzed, ASan nightly, OSS-Fuzz submitted | A signed artifact CI produces without manual steps |
| **5 Containers** | 23–26 | `AvGraph`, remux, `mkv→mp4` bit-identical | The second flagship conversion, asserted in CI |
| **6 → 1.0** | 27–30 | State layer, batch, manifest, `doctor`, docs, packaging, LGPL artifacts, published threat model | **Ship 1.0.** Real users on one platform |
| **7 Windows** | 31–38 | The Windows boundary, with the four extras the spikes found: `HANDLE_LIST`, the per-job container ACL, profile read-back, and whatever S16 decides about the spawn | Same escape suite green; the receipt reports *engaged*, not *requested* |
| **8 GUI** | 39–45 | Tauri + Svelte, with Property 4's capability ACL from the first commit and the state machine in Rust | a11y and CSP gates green; zero outbound from the packaged app |
| **9 Breadth** | 46–60 | `oc-pdf`, `oc-archive`, `oc-audio`, third platform | Full catalogue, sandboxed |

**Staffing, stated the way your own rule demands:** two engineers through phase 6. **A third from week 27, before 1.0 ships, not month 13** — because SR-12 commits publicly to a 72-hour triage clock and a 7-day patch-or-disable from the day you launch, across eleven native upstreams, and the current plan has nobody to answer it for two months.

---

## 7. How you know it is working

Four indicators, checkable weekly, that do not depend on anyone's judgment:

1. **Mutation score on `openconvert-core` is above threshold and rising.** If it falls, tests are being written to pass rather than to catch.
2. **Every `SR-*` in `requirements.toml` whose week has arrived has a test that ran.** The CI job either passes or it does not.
3. **No gate has been moved to a later week.** If one is, record whether it was ever passable at the old one — this has now happened in three consecutive revisions and it is the project's most reliable early warning.
4. **The number of documented-but-unbuilt claims is falling.** Right now it is high and every revision has raised it. Ship narrower and it falls on its own.

And one that does depend on judgment, but is the most important: **when a review finds a defect, fix the property, not the instance.** Five of the current top findings were introduced by v0.5's remedies for v0.4's findings. That pattern is why four independent reviews scored this design 59–65 across three revisions, and it will continue until the fixes move up a level.

---

## 8. What still needs to be tested

Thirty-three spikes answered a lot. This section is what they did **not** answer, so nothing here is mistaken for coverage. It is ordered by how much damage the unknown can still do.

### 8.1 The three that can still invalidate something

> **All three are packaged as runbooks for another agent: the private platform validation runbooks.** One per platform, with exact commands, expected output, interpretation guidance, error-recovery tables, a deliverable template and copy-paste prompts. Hand them to whoever has the hardware.

| # | Unknown | Why it is the top of the list | Where |
|---|---|---|---|
| **1** | **All of macOS.** `sandbox_init` under a hardened runtime, App Sandbox entitlements, per-worker signing, notarization | The **only** platform with zero measurements. `03 §17` calls it *"the hardest week"* and `08 §3` gives it **one engineer-week at week 13**. Every other platform's estimate has been checked against reality; this one has not. S11 is written and ready | **Needs a Mac** |
| **2** | **`unprivileged_userns_clone=0` on a real hardened Debian** | This is **the case the entire v0.5 isolation-ladder rewrite exists for**. S12b proved Landlock works without a userns on a kernel that has no such restriction — which is necessary but not the same test. The sysctl is not namespaced, so no container can reproduce it | **Needs a real Linux host** |
| **3** | **Confinement on real arm64 hardware** | QEMU emulation runs the binaries but returns `ENOSYS` for the Landlock syscalls, so **emulation validates compilation and logic, never confinement**. Every arm64 security claim is currently unverified | **Needs arm64 hardware or a full-system VM** |

### 8.2 Runnable on this machine — now RUN

Everything that was in this list has been done; the detailed results remain in the private validation record:

| Was pending | Outcome |
|---|---|
| **S30** `cargo-fuzz` | ✅ **7.4M cases, no counterexample.** And the comparison that matters: mutation testing found a vacuous test and dead code that 7.4M fuzz cases did not, because **fuzzing tests the code and mutation testing tests the tests** |
| **S35** realistic worker size | ✅ **300 KiB binary, 15 MiB with 103 shared libs**, linking real libvips |
| **`CreateProcessAsUser`** + AppContainer | ✅ **They compose.** The `Full` row is constructible as written |
| **`cargo-vet`** | ⚠️ **Init exempts all 67 crates.** Green on day one means nothing; it gates every addition from day two. Treat `exemptions` as a debt list and gate the count downward |
| **ASan on a C-linking worker** | ✅ **Builds and runs** against real libvips. §3.1's nightly job is demonstrated |
| **The five property prototypes** | ❌ **Still not built.** That is Phase 1 work, not a spike |

**One of those runs corrected a result I had already published** — see §1.5, finding 3. ACG engages inside an AppContainer; my "silently declines" measurement was taken on a plain process, which is not what ships.

### 8.3 Cannot be tested until product code exists

These are not gaps in the spikes; they are the reason to start Phase 1.

- **Escape tests per tier, per target** — `08 §4` gates them at w14. They need a real worker and a real engine.
- **A real `heic→jpg` under full confinement**, end to end, with a receipt.
- **Class A byte-identity and the Class B SSIM floor** — and the w4 gate that currently has no Class A operation to protect.
- **Prediction p99 < 10 ms.**
- **`openconvert verify`** round-tripping a receipt.

### 8.4 Decisions, not measurements

Nothing will settle these except someone choosing:

- **The target matrix.** Which OS × architecture combinations ship at 1.0, and which get confinement *assurance* rather than only a build.
- **MSRV and the supported-OS floor.** Landlock ≥ 5.13 with runtime ABI negotiation; a Windows build floor for ACG/CIG that, per S17/S21, may not engage anyway.
- **Static vs dynamic linkage of the LGPL engines.** Determines an obligation `02` declares "not owed".
- **Whether `D11` *should* become per-batch.** S16 proves it **can** safely. But revocation removes the handle, not the memory, so a compromised engine can carry bytes across files within a batch. One process per file does not have that residual. **That is a risk-appetite decision, and it belongs in `09 §8` either way.**

### 8.5 Caveats on what *was* tested — read before trusting any number above

| Result class | Caveat |
|---|---|
| Every Linux measurement | Ran on a **Microsoft WSL2 kernel fork** (6.6.87.2), in a container or WSL — **not bare metal**. Landlock and seccomp behaviour should be re-confirmed on a stock kernel |
| Every Windows measurement | **One machine**, Windows 11 **Home**, **not admin**. ACG/CIG declining may be machine-specific; the ADMX corpus on Home is truncated, so the "Restrict App Containers" policy name remains **unverified** |
| arm64 | **Cross-compiled and emulated only.** No confinement mechanism has ever executed on arm64 |
| Every binary | **Unsigned.** No trusted certificate, no notarization. S8 shows signing does not change spawn cost, but nothing else about signed distribution was exercised |
| The `Limits` values | Derived from **six archives on one filesystem**. Sound as a starting point, not as a calibration |
| S16's warm pool | Measured with a **do-nothing worker**. Real engines hold memory and state between jobs; the amortisation will look different with libvips loaded |
| Spawn timings | Vary **2×** run to run on Windows (22–45 ms p50 observed). Treat single numbers as indicative |

---

## 9. The five things to do first

Rewritten after the spikes. Item 1 used to be "run Phase 0"; Phase 0 has largely run, so it is replaced by the one decision its results opened up.

1. ~~Settle `D11` with S16.~~ **DONE — and it works** (2.19 ms/file, grant and revoke both safe inside an AppContainer). The remaining `D11` question is not a measurement but a **risk-appetite decision**: revocation removes the handle, not the memory, so a per-batch worker lets a compromised engine carry bytes between files. **Decide it, write it into `09 §8`, and move on.** In its place, the highest-value next action is **[S11 on a Mac](#8-what-still-needs-to-be-tested)** — the only platform with zero measurements, carrying a one-week estimate for the week the record calls hardest. Original text: The latency gate is missed by 161% on Windows and met with 92% headroom on Linux, and only 4.4 ms of it is the sandbox. Either the threshold becomes per-platform or one process stops meaning one file. **This is the last measurement that can still change the architecture, and everything in phase 3 onward is sized by the answer.**
2. **Add the clippy purity gate and move `InputHandle` out of the core.** One config file and one type relocation. *(The first draft of this plan recommended a `wasm32` build here; S1 proved that does not work — `std::fs` compiles for `wasm32-unknown-unknown` and fails only at runtime. That is what the spike folder is for.)*
3. **Split wire types from domain types, ban `Deserialize` outside `wire.rs`, clamp limits to a policy ceiling — and construct the `SandboxProfile` by read-back** (Properties 2 and 7 together). The first closes the trust inversion both reviews named as the biggest problem; the second stops the receipt claiming a confinement the OS silently declined. S6 already shows the ceiling working: a compromised engine reporting `width = 4_000_000_000` is refused at the wire boundary.
4. **Write `requirements.toml` and the CI job that enforces it.** The gate-timing defect has recurred three revisions running and will not stop until a machine checks it. ACG is the new argument for this: a claim can report success, be recorded as satisfied, and be false.
5. **Cut v1.0 to Linux, one worker, two conversions, CLI only.** The measurements make the platform choice for you (§5). Then ship it — everything else is easier from a position of having shipped.
