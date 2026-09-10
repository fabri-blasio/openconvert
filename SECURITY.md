# Security policy

OpenConvert exists to open files that may be hostile. Security reports are the
most valuable contribution this project can receive, and they are treated that
way.

**Nothing is shipped yet.** This policy is live from week 1 so that the first
report has somewhere to go, rather than arriving before the process exists.

## Reporting

Email **security@openconvert.dev** — or, until that address is live, open a
[GitHub security advisory](https://github.com/fabri-blasio/cloudconvert/security/advisories/new),
which is private to the maintainers.

Please do **not** open a public issue for a vulnerability.

Include what you have. A rough report today beats a polished one next month:

- What you did, and what happened
- The file that triggered it, if you can share it safely
- Version, OS, and the isolation tier the receipt recorded

## What we commit to

These are the numbers from [SR-12](docs/spec/09-THREAT-MODEL.md#5-requirements-to-tests),
and they are the same commitment sold to paying customers:

| | |
|---|---|
| Acknowledgement | **72 hours** |
| Patch or disable, for an actively-exploited engine CVE | **7 days to published** |
| Reaching a default-configured install | **7 days from publication**, via the weekly manifest |
| Credit | Yours, unless you ask otherwise |
| Embargo | Whatever you need, within reason. We will not publish before you are ready |

The engine kill switch exists from day one. If an engine is being exploited and
no patch is ready, we can disable that conversion path on every install rather
than asking users to wait.

## Scope

**In scope.** Anything that lets a converted file affect a machine beyond the
job directory: sandbox escapes, path traversal, destruction of an existing file,
network egress from a conversion, a receipt that misrepresents what ran, or a
state file that escalates rather than degrades.

**Known and documented, so not a finding on its own.** Everything in
[09-THREAT-MODEL §8](docs/spec/09-THREAT-MODEL.md#8-what-we-dont-protect-against) — a full
exploit chain through the sandbox stack, a compromised OS, hardware side
channels, mark-of-the-web failing open, cross-file data carried within one batch
under the default `worker_reuse` setting.

**We would still like to hear it** if you can show one of those is worse than we
described, or that a mitigation we named does not hold.

## Our own mistakes

This project publishes measurements that contradict its own earlier claims —
see [DESIGN-DELTAS.md](docs/spec/DESIGN-DELTAS.md), where four corrections are to things
we had already published. A report that shows a security claim in our documents
is false is exactly as welcome as one that shows a bug in the code.
