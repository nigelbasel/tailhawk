# Relay — two machines, one master

The owner works on Tailhawk from more than one machine, each with its own Claude session. The
sessions talk to each other through this file, by commit. Nothing else carries a message between
them: neither session can see the other's conversation, scratchpad, session note or `logs/agent.log`
(it is gitignored). **Anything one needs the other to know is written here, or in `HANDOFF.md`, and
pushed.**

## The baton

**Holder: the other machine** — handed over by `osprey` on 2026-11-09. Write its hostname here
when it takes the baton.

Only the holder commits code. Two sessions committing to one master without branches will collide,
and a rebase over someone else's half-finished change is how work gets lost. The machine without
the baton reads, answers here, and does not touch the source. The baton moves by a relay entry
that says so, and by editing the line above in the same commit.

## How a session uses this file

1. **At the start of every turn**, `git pull --rebase` and read any entry below newer than the
   last one you wrote or acknowledged. Also read `git log --oneline -15`: the other session's code
   commits are messages too.
2. **To say something**, append an entry at the bottom, never edit another's:

   ```
   ## 2026-11-09 19:05 — osprey → <other host>
   <what you need them to know, what you need from them, and what you will do meanwhile>
   ```

   Use `hostname` for the names and the local time with the date. Name files by their path inside
   the repository: the two checkouts are not at the same place on disk.
3. **Commit it alone**, `git add docs/RELAY.md`, with a subject starting `Relay:` and ending
   `[skip ci]`. Every push to master otherwise runs CI and **publishes a release**; a message is
   not a release. A code commit is never `[skip ci]`. **GitHub reads the skip from the newest
   commit of a push**, so a relay commit pushed on top of unpushed code skips CI for the code
   too: push the code first and wait for its CI, or leave `[skip ci]` off that one entry.
   **And GitHub finds the token anywhere in the message, body included.** A code or docs commit
   whose message merely *mentions* the token — `dadde95`, describing this protocol — runs no CI
   at all; write "the skip marker" in prose instead.
4. **Push at once**, and if the push is rejected, `git pull --rebase` and push again. Entries only
   ever append, so the rebase is clean unless both wrote at the same moment; then keep both.
5. **Acknowledge** an entry that asks for something, even if only to say when you will get to it.

Neither session is notified of a push. The owner tells a session to look, or it looks at the start
of its next turn; a session waiting on the other says so to the owner rather than polling GitHub.

## Entries

## 2026-11-09 19:00 — osprey → the other machine

The owner is moving Tailhawk work to your machine. You hold the baton from this commit on; osprey
stops committing code.

**Start with the top resume point in `docs/HANDOFF.md` (2026-11-09).** It says what is built,
what is next, and what is waiting on the owner. In short: Go to source, increment 1 —
`crates/tailhawk-core/src/frames.rs` (finding stack frames and deciding whether their file may be
opened) and `crates/tailhawk/src/editor.rs` (the editor presets and the command line) are
committed, reviewed and mutation-checked; the shell work that uses them is next and is not
started.

Things only osprey knew, now written down here because nothing else carries them:

- The workstream's tail log on osprey was `C:\Users\nigel\.claude\logs\tailhawk-9e.log`. Use the
  same slug, `tailhawk-9e`, on yours so the owner's habit carries over.
- Mutation checks were run by small Python scripts that mutate a file in memory, run the named
  test by its full path (`frames::tests::<name>`, `editor::tests::<name>`), require that it ran
  and failed, then write the saved text back and assert it is identical. They lived in osprey's
  scratchpad and are not in the repository; write your own the same way. Two traps from them:
  pass `encoding="utf-8"` to `subprocess.run` (cargo's output is UTF-8 and the Windows default
  decoder dies on it), and never "restore" with `git checkout --`, which discards every
  uncommitted change in the file.

osprey will read this file when the owner next asks it to. Answer here if anything above is
unclear.
