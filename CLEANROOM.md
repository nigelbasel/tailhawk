# CLEANROOM.md — provenance record

Tailhawk ships under **MIT OR Apache-2.0**. The closest prior art in this product category is
GPL-licensed: **klogg** (GPL-3.0), **TailBlazer** (GPL-3.0), **SnakeTail** (GPL-3.0). Reading their
implementation source and then writing equivalent code would make the derivative-work question a
real one rather than a theoretical one.

This file is the record that we didn't. It has to stay **ahead of the code** — an entry written
after the fact is worth very little.

**Populated 2026-07-29, before any implementation code exists.**

---

## 1. The rule

1. **Do not read GPL-licensed implementation source** for any component in the register in §4.
2. **Permitted inputs:** published documentation, public specifications, academic papers, vendor API
   reference, textbooks, your own measurements, and the *observable behaviour* of any tool — what it
   does, never how it does it.
3. **Prose is not code.** An algorithm described in a project's README, design doc or header-comment
   prose is usable; the implementation below it is not. Any such use gets logged in §5 with the URL
   and the commit hash it was read at, so the exact text can be re-examined later.
4. **Whoever reads, doesn't write.** Any contributor — human or agent — who has read forbidden
   source for a component is disqualified from writing that component. This is the whole mechanism;
   everything else is bookkeeping.
5. **Log before you code.** The §5 entry for a component is written and committed *before* the first
   line of that component, not alongside it.

## 2. Known contamination

**`docs/RESEARCH.md` §5.3 is contaminated and is reference-forbidden for implementation.**

The section carries its own warning. It cites internal constants that could only have come from
reading GPLv3 implementation source — in the same section that mandates a clean-room process, and
without recording who read what. That combination is why it cannot be relied on: not because the
design is necessarily wrong, but because its provenance is unknown and unreconstructable.

- **Affected component:** the block-sparse line-offset index.
- **Consequence:** the index design must be **re-derived from §3 sources only**, with the
  re-derivation logged in §5, before any index code is written.
- **Do not** implement from §5.3 directly, and do not quote it into a design document that then
  feeds implementation.

`docs/RESEARCH.md` §11 additionally records that klogg, TailBlazer and SnakeTail are all GPL and
that a clean-room rule was needed. This file is that rule.

## 3. Source allow-list

| Source | Licence | Status |
|---|---|---|
| klogg | GPL-3.0 | **Source FORBIDDEN.** Observable behaviour, published docs and README prose permitted, logged. |
| TailBlazer | GPL-3.0 | **Source FORBIDDEN.** Same carve-out. |
| SnakeTail | GPL-3.0 | **Source FORBIDDEN.** Same carve-out. |
| LogExpert | MIT | Source permitted. If code is used or closely followed, attribute it and record it here. |
| BareTail, Hoo WinTail | proprietary | **Observable behaviour only.** No disassembly, no decompilation. |
| Microsoft Learn / Win32 & DirectWrite / D3D11 reference | MS docs terms | Permitted. |
| Unicode, IETF RFC, W3C, OpenTelemetry specifications | open | Permitted. |
| Rust crate documentation and source | per crate | Permitted subject to each crate's own licence; `cargo-deny` governs what we *link*, which is a separate question from what we *read*. |
| Academic papers, textbooks | per publisher | Permitted; cite in §5. |
| Our own measurements and experiments (`PLAN.md` §3, Phase 0) | ours | Permitted, and preferred. |

## 4. Component register

Components are clean by default. Only those listed here carry a restriction.

| Component | Status | Cleared to implement? | Notes |
|---|---|---|---|
| Block-sparse line-offset index | **CONTAMINATED** via `RESEARCH.md` §5.3 | **No** | Requires a §5 re-derivation entry and an attestation before any code. |
| Everything else | Clean | Yes | Add a row here the moment that stops being true. |

## 5. Re-derivation and consultation log

Append-only. Newest last.

| Date | Component | Who | Sources consulted | Notes |
|---|---|---|---|---|
| 2026-07-29 | *(none — file creation)* | Claude Opus 5, session 2 | `docs/RESEARCH.md` lines 456–465 and 493 only — the §5.3 contamination notice and the "GPL hazard" line | **The technical content of §5.3 was deliberately not read.** Only the warning text and the identification of the affected component were consulted, specifically so this agent remains eligible to implement the index under §1.4. |

## 6. Attestation

Before the first line of code for any component marked CONTAMINATED, append an entry below naming
the author, the date, the sources relied on, and an explicit statement that no GPL implementation
source was consulted for that component.

> *(No attestations yet — no implementation code exists.)*

## 7. The dependency allow-list

`cargo-deny.toml` is the counterpart to this file. It governs what Tailhawk **links**; everything
above governs what its authors **read**. Both are needed and neither substitutes for the other — a
clean-room process cannot stop a GPL crate arriving transitively, and a licence scanner cannot stop
someone reading GPL source and writing it out again from memory.

The config carries no comments, so the reasoning lives here:

- **The list is an allow-list, not a deny-list.** Under `[licences] version = 2`, anything not
  explicitly allowed fails the check. GPL, AGPL and LGPL are therefore rejected without being named,
  and so is any licence nobody has looked at yet. A new copyleft licence appearing in the ecosystem
  fails closed. Naming licences to deny would be the weaker construction — it only rejects what
  someone thought to list.
- **MPL-2.0 is deliberately absent.** It is only file-level copyleft and plenty of permissive
  projects accept it, but accepting it means the outbound "MIT OR Apache-2.0" statement no longer
  describes the whole artefact without qualification. If a genuinely necessary dependency turns up
  under MPL, add it to `exceptions` for that one crate with a note here — not to `allow`.
- **`Unicode-3.0` and `Unicode-DFS-2016` are both present** because `unicode-ident` and its
  relatives changed licence between versions, and the resolved graph can legitimately contain
  either.
- **`unknown-registry` and `unknown-git` are `deny`.** A single-file, copy-and-run binary that
  claims no network I/O of its own should not be built from a source nobody can name. This also
  makes an unreviewed git dependency a build failure rather than a code-review question.
- **`multiple-versions` is `warn`, not `deny`.** Duplicate versions in the graph matter here because
  of the 15 MB binary-size gate in `SPEC.md`, but they are usually somebody else's transitive
  problem and blocking on them would be noise. Revisit if the gate gets tight.
- **`yanked = "deny"`** — a yanked crate is a supply-chain signal, and this project has no deadline
  pressure that would justify shipping one.

## 8. Related

- `LICENSE-MIT`, `LICENSE-APACHE` — the outbound licence.
- `cargo-deny.toml` — the config the section above explains.
- `docs/RESEARCH.md` §11 — the record of which research claims were refuted, including the licensing
  ones.
