# CLAUDE.md — working on Tailhawk

Tailhawk is a Windows log viewer: a portable Rust core, a Win32 shell, a D3D11 + DirectWrite
renderer. `SPEC.md` §2.1 scopes v1 to Windows 10 1809+ — **Linux and macOS are not on the plan.**

## Read these first, in this order

| File | What it is |
|---|---|
| `docs/HANDOFF.md` | **The resume point is the top section.** Current state, what is next, what is knowingly left undone. Read it before anything else and keep it current. |
| `docs/SPEC.md` | The contract. Behaviour is settled here, not in code review. |
| `CLEANROOM.md` | The provenance record and the rule that governs how components may be written. |
| `docs/UI-DESIGN.md` | The screen: layout, keys, the surfaces V-numbers refer to. |
| `docs/PLAN.md` / `docs/EFFORT.md` | Estimates, and actuals scored against them. |

## The rules that have actually been broken

**Log before you code.** `CLEANROOM.md` §1.5: the §5 provenance entry for a new core module is
written and committed *before* the module's first line. This has slipped five times. CI's
`provenance` job now fails the build when a file in `crates/tailhawk-core/src/` is named nowhere in
`CLEANROOM.md` — but that gate only catches a *missing* entry. It cannot tell you the entry came
late, or that it is accurate. Write it first.

**Feed the activity log from the first turn.** `./tools/agentlog.sh <LEVEL> <kind> "<message>"` —
kinds in use: `turn`, `task`, `commit`, `test`, `docs`, `ci`, `push`, `note`, `idle`. The owner
tails `logs/agent.log` to see what you are doing. A session that works silently is a session the
owner cannot supervise.

**Read the log before you write to it.** `tail -30 logs/agent.log` at the start of every turn, after
any pause, and before every commit. Writing to a log you never read is bookkeeping; reading it is
what makes it memory. Session 21 wrote a `note` naming every file it had created, then ten minutes
later could not account for one of those files and went to the transcripts to identify it — the
answer was already in the log, four lines up.

**Log every file you create or substantially rewrite, at the moment you do it — naming the file.**
The log is not only the owner's window in; it is *your own memory*, and the only part of this
session that survives compaction. Session 21 wrote `menu.rs` — 658 lines and 20 tests — logged
nothing, lost it when the conversation was compacted, then found it in `git status` looking like
someone else's work and recorded it in `CLEANROOM.md` as the **owner's** module. A one-line
`task` entry naming the file would have made that impossible. Write enough detail that you can
reconstruct what you did from the log alone, because at some point that is all you will have.

**An unexplained file in `git status` is investigated, never attributed by inference.** `git log`
shows the owner's name on every commit — that is the configured identity, not evidence. Check
`logs/agent.log` first, then this project's transcripts:
`grep -o '"file_path":"[^"]*NAME"' ~/.claude/projects/C--dev-git-TailHawk/*.jsonl`.

**Stage named paths. Never `git add -A` or `git add .`** — this project commits straight to master,
so a swept-in file and a wrong `CLEANROOM.md` row publish in the same step.

**Commit straight to master, and often.** No branches, no PRs on this project. Push, then check CI
and *wait for the result* — do not push and walk away. Sessions have ended leaving master red.

**Review each component with a subagent before committing it.**

**Don't write backslash-heavy source through a heredoc.** `cat`/`perl` heredocs halve `\` — write
`\`, `\0` and regex-bearing Rust with an editor tool instead.

## Building and checking

```
cargo fmt -p tailhawk -p tailhawk-core -- --check
cargo clippy --release -p tailhawk -p tailhawk-core -- -D warnings
cargo test --release --workspace -- --nocapture
```

CI runs `deny`, `provenance`, then build + clippy + fmt + test on **x64 and arm64**, plus a binary
size gate and assertions that the binary carries no runtime shader compiler and no CRT
redistributable dependency. `cargo fmt --check` has broken the build on its own more than once.

The `tools/verify-*.ps1` harnesses drive the real binary. They need **`powershell`, not `pwsh`**,
and a desktop session. `tools/verify-uia.ps1` is the exception — it needs no foreground.

**A known flake:** `semantic::tests::a_screenful_costs_a_fraction_of_a_frame` fails under the full
parallel suite on a loaded machine and passes alone and on CI. It is a timing criterion. Rerun it
alone before blaming a change for it.

## Style

No inline comments explaining internal logic — the code says what it does, doc-comments say what a
thing is for and why it is shaped that way. Match the surrounding prose register in the docs: they
are written to be read, not skimmed.
