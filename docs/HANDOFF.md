# Handoff — resume here

**Paused:** 2026-07-28, end of session 1.
**Everything below is on disk. Nothing is held in a chat session.**

---

## Where things stand

The project is **Tailhawk** (command `tailhawk`) — a Windows desktop log tailer/viewer.
Research, specification, UI design and development plan are **complete and adversarially reviewed twice**.
**No code has been written.** Nothing has been committed to git; `C:\dev\git\WinTail` is not yet a repo.

### The four documents

| File | State |
|---|---|
| `docs/RESEARCH.md` | Complete. §11 records every claim critics refuted. §12 lists the gating experiments. |
| `docs/SPEC.md` | Complete. §16 traces both review rounds. §17 lists open decisions. |
| `docs/UI-DESIGN.md` | Complete. Phase-tagged `[v1]`/`[v2]`/`[v3]` throughout — **`SPEC.md` §15 is authoritative on phasing, not this document.** |
| `docs/PLAN.md` | Complete. **v1 = 81.5 person-weeks (~19 months part-time).** |

### Decisions locked in

- **Name: Tailhawk.** Chosen for the "watch like a hawk" idiom. Tagline: *watch your logs like a hawk*.
- **Stack:** Rust + windows-rs + D3D11 + DirectWrite. Shared core owns the whole grid *including rendering*; thin shell owns window/input/IME. Per-platform leaf backends for rasterisation and presentation.
- **Windows-only v1.** Cross-platform stays possible via the leaf-backend seam but is unpromised.
- **No memory mapping**, ever — a section handle blocks the writer's log rotation.
- **Polling is the correctness mechanism** for following; change notification is an accelerator that may fail silently.
- **Record model is OTel-shaped**, extended with `raw`/`format_id`/`parse_state`.
- **Merged timeline + trace correlation stay v2** (owner-confirmed).
- **Personal project**, personal GitHub, no employer identity anywhere in the repo.
- **Licence: MIT OR Apache-2.0.**

---

## Resume here tomorrow

### 1. Finish the Loki source design *(was in flight when we paused)*

A workflow was running and was **stopped cleanly, not completed**. Re-run it — the script is saved:

```
Workflow({ scriptPath: "C:\\Users\\nigel\\.claude\\projects\\C--dev-git-WinTail\\c8d47c59-98d0-40f0-86b8-bcd47f6558a3\\workflows\\scripts\\tailhawk-loki-source-wf_08f0220c-eba.js" })
```

> `resumeFromRunId` is **same-session only**, so tomorrow this is a fresh run. Expect ~35–45 minutes.
> **Note:** this session exhausted its 200-call WebSearch budget. A new session resets it. The script
> already instructs agents to use WebFetch against direct URLs rather than search.

**What it is answering:**
- The Loki HTTP API surface a desktop client needs (`query_range`, the `tail` WebSocket, label/series metadata endpoints, server-side limits).
- Auth models — self-hosted, `X-Scope-OrgID` multi-tenancy, Grafana Cloud, and querying *through* Grafana's datasource proxy. **Owner deferred the auth decision to this research.**
- How a query API maps onto a viewport contract built for seekable byte streams. **This is the architectural crux:** "line 4,182,995" and jump-to-line have no meaning in Loki, so the viewport may need to become cursor-based. If that change is small the design is clean; if large, it complicates the grid for the local-file case, which we do not want.
- Which filter chips can be pushed down into LogQL vs evaluated client-side.
- Two hostile critics: product scope (does this make a worse Grafana?) and security (**credential storage is the hard problem** — config files are shareable and may sit on a read-only network share; DPAPI is user+machine scoped so a config carried to another machine breaks).

**When it lands:** fold into `SPEC.md` §4 as a source kind, add a costed component to `PLAN.md` §2.3b, and assign a phase. Note the project currently has **no HTTP client dependency at all**, against a 15 MB CI size gate.

### 2. Then, in order

1. **Resolve the code-signing route** (`PLAN.md` §7.1). Azure Artifact Signing is **ruled out** — its eligibility covers neither South African organizations nor individuals. Confirm **Certum Open Source Code Signing** eligibility for a South African individual (from €69, hardware token shipped internationally). **Not a blocker for v1** — ship unsigned via scoop; it blocks the first *signed* release only.
2. **Check `tailhawk.com` / `.dev` / `.io`** at a registrar. Never verified.
3. **Claim namespaces:** GitHub org `tailhawk`, crates.io, scoop, winget. Verified free during naming, but that decays.
4. **Run Phase 0** (`PLAN.md` §3) — 4 weeks, seven experiments. **G1 (SMB stale size)** and **G4 (colour-glyph atlas composition)** are the two that can change the architecture.
5. **`git init`** and make the first commit. Add `LICENSE-MIT`, `LICENSE-APACHE`, `CLEANROOM.md`, `cargo-deny.toml`.

---

## Dogfooding — first runnable build

The owner has nominated **two real logs currently in daily use** as the first dogfood targets. The
build does not have to be good; it has to open these two files and follow them. Treat this as the
acceptance gate for "a first version that can actually run".

Actual paths are **deliberately not recorded in this repo** — they sit inside an employer source
tree and their content contains customer names. They are held in the private project memory
(`~/.claude/projects/C--dev-git-WinTail/memory/`). See the identity note below.

| | Corpus A | Corpus B |
|---|---|---|
| Size | ~450 KB | ~3.3 MB |
| State when sampled | idle — batch writer had exited | **live, actively appended** |
| Encoding | ASCII / UTF-8, no BOM | **UTF-8, no BOM, with non-ASCII** (U+2014 em dash) |
| Format | single, log4net-shaped | **heterogeneous — three formats in one file** |

**What each one actually tests:**

*Corpus A* — log4net layout, `yyyy-MM-dd HH:mm:ss,fff LEVEL␠␠Logger.Name message`. Note the
**comma** as the millisecond separator and the padded level field. Straight columnisation exercise
plus open-a-completed-file.

*Corpus B* is the valuable one, and it breaks several assumptions:

1. **BOM-less UTF-8 with non-ASCII content.** Windows PowerShell 5.1's `Get-Content` renders the em
   dashes as `â€”` — i.e. a CP1252 default gets this file wrong. This is a ready-made regression
   test for encoding detection; the correct answer is already known.
2. **Three formats in one file.** A banner line, then a **tab-delimited section with its own header
   row** (`TIME⇥INSTANCE⇥STAGE`) carrying **time-only timestamps** (`17:00:02`, no date), then later
   a different layout entirely (`yyyy-MM-dd HH:mm:ss␠␠[tag]␠␠message`). This confirms `format_id` and
   `parse_state` must be **per-record, not per-source** — already the model in `SPEC.md` §4, now with
   a real specimen behind it.
3. **Time-only timestamps.** `SPEC.md` has **no rule** for a record whose timestamp carries no date.
   Inherit the date from the last fully-dated record? Leave `timestamp` unset and rely on
   `observed_timestamp`? **Open — needs a decision.**
4. **Non-monotonic timestamps.** An `07:14:14` line appears *after* an `09:14:14` line. Nothing may
   assume within-source ordering. This matters most for the v2 merged timeline.

**What they do not test:** both are single-digit MB. Neither exercises the multi-GB path, the
block-sparse index, rotation, or UNC latency. A large file and a rolling set are still needed
separately.

**Identity flag — resolve before the first public commit.** Corpus A lives in an employer repo and
its log lines contain customer names; Corpus B names customer instances. This runs straight into the
locked-in "no employer identity anywhere in the repo" decision and into open question 2 below
(employment IP). Dogfooding against them privately is fine. **Never** paste sample lines, paths, or
screenshots from either into the repo, an issue, or a release note without scrubbing.

---

## Traps — do not rediscover these

Recorded because each cost real effort to find, and two of them were caught only by adversarial review.

| Trap | Where |
|---|---|
| **The index depends on encoding.** Decode *must* precede index — chunk boundaries need code-unit alignment, and `0x0A` is a legal DBCS trail byte. The first plan drafted them 13 weeks apart and would have thrown away tested work. | `PLAN.md` §4 |
| **`RESEARCH.md` §5.3 is GPL-contaminated.** The line index must be **re-derived from published docs only**, with `CLEANROOM.md` populated *before* writing code. Do not implement from that section. | `RESEARCH.md` §5.3 |
| **Serilog and NLog roll to *new filenames* by default.** Neither copy-truncate nor rename-and-recreate describes it. Rolling sets (§5.5b) are v1 for this reason. | `SPEC.md` §5.5b |
| **Every ripgrep throughput figure in circulation is fabricated.** The cited article publishes timings only. The search performance target is deliberately unset. | `RESEARCH.md` §11 |
| **"Beat BareTail's 2 MB" is unwinnable** and retired — D3D11+DXGI+DirectWrite costs 30–60 MB before reading a byte. The claim is *flatness*, not absolute size. | `SPEC.md` §11.2 |
| **`FILE_SHARE_DELETE` is mandatory.** Without it we block the writer's own rotation — a real, reported bug in klogg. | `SPEC.md` §5.1 |
| **Never poll file size by path** — NTFS only replicates size to the directory entry on last-handle-close, so a path stat on a live log returns a frozen size forever. | `SPEC.md` §5.4 |
| **Filtering is not O(viewport).** Every filter change is a full-file pass. Debounce, stream, and require Enter on network paths. | `SPEC.md` §7.3 |
| **Continuations collapsed by default**, and no `--wrap` in v1 — both destroy the O(1) `u64` scroll model, and MEL Simple logs are multi-line *by default*. | `SPEC.md` §6.4 |
| **Accessibility is the only automated UI-test surface** for a custom-drawn grid. The minimal chrome provider is v1 for that reason, not for compliance. | `SPEC.md` §14.1 |
| **egui's f32 scroll diagnosis is unconfirmed** — the issue reporter explicitly says so. Marked `[L]`, with G7 added to actually diagnose it. | `RESEARCH.md` §3.4 |

---

## Open questions the owner still needs to answer

1. **Certum eligibility** for a South African individual — needs a direct question to Certum.
2. **Employment IP position.** If any of this gets built on work equipment or work time, many contracts assign IP. Worth establishing **before the first public commit**, not after the repo has contributors.
3. **Reference perf machine** — must be fixed before any `[TBM]` target in `SPEC.md` §11.3 becomes a number.
4. **Hoo WinTail hands-on (G6)** — the owner's installed copy is the only reliable source for which of its features are actually used in a week, and for how its encoding detection really behaves.

---

## Session artefacts

Workflow transcripts and scripts, if any reasoning needs re-checking:

```
C:\Users\nigel\.claude\projects\C--dev-git-WinTail\c8d47c59-98d0-40f0-86b8-bcd47f6558a3\
  workflows\scripts\      ← re-runnable workflow scripts
  subagents\workflows\    ← per-agent transcripts and journal.jsonl
```

Seven workflows ran this session: competitor/tech/format research, cross-platform, the agentic-native-UI
thesis, naming (two rounds), OpenTelemetry, the four-artifact adversarial review, and the stopped Loki run.
