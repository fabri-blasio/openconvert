# OpenConvert — Threat Model

What we are defending against, what enforces each defence, and which test fails the build when it rots.

Status: design record, **v0.2 — output-side, host-surface and identity revision** · 2026-08-13
Part 9 of 9 — see [README](README.md). Architecture: [03-ARCHITECTURE](03-ARCHITECTURE.md).

---

## Why this document exists separately

Until v0.4 the security requirements lived inside [05-RESEARCH](05-RESEARCH.md) — a document describing an architecture that has since been replaced. When the architecture was simplified, controls that had been specified were dropped without anyone noticing, because nothing connected the requirement to the design that was supposed to satisfy it. Resource limits and the degraded-sandbox fallbacks both vanished that way, and one attack class had never been written down at all.

So the requirements live where the architecture can point at them, every one has an ID, and **every ID has a test that fails the build at a week that test can actually pass.** [§5](#5-requirements-to-tests) is the traceability table; it is the part of this document that does actual work.

### What changed in v0.6

**This revision is written against measurement rather than reasoning** — 41 spikes run, 19 predictions refuted, change order in [DESIGN-DELTAS.md](DESIGN-DELTAS.md). What moved here:

| # | Change | Evidence |
|---|---|---|
| 1 | **§7 step 7 rewritten.** `expansion_ratio` was named first and cannot separate a bomb from a disk image (1029× vs 1028×) | S23 |
| 2 | **SR-15's lint widened from one destructive API to eleven.** `set_len(0)` and `fs::rename` are reachable past a ban on `File::create` | S4, S25 |
| 3 | **SR-1's test corrected.** It asserted on socket *creation*, which stays permitted in an AppContainer; and our own first run connected to nothing | S9 |
| 4 | **SR-11 now requires read-back.** The receipt records what engaged, not what was requested — an error we found running in *both* directions | S17b, S20b, S21 |
| 5 | **Three new entries in §8**: batch-worker memory residual, our own copy stripping mark-of-the-web, and one archive extracting differently on three filesystems | S16, S26, S13 |
| 6 | **§9 gains mutation testing, advisory scanning and dependency review**, and scopes the reproducible-build claim | S29–S33 |
| 7 | **One finding of our own withdrawn.** A prior review rated `O_EXCL` weakening on non-local filesystems at MEDIUM-HIGH; it held across a 9p boundary to NTFS and the finding is refuted | S27 |
| 8 | **The audit brief corrected** — `-os` now holds an invariant | — |

> **Nothing about macOS changed.** It has zero measurements; [§4](#4-per-os-implementation-and-fallbacks)'s macOS column is unverified and marked as such. The runbook is the private macOS validation runbook.

### What changed in v0.2

A second adversarial review found that the taxonomy was still **inbound and in-process**: it modelled bytes arriving and engines running, and not the three places where the product touches the world outside that frame.

| # | Gap | Now |
|---|---|---|
| 1 | **Every traversal control correctly passed a member named `.bashrc`** — it is not traversal, it is destruction. Undo could not restore it, because undo deletes outputs and the file destroyed was an input | **A13** · **SR-15**. `O_EXCL` everywhere, no truncating open in the codebase, conflict policy resolved before opening |
| 2 | **Nothing tied the bytes routed on to the bytes converted.** Three separate reads of one path, so the receipt could attest to bytes never touched | **A14** · **SR-16**. `FileFacts` owns the handle; untrusted provenance is copied first |
| 3 | **The Windows shell extension ran our parser in `explorer.exe`** — on by default in v1, our only code outside our own process, and absent from this document, the file schema and every gate | **A15** · **SR-17**. Cut from v1; when it lands it opens no file at all |
| 4 | **SR-9's egress gate ran against a CLI with no HTTP stack and was never re-run** after a webview joined the product 30 weeks later | **A15** · **SR-18**. A GUI-layer egress gate and a mandatory CSP |
| 5 | **Persistent state was undesigned**, so nothing said whether a hand-edited history file could influence a security decision | **A16** · **SR-19**. [03 §12](03-ARCHITECTURE.md#12-state-on-disk): every file untrusted on read, none can enable anything |
| 6 | **SR-1 said "ever" while the default floor admitted profiles that could not deny egress** | The floor is three predicates and `network: Denied` has no config key ([03 §9.1](03-ARCHITECTURE.md#91-the-floor-is-three-predicates-not-one-number)) |
| 7 | **SR-2's named enforcement did not exist** — "`Isolation` on every `Step`", on a `Step` that had no such field | I10. `Isolation` is a required field ([03 §7.1](03-ARCHITECTURE.md#71-illegal-states-dont-compile)) |
| 8 | **SR-5, SR-11 and SR-13 were gated at weeks their tests could not pass** — archives, sandbox profiles and extraction did not exist yet | Split into staged rows with honest weeks |
| 9 | **The `Reduced` tier could not be reached** in the case it existed for, so hardened machines were refused | Ladder re-derived; Landlock needs no user namespace ([§4](#4-per-os-implementation-and-fallbacks)) |
| 10 | **The traversal gate would have skipped on fork pull requests**, because its corpus needed a CI secret | The blocking corpus is **generated in-repo**; fetched exploit samples are a separate nightly ([§9](#9-security-engineering-practices)) |

---

## Contents

1. [The core problem](#1-the-core-problem)
2. [Evidence base](#2-evidence-base)
3. [Attack classes](#3-attack-classes)
4. [Per-OS implementation and fallbacks](#4-per-os-implementation-and-fallbacks)
5. [Requirements to tests](#5-requirements-to-tests)
6. [Controls mapped to threats](#6-controls-mapped-to-threats)
7. [The worst realistic attack, walked end to end](#7-the-worst-realistic-attack-walked-end-to-end)
8. [What we don't protect against](#8-what-we-dont-protect-against)
9. [Security engineering practices](#9-security-engineering-practices)

---

## 1. The core problem

A converter's job description *is* the attack: **take a file from an untrusted source and run a complex, memory-unsafe parser over it.** Format parsers are among the highest-density sources of memory-safety bugs in the open-source ecosystem. Moving conversion from a cloud service to the user's laptop *transfers the blast radius to the user*.

So **"local" without "sandboxed" is a downgrade in safety, not an upgrade.** The privacy pitch obligates the security architecture. This is the single most important framing in the project.

### Trust boundaries

```
  ┌─ UNTRUSTED ──────────────────────────────────────────────────────┐
  │  input bytes · file names · archive entry names · declared        │
  │  extensions · engine process memory · our own state files         │
  │  (history, config, recipes, journal — see A16)                    │
  └────────────────┬─────────────────────────────────────────────────┘
                   │  parsed at the boundary (never re-checked after)
  ┌────────────────▼─────────── SEMI-TRUSTED ────────────────────────┐
  │  Sniff · Properties · FileFacts · OutputName · DestinationName    │
  └────────────────┬─────────────────────────────────────────────────┘
                   │
  ┌────────────────▼─────────── TRUSTED ─────────────────────────────┐
  │  openconvert-core (pure)  ·  openconvert-sandbox (the broker)         │
  │  openconvert-os (syscalls) ·  the copy-out path                     │
  │  the update verifier                                              │
  └───────────────────────────────────────────────────────────────────┘

  ┌─ OUTSIDE OUR PROCESS ─── (A15) ──────────────────────────────────┐
  │  the desktop webview (v1)  ·  the shell extension (v1.1)          │
  │  — hostile input reaches these before any control above engages   │
  └───────────────────────────────────────────────────────────────────┘
```

**The copy-out path is trusted code handling untrusted names.** That is where v0.3 was exploitable to traversal and where v0.4 was still exploitable to destruction. **The fourth box is where v0.4 had nothing at all** — the two surfaces that are not our process and therefore not covered by anything in the first three.

---

## 2. Evidence base

A sample, not a survey. ⚠️ marks a figure from a secondary source.

**ImageMagick** — the canonical "conversion library as RCE vector":

| CVE | Nature | Note |
|---|---|---|
| CVE-2025-57803 | 32-bit integer overflow in the BMP encoder scanline-stride computation → heap corruption | CVSS 9.8 ⚠️; explicitly flagged as dangerous in **auto-convert pipelines** — i.e. exactly what we are building |
| CVE-2025-55298 | Format-string flaw: `InterpretImageFilename` passes user-controlled input to `FormatLocaleString` | **Crafted filenames** are an input vector, not just file contents |
| CVE-2025-68618 | Uncontrolled recursion in the SVG parser via deeply nested elements → DoS | Any service ingesting user SVG |
| CVE-2025-54418 | Command injection via a handler passing filenames/params into a shell | **The integration is the bug, not the library** |

**Ghostscript** — CVE-2024-29510: format-string injection in the `uniprint` device that **escapes the `-dSAFER` sandbox**, enabling command execution and file I/O. Exploited in the wild using **EPS files disguised as JPGs**. Shipped alongside CVE-2024-29506/29507/29509 (buffer overflows), CVE-2024-29508 (pointer leak), CVE-2024-29511 (arbitrary file read/write). Ghostscript sits underneath ImageMagick, LibreOffice and GIMP, so "I don't use Ghostscript" is usually false.

**FFmpeg** — CVE-2025-1373 (use-after-free in the MOV parser), plus 2025 issues in the ALS decoder, the Firequalizer filter, the HLS implementation, and a heap overflow in the JPEG 2000 decoder. There is **no official Rust rewrite**. Note also: Rust *bindings* to FFmpeg confer no memory safety — safe-looking Rust can still trigger a UAF in the C below.

**Zip Slip** — a name for the class, not a single CVE: archive members whose stored path escapes the extraction root. It has recurred continuously across languages and libraries since at least 2018, including in libraries whose authors believed they had fixed it. `libarchive` does **not** sanitise by default; `ARCHIVE_EXTRACT_SECURE_NODOTDOT`, `SECURE_SYMLINKS` and `SECURE_NOABSOLUTEPATHS` must be set explicitly.

**Zip Slip's quieter sibling** — the same archives commonly carry members named for files that already exist. There is no CVE class for this because it is not a memory-safety bug and not an escape; it is simply that most extractors truncate on collision by default, and the file that was there is gone. It reached the v0.4 design unmitigated for exactly the reason A12 reached v0.3 unmitigated: the model was about *where* a write lands, not *what was already there*.

---

## 3. Attack classes

| # | Class | Concrete example | Primary control | Requirement |
|---|---|---|---|---|
| **A1** | Memory corruption in decoders | MOV UAF, BMP overflow, JP2 heap overflow | OS sandbox per engine; pure-Rust fast paths for common formats; continuous fuzzing per adapter; ASLR/CFG/CET on all shipped binaries | SR-2 |
| **A2** | Parser sandbox escape | Ghostscript `-dSAFER` bypass | Never trust in-engine sandboxes; wrap in an OS-level one; Ghostscript excluded from core entirely | SR-2 |
| **A3** | **Command injection via filenames or parameters** | ImageMagick handler shelling out; `InterpretImageFilename` | **No shell anywhere.** `EngineBin` is a closed enum; `Argv` takes no program string; **engines receive pre-opened file descriptors and no name at all** | SR-3 |
| **A4** | External reference / SSRF / local file read | ImageTragick delegates, SVG `<image href>`, XXE in Office XML, PDF remote resources | Network denied at the **sandbox** level and structurally required by the floor; delegates disabled; entity resolution off; SVG via resvg | SR-1 |
| **A5** | Resource exhaustion | Zip bombs, image bombs, billion-laughs XML, SVG recursion, 500-megapixel PNG | `Limits` on **every** step, both paths, checked *during* extraction against a running total; plus a per-job `Budget` so 40,000 legal files cannot fill a disk | SR-5 |
| **A6** | Polyglot / type confusion | EPS masquerading as JPG | Content sniffing; multi-signature detection → **quarantine and ask**; route strictly by detected type; mismatch in the receipt and the UI icon | SR-4 |
| **A7** | Active content survival | Office macros, PDF JavaScript/OpenAction, SVG `<script>` | Strip on ingest; rebuild from a representation that cannot express executable content; Paranoid mode adds a pixel bottleneck *(post-v1)* | SR-10 |
| **A8** | Metadata leakage | GPS in EXIF, author/company in Office, revision history, thumbnails retaining redacted content | Metadata policy per profile; `Share` strips GPS/serial/author/history/thumbnails; redaction removes objects; the receipt lists exactly what was removed | SR-11 |
| **A9** | Model supply chain | Pickle checkpoints execute code on load; malicious hub uploads rising ⚠️ | safetensors/GGUF/ONNX only; **pickle rejected at the file-type layer**; pinned hashes; signed model index | SR-8 |
| **A10** | Module supply chain | 27 malicious VS Code extensions in 2024 → 105 in 2025 ⚠️ | *No module system in v1.* When one exists: enforced capability manifests, unreviewed = no native code and no network | SR-6 |
| **A11** | Update channel compromise | A malicious update pushed to all users | Signed updates, transparency log, staged rollout, no silent capability expansion; the revocation list rides **inside** the signed manifest | SR-7 |
| **A12** | **Path traversal & arbitrary file write** | `../../../.ssh/authorized_keys`, absolute paths, a symlink entry, `evil.exe.`, `file.txt:payload.exe` | **`OutputName` parses exactly one path component.** Traversal is unrepresentable. Symlink, hardlink, device and FIFO entries refused. Checked inside the sandbox *and* in the trusted copy-out | SR-13 |
| **A13** | **Destructive write to an existing file** | An archive member named `.bashrc`, `id_rsa`, `resume.docx` or `Thumbs.db`; a naming template that collides across a batch; a re-run that silently replaces yesterday's output | **Every create is `O_EXCL`/`CREATE_NEW`.** No truncating open exists in `-sandbox` or `-run`; a source scan gates it. `Policy::on_conflict` (default `Suffix`) resolves *before* opening | **SR-15** |
| **A14** | **Input substitution between decision and execution** | A file on a sync folder or network share replaced after sniffing but before decoding: routed as PNG, decoded as something else, hashed as a third thing — and the receipt attests to bytes that were never converted | **`FileFacts` owns the handle it was parsed from.** One open, all reads through it. Untrusted-provenance, network and removable-volume inputs are **copied** into the job dir before routing | **SR-16** |
| **A15** | **Hostile input reaching a host surface outside our process** | Our COM DLL parsing a header inside `explorer.exe`; the desktop webview loading a remote origin; a crafted filename rendered into the UI | **Nothing that runs outside our process opens a file.** The shell extension is cut from v1 and, when it lands, reads no bytes. The webview ships a strict CSP with no remote origin, gated in CI | **SR-17, SR-18** |
| **A16** | **Local state tampering** | A hand-edited or corrupted `config.toml`, `history.jsonl`, `recipes/*.toml` or `quarantine.json` used to lower confinement, re-enable a revoked engine, or crash the app on launch | Every state file is **untrusted on read**, parsed and fuzzed; none can *enable* anything — `quarantine.json` may only disable, `config.toml` cannot touch `network: Denied`, history feeds only target ranking | **SR-19** |

**A13, A14, A15 and A16 are new in v0.2**, and they share a shape worth naming: each is a place where the product touches something *other than a file being converted* — the destination directory, the passage of time, the operating system's own UI, and its own state. The taxonomy modelled the conversion and not the program around it.

---

## 4. Per-OS implementation and fallbacks

`SandboxProfile` ([03 §9](03-ARCHITECTURE.md#9-isolation-the-profile-and-the-floor)) is a **sealed value** describing what actually engaged on this machine. `available()` in `openconvert-os` probes once at startup and caches; it is the only code permitted to construct one (I11).

### The floor is three predicates

```
network:    Denied     — NOT LOWERABLE. There is no config key. SR-1 says "ever".
filesystem: Confined   — default; explicit opt-out exists and is loud
strength:   Reduced    — default; "Elevated" in the UI requires Full
```

A profile that cannot deny network yields **no executable steps**, regardless of policy. That is what makes SR-1's "ever" a true word rather than an aspiration, and it is asserted by a property test over every profile the prober can produce.

### The ladder

v0.4's `Reduced` tier required a bind-mount namespace, which requires a user namespace — exactly what is missing in the cases it listed. **Landlock requires neither privilege nor a namespace**, only `no_new_privs` and kernel ≥ 5.13. The ladder below is derived from what is actually reachable unprivileged.

| Strength | Linux | macOS | Windows |
|---|---|---|---|
| **Full** | Landlock path rules + seccomp-bpf allowlist + **empty netns** + cgroup v2 (mem/cpu/pids) + `no_new_privs` + user namespace | App Sandbox + `sandbox_init` in the worker, **no `com.apple.security.network.*` entitlement**, hardened runtime, notarized, `setrlimit` | **AppContainer** (low-privilege SID) + restricted token + **Job Object** + **no network capability SID** + ACG/CIG/DEP/CFG |
| **Reduced** | **Landlock + seccomp-bpf (network denied at the syscall filter) + `setrlimit` + `no_new_privs`. No user namespace needed.** | *(none — App Sandbox is present on every supported release)* | AppContainer + Job Object, without ACG/CIG |
| **Minimal** | seccomp-bpf + `setrlimit` + `no_new_privs`, **no filesystem confinement** | — | restricted token + low integrity + Job Object, **no network denial** |
| **Meets the default floor?** | Full ✅ · Reduced ✅ · **Minimal ❌** (filesystem) | Full ✅ | Full ✅ · Reduced ✅ · **Minimal ❌** (network) |
| **Typical cause of a drop** | kernel < 5.13 → no Landlock → Minimal. `unprivileged_userns_clone=0` (Debian), unprivileged container, AppArmor userns restriction → **Reduced, which converts** | — | ACG incompatible with an injected AV DLL → Reduced. Managed-desktop policy blocking AppContainer → Minimal |

**Principle: never silently degrade, and never refuse when the truth would have served.** The hardened-Debian case — the one v0.4 named as its motivation and did not actually fix — now converts at `Reduced`, with Landlock confining the filesystem and seccomp denying the network, and both named in the plan preview and the receipt. The cases that still refuse are the ones that genuinely cannot satisfy SR-1 or SR-2, and the message names the missing mechanism and the user's options.

Files carrying mark-of-the-web or a quarantine attribute raise the floor to `Full` automatically. **That detection fails open** — see [§8](#8-what-we-dont-protect-against). It is a useful signal, not a guarantee, and it is described as one.

**The browser is a profile too**, with the caveat stated rather than glossed: `BrowserOrigin` confines the page, not `image-rs` from the rest of the page's memory, so it scores `Reduced` and never `Full` ([03 §9.4](03-ARCHITECTURE.md#94-the-profile-absorbs-wasm--with-the-honest-caveat)).

---

## 5. Requirements to tests

**Every requirement has a test. Every test fails the build, at a week it can actually pass.** Weeks refer to [08-EXECUTION-PLAN §3](08-EXECUTION-PLAN.md#3-weeks-544--outcomes).

v0.4 gated SR-5, SR-11 and SR-13 at week 4, when archives, sandbox profiles and extraction did not exist. A gate that cannot pass at its stated week gets moved on discovery, which restarts exactly the drift this table exists to stop. Staged rows are the fix.

| ID | Requirement | Enforced by | Test | Gate from |
|---|---|---|---|---|
| **SR-1** | No network egress from any conversion process, ever. **Two mechanisms, because there are two paths.** For a `Sandboxed` step: a profile that cannot deny network yields no executable steps ([03 §9.1](03-ARCHITECTURE.md#91-the-floor-is-three-predicates-not-one-number)). For an `InProcess` step there is no third-party parser to confine — I10 and the format table guarantee it — so the binding instrument is SR-9's single-call-site scan and `net::no_outbound_during_conversion`. *(v0.6.1: this row previously stated the profile rule unscoped, which contradicted `03` §9.1 and, implemented literally, refused a pure-Rust PNG conversion on any machine without a probed profile. Found by running the CLI.)* Network is a separate, explicitly user-initiated subsystem with pinned endpoints and hash verification. | The floor's non-lowerable `network: Denied` + OS sandbox (netns / entitlement / capability SID / seccomp) | `route::no_steps_without_network_denial` (property, **w2**) · `escape::network_denied` — **connect to a listener the test itself is running** over TCP, UDP and DNS, plus a unix socket, from inside an engine, **once per reachable profile tier** (w14). The assertion is on *reachability*, never on socket creation: socket creation stays permitted inside an AppContainer and restriction is enforced at connect/send, so a test that asserts creation fails proves nothing. A connect to nothing returns `TimedOut` in every configuration, confined or not — both mistakes were made in our own first attempt at this spike (S9) | **w2** / w14 |
| **SR-2** | Every third-party parser runs `Sandboxed`. No engine that is not pure safe Rust runs in-process. | **`Isolation` is a required field of `Step` (I10)**; the engine registry declares each engine's minimum | `plan::every_step_has_isolation` (property, **w2**) · `route::no_unsafe_engine_inprocess` (property, **w2**) · `escape::filesystem_denied` (w14) | **w2** / w14 |
| **SR-3** | No shell. Every process launch names its program with `EngineBin` and passes an argument vector. Engines receive pre-opened descriptors, never names. | `EngineBin` closed enum; `Argv` private fields (I7) | `lint::no_command_new_outside_sandbox` (source scan) + a compile-fail test asserting `EngineBin` cannot be built from a string | w5 |
| **SR-4** | File type is determined by content, never by extension. Mismatches are surfaced. Polyglots are quarantined. | Two-phase detection ([03 §11](03-ARCHITECTURE.md#11-detection-in-two-phases-and-input-identity)) | `detect::routes_by_content` — a PostScript file named `.jpg` routes as PostScript and the receipt records both | w4 |
| **SR-5** | Every step carries hard limits — wall, CPU, RSS, output bytes, temp bytes, expansion ratio, decode pixels, archive depth/entries/total — enforced on **both** paths. Every *job* carries a `Budget` so a legal batch cannot fill a disk. | `Limits` required on `Step` (I4); `Budget` on `Policy` | `plan::every_step_has_limits` (property, w2) · **`wire::engine_facts_cannot_widen_limits` (property, w2)** · `limits::pixel_bomb_inprocess` (w4) · `budget::batch_respects_free_space` (w4) · `limits::zip_bomb` + `limits::nested_archive` (**w24, when `oc-archive` exists**) | w2 / w4 / **w24** |
| **SR-6** | *(Deferred with the module system.)* Modules declare capabilities; the runtime enforces them; undeclared access fails closed. | — | — | on module trigger |
| **SR-7** | All models and updates are signed; artifacts are content-addressed and hash-pinned; the trust root ships with the app. | Update verifier | `update::rejects_unsigned` · `update::rejects_wrong_hash` · `update::rejects_downgrade` | w43 |
| **SR-8** | Model formats restricted to safetensors / GGUF / ONNX. Pickle rejected at the file-type layer. | Format allowlist in the model loader | `models::pickle_rejected` | on AI trigger |
| **SR-9** | Zero telemetry. **Exactly one default-on network call exists: the weekly update-and-revocation manifest fetch** — contentless, identifier-free, previewable, disableable, and the reason SR-12 works. No other component polls anything. Diagnostic uploads are manual and show the exact payload. | A single audited call site; everything else has no outbound path | `net::single_outbound_call_site` (source scan, w4) · `net::no_outbound_during_conversion` — a full conversion makes zero connection attempts (w4) · **`net::gui_no_outbound`** (w35) | w4 / **w35** |
| **SR-10** | *(Paranoid mode deferred.)* Output produced through a rebuild retains no structure from the source parser. | — | — | on Paranoid trigger |
| **SR-11** | Every output has a receipt recording engines, versions, parameters, class, limits, **isolation and the profile that engaged — read back from the OS in the child, never the profile that was requested** ([03 §9.5](03-ARCHITECTURE.md#95-the-profile-is-read-back-not-requested)), the `content_id` of the bytes actually converted, and what metadata was removed. A receipt that cannot be written fails the step. | `Receipt` built from the execution log | `receipt::complete` (property: every executed step appears, w4) · `failure::receipt_unwritable` (w4) · **`receipt::records_reduced_profile` (w14)** · **`isolation::readback_rejects_unengaged` — request a mitigation known to be declined on the test machine and assert the profile does not claim it (w14)** | w4 / **w14** |
| **SR-12** | Security response is published: triage < 72 h, patch-or-disable for actively-exploited engine CVEs < 7 days **published**, reaching a default-configured install within 7 days of that via the SR-9 manifest. Engine kill switch from day one. | Process + the revocation list inside the signed manifest | `update::revocation_disables_engine` · `update::revocation_applies_offline` (a cached list still disables) | w43 |
| **SR-13** | **An engine or archive member cannot cause a write outside the job directory.** Entry names parse to exactly one path component; symlink, hardlink, device and FIFO entries are refused; checked inside the sandbox and again in the trusted copy-out. | `OutputName` / `DestinationName` / `BrokeredOutput` (I6) | `traversal::names` — **generated in-repo**: `../`, absolute, UNC, drive-relative, deep-nested, ADS, `CON`, trailing dot, Unicode-normalisation collision, and a member whose name is valid but whose parent chain escapes. Plus a fuzz target on `OutputName::parse` (w6) · `traversal::extraction` — the same corpus through real extraction (**w24**) | w6 / **w24** |
| **SR-14** | **A killed, crashed or cancelled job leaves no orphaned temp data and no partial output.** Cancellation is bounded. | `WorkerSession` owns the job dir; lock file + startup sweep ([03 §8.4](03-ARCHITECTURE.md#84-the-worker-session-and-how-many-files-one-worker-sees)) | `lifecycle::kill_mid_conversion` · `lifecycle::cancel_bounded` (< 500 ms) | w6 |
| **SR-15** | **A conversion cannot destroy a file that already exists.** Every create is `O_EXCL`/`CREATE_NEW`; **no destructive filesystem API is reachable** from `openconvert-sandbox` or `openconvert-run`; conflicts are resolved by policy before anything is opened. | I12 + `Policy::on_conflict` | `lint::no_destructive_fs_api` (source scan over **eleven** APIs — [03 §7.4](03-ARCHITECTURE.md#74-impossible-hard-forbidden); a scan for truncating *opens* alone missed seven live routes, incl. `set_len(0)` and `fs::rename`) · `traversal::collision` — an archive whose members collide with pre-existing files in the destination leaves every original intact · `conflict::suffix_skip_fail` | **w4** |
| **SR-16** | **The bytes routed on are the bytes converted, and the bytes the receipt attests to.** One open per input; untrusted-provenance, network-volume and removable-volume inputs are copied before routing. | One open handle in `-run`'s `HandleTable`, addressed by the `InputToken` in `FileFacts`; `content_id` computed once (I13) | `identity::rename_swap_defeated` — replace the file by rename between detect and execute; the conversion uses the original bytes and the receipt matches · `identity::untrusted_is_copied` | **w6** |
| **SR-20** | **A file of untrusted provenance never shares a worker process with another file.** `worker_reuse` may be raised by any policy layer and lowered by none. | `route()` groups dispatch by `Provenance`; `Policy::worker_reuse` has no lowering setter — the same pattern as the network floor ([03 §8.4](03-ARCHITECTURE.md#84-the-worker-session-and-how-many-files-one-worker-sees)) | `policy::worker_reuse_never_lowers` (property, **w2**) · `dispatch::untrusted_never_shares_a_worker` (property, **w2**) · `lifecycle::isolated_spawns_once_per_file` (w6) | **w2** / w6 |
| **SR-17** | **Nothing that runs outside our own process opens a file.** The shell extension is not in v1. When it ships it is out-of-process where the OS allows, reads no bytes, predicts from extension + sibling listing + history only, marks its suggestion provisional until the app has sniffed, and catches every panic at the FFI boundary. | Design constraint + a source scan over the extension crate for any file-open API | `lint::shell_ext_opens_nothing` (source scan) · `shell_ext::panic_never_crosses_ffi` | **v1.1** |
| **SR-18** | **The desktop application makes no network request.** A strict CSP with no remote origin; every asset local; no analytics; no remote font, image or script. | `tauri.conf.json` `csp` + a config lint + an OS-level socket assertion around the packaged app | `config::csp_is_strict` (lint) · `net::gui_no_outbound` — drive a full conversion through the packaged GUI and assert zero connection attempts | **w35** |
| **SR-19** | **No file we persist can enable a capability.** State is untrusted on read; a corrupt or hostile state file degrades, never escalates, and never crashes the app. | [03 §12](03-ARCHITECTURE.md#12-state-on-disk) rules 1–4 | `state::corrupt_degrades` — a truncated, hostile and hand-edited variant of each state file · `state::config_cannot_lower_network_floor` · `state::quarantine_can_only_disable` · fuzz targets on each parser | **w6** / w33 |

Requirements marked deferred have no test because they have no implementation; each is tied to a named, **observable** trigger in [03 §15.3](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires). **Deferred is not the same as missing** — but it is written down, which is the difference between a decision and an oversight.

---

## 6. Controls mapped to threats

| Threat | Controls |
|---|---|
| **A1** memory corruption | `Sandboxed` for every third-party parser, enforced by I10; pure-Rust in-process fast paths for PNG/JPEG/WebP/GIF/BMP/TIFF, WAV/FLAC, CSV/JSON; a fuzz target per adapter, each a CI gate; per-engine crash quarantine (three crashes on a format → that path auto-disables locally with a visible reason); ASLR/CFG/CET on shipped binaries |
| **A2** sandbox escape | Never rely on an in-engine sandbox — `-dSAFER` taught us; Ghostscript excluded from core; defence in depth, since a compromised engine still faces Landlock, seccomp and a cgroup, and holds no descriptor it was not handed — **which requires an explicit inherit list, because `std::process::Command` otherwise leaks every inheritable handle in the parent (64 observed, 2 writable — S3)** |
| **A3** command injection | `EngineBin` is closed, so no program is nameable by string; `Argv` has private fields; **engines receive pre-opened descriptors and no filename at all**, which closes the `InterpretImageFilename` class more completely than renaming did; parameters are typed values; a source scan asserts `Command::new` appears nowhere outside `openconvert-sandbox` |
| **A4** external refs / SSRF | The sandbox has **no network capability**, and a profile that cannot guarantee that yields no steps; delegates and risky coders disabled; XML entity resolution off; SVG through resvg; PDF remote-resource fetching off |
| **A5** resource exhaustion | `Limits` on every step; in-process limits checked **before allocation** from the header; archive limits against a running total *during* extraction; a per-job `Budget` checked between steps; streaming decode throughout |
| **A6** polyglot | `infer` + `tree_magic_mini` + table-driven magic signatures; multi-signature detection → quarantine and ask; routing strictly by detected type; the mismatch in the receipt, the plan preview, and the file-type icon |
| **A7** active content | Strip on ingest; the output is rebuilt rather than passed through; Paranoid mode adds the pixel bottleneck when it lands |
| **A8** metadata leakage | Policy per profile; `Share` strips GPS/serial/author/history/thumbnails; redaction removes objects and re-linearises; the receipt lists what was removed. **First testable in w3**, when the JPEG metadata strip lands ([08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates)) — before that this control had a policy and no test |
| **A9** model supply chain | safetensors/GGUF/ONNX only; pickle rejected at the type layer; signed index; sha256 verified on every load; ONNX custom-op allowlist |
| **A10** module supply chain | No module system in v1. When it lands: WASM-only for unreviewed modules, no network, no subprocess, capability-diff prompt on update |
| **A11** update channel | Signed manifest + transparency log; staged rollout; capability-diff approval; the revocation list travels inside the signed manifest, so there is no second channel to compromise; the fetch is contentless, so the channel leaks nothing but the existence of an install |
| **A12** path traversal | `OutputName::parse` makes traversal unrepresentable; no symlink/hardlink/device/FIFO entries; `O_NOFOLLOW` on create; post-canonicalisation containment assert; checked in the sandbox *and* in the copy-out |
| **A13** destructive write | **`O_EXCL` is the only create.** A lint over **eleven** destructive APIs gates the two crates that write files — `File::create` alone left seven live routes, and the two sharpest (`set_len(0)` through a non-truncating handle, and `fs::rename`, which is *the* idiomatic atomic write) no scan for truncating opens would find; `Policy::on_conflict` resolves before opening; the receipt records the name actually written; `Fail` aborts the whole step rather than leaving a half-extracted tree |
| **A14** input substitution | One open per input, held by `-run`'s `HandleTable`; share-deny-write on Windows; copy-before-route for untrusted provenance and volatile volumes; `content_id` computed through the same handle the conversion reads |
| **A15** host surfaces | Shell extension not in v1, and when it ships it opens nothing and runs out-of-process where the OS allows; the webview ships a strict CSP with no remote origin, every asset local, gated in CI at the packaged-app level rather than the library level |
| **A16** local state tampering | Every state file untrusted on read and fuzzed; parse failure degrades rather than aborts; state can only ever *disable* (`quarantine.json`) or *rank* (`history.jsonl`), never enable or confine less; the network floor has no config key at all |

---

## 7. The worst realistic attack, walked end to end

v0.4 walked a hostile archive against the outbound path, and that walk now stops at four independent points. So this one attacks the surface the taxonomy did not model: **the passage of time, and the destination directory.** It needs no memory-safety bug and no sandbox escape.

**Setup.** A designer keeps a shared OneDrive folder with a contractor. The contractor's account is compromised. Nothing on the designer's machine is.

| Step | What happens | What stops it |
|---|---|---|
| 1 | Attacker places `assets.zip` in the shared folder. It syncs. No mark-of-the-web — it arrived by sync, not by browser | **Nothing.** MOTW fails open, which [§8](#8-what-we-dont-protect-against) states plainly. Floor stays at the default `Reduced` |
| 2 | Designer drops `assets.zip` on OpenConvert to convert its contents | — |
| 3 | Sniff — pure Rust, in-process, decides ZIP and `Sandboxed` | A6 · SR-4 |
| 4 | **The file sits on a sync volume.** SR-16 classifies it as a volatile volume and the broker **copies it into the job dir before routing** | **A14 · SR-16.** In v0.4 this file would have been sniffed, re-opened by the engine, and hashed — three reads, any of which the sync client could have replaced mid-flight. The receipt would have attested to bytes nobody converted |
| 5 | Attacker replaces `assets.zip` mid-conversion via sync | Too late; we hold our own copy and the `content_id` is already fixed |
| 6 | Machine offers `Reduced` (Landlock + seccomp, no userns — a container host) | **Converts, and says so.** In v0.4 this machine fell to `Minimal` and was refused ([§4](#4-per-os-implementation-and-fallbacks)) |
| 7 | Bomb attempt: 4 KB → 1 GB, nesting depth 40 | Running totals trip **`archive_depth` (32) and `archive_total_bytes`** **during** extraction · A5 · SR-5. **`expansion_ratio` is not what stops this and v0.5 named it first** — a legitimate 200 MB zeroed disk image compresses **1029×** and this bomb **1028×** (S23), so no threshold separates them. Absolute size and depth do the work; the ratio is a logging tripwire set at ≥10,000. **libarchive imposes no limit of its own** — 512 MB extracted from a 522 KB archive with no error |
| 8 | Traversal attempt: `../../../../Startup/x.lnk` | `OutputName::parse` rejects on the separator · A12 · SR-13 |
| 9 | Symlink entry `link → ~/.ssh`; `evil.exe.`; `report.txt:run.exe`; `CON` | Refused as symlink, trailing dot, ADS separator, reserved device name · A12 · SR-13. Note the reserved-name rule protects **the C engines and downstream tools**, not our own Rust, which uses verbatim paths — only `NUL` actually vanished in testing (S13) |
| 10 | **Members named `.gitconfig`, `Brand-Guidelines.pdf`, `logo.ai`** — every one a legal single path component, every one a file already in the destination | **A13 · SR-15.** `create_new` is `O_EXCL`; `on_conflict` defaults to `Suffix`, so they land as `.gitconfig (2)` and the originals are untouched. **This is where v0.4 lost the designer's work** — every traversal check passed these, correctly, because they are not traversal |
| 11 | Engine is compromised anyway and writes directly | Confined to the job dir by Landlock; holds no descriptor outside it · A2 · SR-2 |
| 12 | Copy-out — the trusted path | Every name re-parsed as a `DestinationName`, independently of step 8, and every create is `O_EXCL` · A12, A13 |
| 13 | Engine crashes mid-extraction | `StepOutcome::Crashed` recorded honestly; no partial output copied out; job dir removed · SR-14 |
| 14 | Process SIGKILLed | Lock released; next launch sweeps · SR-14 |

**Where it stopped in v0.4:** step 10, nowhere — with a plausible archive containing no traversal at all. And step 4 was a silent correctness failure rather than an attack: the receipt, which is the product's second pillar, could describe a conversion of bytes that no longer existed.

**Residual risk:** a memory-corruption exploit in libarchive that achieves code execution *and* a working escape from AppContainer or the Landlock+seccomp combination. That is a real chain, it is what [§8](#8-what-we-dont-protect-against) covers, and it is why the engine set is kept to four workers and the fuzzing is per-adapter and gated.

---

## 8. What we don't protect against

Published verbatim on `/security`. Dangerzone does this, and it earns more trust than any marketing claim.

| Not covered | Why |
|---|---|
| **A full exploit chain through the sandbox stack** | A memory-corruption bug plus a working kernel or AppContainer escape defeats us. We reduce the odds — four workers, memory-safe fast paths, no Ghostscript, per-adapter fuzzing, a kill switch — but we do not claim immunity |
| **A compromised operating system** | If the OS or a kernel driver is already hostile, nothing in user space helps |
| **Hardware side channels** | Spectre-class attacks across the sandbox boundary are out of scope |
| **Internet-provenance detection** | Mark-of-the-web and the macOS quarantine xattr **fail open.** A file that reached the disk via sync, a non-NTFS volume, a USB stick, an archive extracted by another tool, or a copy operation that drops the attribute looks local — because the OS has told us nothing else. The automatic floor raise is a useful signal, not a guarantee, and a user who knows a file is untrusted should set the floor themselves |
| **Data carried between files in one batch by a compromised worker** | Under the default `worker_reuse: Balanced` ([03 §8.4](03-ARCHITECTURE.md#84-the-worker-session-and-how-many-files-one-worker-sees)) one worker process serves several files that share a trusted provenance. A capability is revoked between files — verified, including inside an AppContainer — but **revocation removes the handle, not the memory.** An engine compromised by file A can retain A's bytes and write them into the output of file B in the same batch. Files of *untrusted* provenance never share a worker — **asserted by SR-20, because a mitigation named in this table and tested nowhere is not a mitigation** — and `worker_reuse: Isolated` removes the residual entirely at ~38 ms per file on Windows. **The receipt records which files shared a worker**, so this is disclosed per conversion and not only here |
| **Our own copy not preserving mark-of-the-web** | Untrusted-provenance inputs are copied before routing, and the mandated `create_new` path **strips `Zone.Identifier`** (S26). `Provenance` is therefore captured at `detect()` before any copy exists and never re-derived from the copy ([03 §11.3](03-ARCHITECTURE.md#113-input-identity)). Any future code path that re-reads provenance from a job-dir copy would silently see a local file |
| **One archive extracting to different files on different platforms** | **NTFS does not fold NFC/NFD; APFS does; ext4 does not** (S13, S24). The same archive containing both forms of one name yields two files on Windows and Linux and one on macOS. We preserve names byte-for-byte and treat NFC-equal names as a collision, which is the best available behaviour — it does not make the outcome identical across platforms, and for a product whose second pillar is telling you what happened, that is worth stating |
| **In-place modification of an input we are reading** | We hold one read handle per input, which defeats replace-by-rename. A process that already has write access to the same inode can still modify it underneath us. Untrusted-provenance and volatile-volume inputs are copied first; local trusted inputs are not, because copying every byte of every conversion is a real cost |
| **Semantically harmful but structurally valid content** | A perfectly well-formed PDF containing a fraudulent invoice converts cleanly. We check structure, not truth |
| **The user's own destination choices** | Converting to a network share or a synced folder sends the output there. We refuse to overwrite; we do not refuse to write |
| **Malicious models the user fetches deliberately** | A hostile ONNX graph from an untrusted source is a risk the user has chosen. The persistent indicator says so |
| **Correctness of third-party engines** | libvips can produce a wrong image without being compromised. The receipt records what ran; it does not certify the result |
| **A user who has already lost their account** | Every file in [03 §12](03-ARCHITECTURE.md#12-state-on-disk) is writable by the user, so an attacker who is the user can edit them. A16's controls stop those files being an *escalation* path, not an access path |
| **Physical access and shoulder-surfing** | Out of scope |

---

## 9. Security engineering practices

- **Continuous fuzzing, and each target is a CI gate with a week.** The sniffer, the route table, `OutputName::parse`, `DestinationName::parse`, receipt parsing, recipe parsing, each state-file parser, the worker protocol in both directions, and **one target per engine adapter** — the last being the highest-value set, because it is the code that hands attacker bytes to C libraries. v0.4 promised these and put none of them in the gate table, which by its own rule made them prose. They are rows now ([08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates)).
- **Mutation testing, because fuzzing does not test the tests.** `cargo-mutants` over `openconvert-core`, a CI gate from w2 at a rising threshold (80% to start). This is not redundant with the list above: run against a parser that had already passed 24 evil-corpus cases and 4,000 property cases, it found **10 of 30 mutants surviving** — including a corpus test that **passes vacuously when its own input is emptied**, and an unreachable branch. Over the same period `cargo-fuzz` ran **7.4 million cases** against that parser and found nothing (S29, S30). **Fuzzing tests the code; mutation testing tests the tests**, and a threat model that leans on a test list this long needs the second.
- **The blocking corpus is generated in-repo.** `corpus/src/evil.rs` builds every traversal, collision, ADS, device-name and normalisation case at test time from checked-in code. This matters for a reason that is easy to miss: a corpus fetched from an encrypted archive needs a CI secret, and secrets are unavailable to pull requests from forks — so the SR-13 and SR-15 gates would have **skipped, showing green, on exactly the contributions least likely to have been reviewed.** Real exploit samples still never enter the repository; they run in a separate nightly job that is allowed to be secret-dependent because it is not the gate.
- **Dependency inventory and CVE watch** on every bundled engine, with a per-engine "days since upstream release" dashboard. Four workers rather than ten engines is what makes this affordable. A standing operational cost; budget for it.
- **Three licence surfaces, three gates:** `cargo-deny` for Rust crates, `engines.toml` for native engines (licence + **three-valued `linkage`** — `subprocess` / `linked-into-worker` / `linked-into-host` — plus **`link_mode: static | dynamic`**, because *that* is where the LGPL boundary actually falls: the wall is between our host and the worker, while libheif is **inside** the worker. **CI fails any LGPL engine that is not `dynamic`** ([03 D19](03-ARCHITECTURE.md#16-decisions))), `models.toml` for model weights. CI asserts every engine and model referenced in code appears in its table. See [02-FEATURES §11](02-FEATURES.md#11-model-licensing-policy) for what the LGPL obligation actually is — it is bounded, not absent.
- **Advisory scanning and dependency review — new in v0.6, because neither existed.** The bullet above covers *licences*; nothing in the record covered *vulnerabilities*. `cargo-audit` runs as a gate, **with `--target` filtering**: a live advisory was found in `postcard → heapless → atomic-polyfill` — `postcard` being the crate [03 §5.2](03-ARCHITECTURE.md#52-the-hostworker-protocol) names for the worker protocol — but it applies only to a target we never build, and appears solely because `Cargo.lock` is target-independent (S33). Without the filter this gate cries wolf on day one, and a gate that cries wolf acquires an `--ignore` flag within a month. `cargo-vet` is added with its `exemptions` file treated as a **debt list** and a CI check that the count only ever decreases — `cargo vet init` grandfathers every existing crate, so a green first run is green by construction.
- **Reproducible builds, scoped to what is actually achievable.** A rebuild **in the same container image and the same build path** is byte-identical with `SOURCE_DATE_EPOCH` and `--remap-path-prefix`; a build from a *different root* is not (S31). The attestation promises the former, which is verifiable and useful; it does not promise bit-identical output across arbitrary build roots, which the two flags everyone quotes do not deliver. **The C engine builds are a separate problem** — pinned and hashed, not bit-reproducible. This has a week and a gate in [08 §4](08-EXECUTION-PLAN.md#4-the-ci-gates), because [04 §10.2](04-GROWTH.md#102-the-paid-boundary) sells it.
- **Published threat model** (this document), `security.txt`, coordinated disclosure policy, and a bug bounty even if small.
- **Third-party audit before 1.0** of `openconvert-sandbox` *and* `openconvert-os`, with the distinction made explicit to the auditor: the first holds most invariants and is `#![forbid(unsafe_code)]`; the second holds the syscalls, process creation, and — **since v0.6** — **one invariant of its own**, because it constructs the `SandboxProfile` by reading back what actually engaged (I11, [03 §9.5](03-ARCHITECTURE.md#95-the-profile-is-read-back-not-requested)). v0.5 briefed the auditor that `-os` holds no invariants; that is no longer true and the brief must say so. Together they are the boundary, and together they are small — which is what makes the audit affordable and its result meaningful. Publish it.
- **A "what we don't protect against" page** ([§8](#8-what-we-dont-protect-against)), kept current — including the two entries added in v0.2, which are the ones a competitor would have left out.
