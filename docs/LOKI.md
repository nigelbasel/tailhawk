# Loki as a Tailhawk source — design note

**Status: research complete, adversarially reviewed, folded into `SPEC.md` §1.3 and `PLAN.md` §2.4
on 2026-08-24** — a month late, and only after the stale §1.3 non-goal had been quoted back to the
owner as the current position. The cost reconciliation §8 owes `PLAN.md` is still outstanding.
The two blocking product decisions were settled by the owner on 2026-07-29 — **Tailhawk is a Loki
client, staged client-lite first** (§8). One design decision (§4) affects v1 and lands regardless.

Produced 2026-07-29 by a 4-agent workflow: two researchers (API surface; mapping onto the existing
source/record/filter model) and two hostile critics (product scope; credential and network
security). Both critics re-verified the researchers' numbers rather than accepting them — see §5.

> **Scrubbing note.** The research was grounded against a live OTLP-native Loki 3.5.2 deployment
> reachable from this workstation. Service names, hostnames, session identifiers, datasource UIDs
> and label values observed there are **deliberately omitted from this document** — same rule as the
> dogfood corpus in `HANDOFF.md`. Shapes and measurements only.

---

## 1. The architectural crux, and its answer

The question the workflow existed to answer: *"line 4,182,995" has no meaning in Loki. Does the
viewport contract have to become cursor-based, and does that damage the local-file case?*

**Answer: no — and the mechanism to avoid it already exists in the spec.**

Loki has no ordinal, no cursor, no continuation token, and no honest total. `/loki/api/v1/index/stats`
returns an `entries` count that the docs describe as an approximation excluding ingester data,
computed probabilistically, which may double-count — it cannot size a scrollbar. Worse, nanosecond
timestamps are **not unique**: a measured 200-entry page spanning 694 ms held 199 distinct
timestamps, so a timestamp-only page cursor provably drops or duplicates rows at every boundary. A
correct cursor is the triple `(timestamp_ns, stream_fingerprint, intra_batch_sequence)`.

But `SPEC.md` §4.2 already turns a non-seekable, consume-once stream into a seekable one with a
block-sparse index: **the stdin spill file**. A Loki source materialises fetched pages into exactly
that spill. The §5.3 line index, §7.3 filter passes, §7.4 chunked search, §10.2 export, column sort
and the virtualised grid then run **unchanged**. The cursor triple, the LogQL and the paging all
live inside the source, below the §3.1 seam — the grid never sees a cursor and never sees time.

Spill the pages as **CLEF NDJSON**. `SPEC.md` §6.3 Stage 2 already short-circuits on `{` + `"@t":`,
so the spill is self-describing and the whole columnisation/filter/search/sort/bookmark/export
pipeline works with zero new code. Cheapest decision in the design.

The alternative — paging the grid directly from the network against a cursor — simultaneously breaks
the scrollbar, the match counter, sort, cross-file search and export. Rejected.

Consequence to write down honestly: "line 4,182,995" means the 4,182,995th record **in the
materialised window**, not in the store. The scrollbar measures the window. Jump-to-line works
inside the window and becomes jump-to-time outside it.

## 2. What the record model gets, free

The mapping is near-1:1, and better than any local log format Tailhawk parses. On an OTLP-native
deployment, verified present as structured metadata: `severity_number` (OTel 1–24 banding),
`severity_text`, `observed_timestamp`, `trace_id`, `span_id`, `scope_name`, plus per-record
properties.

- **Timestamps are the best input the product will ever get** — nanosecond, UTC, from the emitting
  process. §8.3's entire normalisation table (RFC 3164 year inference, log4net local-time guessing,
  Serilog-console-has-no-date) simply does not apply.
- **`level >= Warning` pushes down exactly** — `| severity_number >= 13`; §6.2's universal error
  predicate becomes `| severity_number >= 17`. Both server-side, both numeric. Genuinely lucky, and
  it generalises to any OTLP-native Loki.
- **§9 trace correlation works with zero extra work**, which argues for landing Loki near §9.

Where the model leaks:

1. **`resource` cannot be per-source constants.** A selector spanning several services makes
   `service_name`/`app` vary per row. Rule: `resource` = labels invariant across the source's
   **stream selector**, computed once at connect time from `/series`; everything else becomes an
   `attributes` entry and hence a column. Computed from the selector, not the returned page, so it
   does not flicker while scrolling. Suppress `__stream_shard__` and all `__`-prefixed labels.
2. **Format detection must become per-partition.** One Loki source genuinely mixes formats across
   services in a way no local source does. Run §6.3 Stages 2–3 on the **body only**, accept results
   only for fields Loki did not supply, and key the detector on `app`/`service_name`. When detection
   fires on a body carrying a redundant timestamp/level prefix, strip it from display — a real
   quality win over Grafana Explore.
3. **`raw` becomes subtly false.** Loki's `max_line_size` defaults to 256 KB and structured metadata
   is capped at 64 kB / 128 entries, so upstream truncation is possible and undetectable. Document
   `raw` for a Loki record as **"as received from Loki"**, not "as emitted". Keep `raw` = the log
   line bytes with the label map held separately, or "copy raw" pastes JSON.

## 3. Wire format — the docs are wrong for this deployment

The HTTP API reference documents `values` entries as 3-element tuples `[ts, line, {metadata}]`. That
is the **push** format. In `query_range` **responses** on 3.5.2, structured metadata is merged into
the `stream` object and `values` entries are 2-element. Because metadata differs per record, **each
stream object holds exactly one entry** — measured at `limit` 50, 200 and 1000.

Measured payload split at `limit=1000`: **491,250 bytes of label JSON against 90,290 bytes of actual
log lines — 5.4x amplification.** Independently corroborated:
`totalStructuredMetadataBytesProcessed` 71.2 MB of `totalBytesProcessed` 112.3 MB.

Therefore: **key and repeated-value interning is mandatory, not an optimisation**, and the parser
must be a streaming serde `Visitor`, never a DOM parse — a DOM parse allocates a `HashMap` per
record and breaks §11.2's memory-flatness claim on the first page. The parser must handle **both**
2- and 3-element tuples by probing, never by assuming.

## 4. The v1 decision that cannot wait

Two additions to the viewport contract, **~0.5 PW now versus a ~3 PW structural rewrite after the
grid ships.** They should enter the v1 source model even though no Loki code ships in v1:

1. **`ExtentState` per end of a source:** `Bounded | MoreAvailable(anchor)`. Reaching an end whose
   state is `MoreAvailable` calls `source.extend(end, budget)` — the identical path already used for
   "the file grew". A local file is `Bounded` at BOF and either at EOF depending on follow.
2. **`RowId(u64)`** — opaque, dense and monotonic *within the materialised window*. For a local file
   it is literally the line number, so nothing changes.

`ExtentState` is a prerequisite for §8.3's bounded-window scrollback anyway. Add a "remote query
source" row to §4's table now, listed but unimplemented.

## 5. Three research claims that did not survive re-verification

This is why the critics were worth running.

| Claim | Verdict |
|---|---|
| "An HTTP stack threatens the 15 MB CI gate, so hand-write WinHTTP." | **False — measured.** Four binaries built locally. At `opt-level=3`/LTO/strip: full `reqwest + tokio + tokio-tungstenite + native-tls + serde_json` costs **+1.68 MB** over baseline; at `opt-level="z"`/`panic=abort`, **+0.77 MB**. Against a 15 MB budget. The byte argument for WinHTTP is dead, and with it the +1.5 PW it justified. |
| "Route A is `reqwest + rustls`." | **Disqualified for a different reason neither researcher found.** `rustls` resolves to `ring`, whose build script invokes `cl.exe` through `cc-rs` — it failed to build on this machine at `curve25519.c`. That directly contradicts the zero-C-dependency position `SPEC.md` §5.3 used to reject `streamvbyte`. `aws-lc-rs` is likewise C/CMake. **`native-tls` binds Schannel through the pure-Rust `schannel` crate** and built cleanly on the same broken-headers machine. |
| "Loki is 20–30x slower than Tailhawk's entire first-paint budget." | **Artifact of a pathological query.** The headline figure came from `{app=~".+"}` — reproduced worse at 4.0 s and 112 MB scanned to return **three** lines. The counter-case was never run: a *tight* selector returned 1000 entries over 6h in **0.068 s**, and 200 entries over 5m in **0.031 s**. That is 60–130x, comfortably inside §11.3's <150 ms warm budget before RTT. |

The third one matters most, because the bad number was used to justify the 1 s poll cadence, the
30 s idle back-off, and the "frozen scrollbar" argument against merging outside the materialised
extent. Those conclusions may survive on other grounds, but **their stated justification is wrong**.

Re-motivate the guardrail correctly: **selector quality is a 60–130x performance lever**, which makes
"refuse to emit an unbounded or `.+`-only selector without explicit confirmation" the highest-value
UX rule in the feature — not a courtesy to the platform team.

Net effect: `native-tls` delivers the only substantive advantage claimed for WinHTTP (Windows
certificate store, enterprise proxy behaviour) with no hand-written Win32 and no C toolchain. The
case for Route B is gone. **Re-measure inside the real tree once windows-rs/D2D is present**, since
LTO across a larger crate graph shifts the delta.

## 6. Correctness rules that are non-negotiable

**Silent truncation is the single most important rule in the feature.** Loki returns at most `limit`
entries and sets **no truncated flag**. If a page returns exactly `limit` rows, the client cannot
distinguish "that was all" from "you were cut off". Tailhawk's chips are client-side and run *after*
truncation, so chip counts on a truncated window are confidently wrong — and the spill design makes
this *worse* by hiding page boundaries behind something that looks exactly like a file.

> Rule: when a page returns exactly `limit`, mark the window truncated, render a persistent banner
> naming cause and remedy, and downgrade every client-side chip and search count in that window from
> an exact number to **"at least N"**. A local file's "7 matches" is exact; this must never look like
> it.

Other rules:

- **Never push down a predicate on a field outside the source's discovered field set** (from
  `/labels`, `/series`, `/detected_fields`, refreshed on time-range change). Out-of-set predicates
  evaluate client-side and render in §7.2's existing warning state. This preserves the UNKNOWN
  tri-state without depending on Loki's missing-label coercion, which could not be verified for
  numeric comparisons.
- **Bare-text chips diverge and must be redefined.** §7.2 defines bare text as substring over the
  **whole record**; `|~ "(?i)…"` matches the **line** only — false negatives against structured
  metadata, on a deployment where metadata is the *majority* of the payload. Redefine as body-scoped
  with a visible tooltip. This produces a plausible wrong answer, not an error, so it must be
  specified rather than discovered.
- **fancy-regex chips can never be pushed down.** §7.3 deliberately supports lookaround and
  backreferences because log4net/NLog users write `(?<!DEBUG).*Exception` routinely. Loki is RE2 and
  has neither. Such chips search only what was fetched, and must say so.
- **Emit the pipeline in canonical order:** `{selector} | structured-metadata filters | line filters
  | (parser) | post-parse filters`. Metadata filters must precede any parser/`label_format`/`drop`/
  `keep` stage or bloom acceleration is lost — this is semantic, not just cost.
- **Hoist timestamp comparisons out of the chip row into `start`/`end`**, or they silently cost a
  full-range scan.
- **`detected_level` is a pushdown hint only** and is never written into `severity_number` — §6.2
  exists precisely because of Loki's level-detection bug where a level word anywhere in the message
  hijacks detection. Severity is a three-tier capability probe at connect: `severity_number`, then
  `detected_level`, then a promoted `level` label.
- **Polling stays the correctness mechanism; the tail WebSocket is an accelerator allowed to fail
  silently** — §5.4's rule transfers in shape for four independent reasons: `dropped_entries` is a
  documented admission of loss; `max_concurrent_tail_requests` defaults to **10 per tenant**, so one
  workstation with six tabs can starve a colleague's Grafana live-tail; long-lived sockets die
  silently on proxy idle timeouts; and out-of-order ingestion lands records behind the tail cursor
  forever. Render `dropped_entries` as a **gap row**, using §5.5's truncation-separator precedent.
- **Reuse §8.3's reorder window `W` (2 s)** for Loki's out-of-order ingestion — structurally the same
  problem as Serilog's periodic-batching sink, and it earns the same settling band.
- **Discover server limits by failing and parsing the error.** There is no reliable limits endpoint,
  `/config` is commonly disabled, and errors arrive as **bare text, not a JSON envelope**. Cache per
  endpoint. Do not write any Loki server flag name or default into the spec — the two research rounds
  disagreed on the flag prefix, and the value is per-tenant anyway.

## 7. Security — the credential design as researched is unsound

Two of the research's own load-bearing controls are defects, not fixes.

1. **A credential reference keyed by source *name* is an exfiltration primitive.** The name is
   attacker-chosen data arriving inside untrusted imported config, decoupled from the URL the
   credential will be sent to. Send someone a `tailhawk.toml` reusing their source name with your
   URL, and their stored token is attached to your host behind one plausible dialog. Grafana
   service-account tokens have **no expiry by default** and are not tied to a user account, so
   offboarding the human does not revoke it.
   **Fix:** key by canonical **origin** (`tailhawk:loki:<scheme>://<host>:<port>[:<tenant>]`), store
   the origin inside the credential record, recompute from the URL about to be contacted and require
   an exact match — no fallback, no "reuse?" prompt. Imported config must be **structurally
   incapable of naming a credential**: it carries a URL, nothing else.
2. **"DPAPI or Credential Manager" is not a design.** Microsoft's own docs: a roaming-profile user
   can decrypt a DPAPI blob **from another machine**, and `CRYPTPROTECT_LOCAL_MACHINE` exposes it to
   every local user. A DPAPI ciphertext stored as a file is portable ciphertext subject to domain
   backup-key recovery.
   **Fix:** the secret **never exists as a file, anywhere**. Credential Manager, `CRED_TYPE_GENERIC`,
   `CRED_PERSIST_LOCAL_MACHINE` — explicitly **not** `CRED_PERSIST_ENTERPRISE`, which roams. DPAPI
   only under `%LOCALAPPDATA%` with non-null entropy, if at all. Under `--stateless`, or when the
   config path is read-only or remote, refuse to persist: prompt per session, memory only. State in
   the spec that "config on a network share" and "stored Loki credential" are **mutually exclusive by
   construction**.

Four things are missing outright and are each independently blocking:

- **A compiled-in endpoint path allowlist.** The same base URL and the same header reach
  `POST /loki/api/v1/delete`, `/flush` and `/ingester/shutdown`. Users will paste the write-scoped
  token already in their shipper config. No config-supplied path fragment, query string or fragment
  may ever reach path construction; reject a base URL carrying any of those, or userinfo.
- **Real SSRF controls.** Confirmation dialogs do not survive a config declaring 400 sources across
  RFC1918 space. Required: **zero DNS** before confirmation (resolution is itself a beacon — a
  hostname can encode the victim's machine identity); deny loopback, link-local and the IMDS address
  with no config-settable override; re-check the resolved address at connect time against DNS
  rebinding; cap previously-unseen origins per import and present them in **one** dialog.
  Note SmartScreen explicitly does not protect files on network shares — no OS layer catches the
  delivery.
- **Explicit redirect handling.** WinHTTP's documented default follows cross-origin https→https
  redirects up to 10 hops with no application involvement, silently violating the stated
  no-cross-host-redirect policy. Never attach an `Authorization` header to a request whose origin
  differs from the origin the credential is bound to — including redirects, retries and WebSocket
  upgrades.
- **Response-parse hardening.** The recommended `Accept-Encoding: gzip` and the *mandatory* intern
  table are both remote-OOM levers under a hostile server: a gzip bomb, or streams whose label keys
  are all unique random strings. Hard non-configurable caps on decompressed bytes, compression ratio
  (abort on ratio, not final size), nesting depth, keys per stream, key/value/line length, records
  per response; bounded intern table with graceful fallback. Strip C0/ANSI/bidi sequences from all
  server text before rendering, copying or exporting.

Two further corrections on the record:

- **WinHTTP does *not* handle enterprise proxy/PAC automatically** — documented as application work
  (`WinHttpGetProxyForUrl` then `WinHttpSetOption`). Worse, the sample code an implementer will copy
  enables WPAD DHCP/DNS auto-detect with `fAutoLogonIfChallenged = TRUE`, i.e. auto-submits domain
  credentials to a proxy chosen by a broadcast race. Never enable WPAD auto-detect; never set that
  flag.
- **The Grafana datasource-proxy route is not a security win.** It swaps a narrow Loki credential for
  a broad, non-expiring Grafana one that also reaches `GET /api/datasources` — every datasource in
  the org and its connection settings. Prefer it for *reachability*, with a minimal service account,
  and warn on an Admin-role token.

Also: **TLS posture is undefined and must be settled before the first private-CA bug report.** No
insecure toggle in the TOML, ever — `insecure`, `skip_verify`, `tls_*`, `ca_file` are
executable-intent-class and rejected at parse time. A boolean that turns off certificate checking is
*worse* than a command in a config, because a command is legible and this is not. Rely on the Windows
certificate store; offer per-origin thumbprint pinning stored locally.

And a privacy clause §13.2 does not currently cover: until now Tailhawk's network I/O has been
*reading* bytes the user already owns. A Loki source makes it **send user-authored filter text** —
which routinely contains customer identifiers — to a third party that logs it, in a URL, into every
proxy access log on the path. Use the documented `POST` + form-encoded form of `query_range` for
anything containing user-typed filter text. The headline promise survives; it needs a second
sentence, not a rewording. Separately, §13.2's spill clause was written for stdin and must be
extended to **credentialed remote data** landing in `%TEMP%`.

Finally: **the zero-network CI assertion must be rewritten before the HTTP code lands**, moving from
"no sockets ever" to "no sockets unless a remote source is configured, and only to the configured
host". The public claim survives unchanged, but the test backing it gets weaker and will rot
silently if written afterwards — the same failure mode §11.1 was written to prevent for performance
numbers.

## 8. Cost and sequencing

> **The decisions at the end of this section are settled.** The analysis above them is retained
> because it is what the two rounds actually found, and because the cost reconciliation is still
> owed to `PLAN.md`.

**Do not adopt either cost figure.** Round 1 said 14–18 PW; round 2 said ~31 PW (range 26–38). A 2x
disagreement on the number the decision rests on. Round 1 is cheaper because it silently omits six
items round 2 costs: the spill/materialised window, the timeline histogram, per-partition format
detection, the interning parser, cost guardrails, and the CI-test rework. Round 1 also never mentions
the 15 MB gate or names an HTTP crate.

Both benchmarked against v1, and both used the wrong denominator — `PLAN.md` §2.4 says v1 is
**81.5 PW A**, not 78. But the work would land in **v2**, whose budget is **28–36 PW A**. At round 2's
figure the Loki source is **86–110% of the entire v2 budget** — the merged-by-timestamp view, trace
correlation, process sources, archives, comparison, alerts, HTML export, full `--stdout` parity, and
the 6–10 PW accessibility workstream. **At that price the Loki source *is* v2.**

Two structural findings complicate the sequencing further:

1. **The timeline histogram, currently v3, becomes load-bearing.** It is the only navigation
   affordance for a time-primary source, and Grafana Explore has one. Largest hidden cost in the
   request.
2. **A materially cheaper 80% path already exists in the plan, and neither researcher proposed it.**
   `SPEC.md` §4/§15 already plan process-spawn sources for v2 (`docker logs -f`, `kubectl logs -f`).
   **`logcli` is exactly that shape** — `tail` subcommand, `--output jsonl`, `--limit/--since/--from/
   --to/--forward`, credentials from `LOKI_ADDR`/`LOKI_USERNAME`/`LOKI_PASSWORD`/`LOKI_BEARER_TOKEN`/
   `LOKI_ORG_ID`. Against v1's already-planned stdin spill,
   `logcli query --output=jsonl --tail '{app="x"}' | tailhawk -` works at **roughly zero incremental
   cost the day v1 ships**, and keeps credentials entirely outside the product — sidestepping the
   credential subsystem, the DPAPI work, the parse-time rejection and most of §7's attack surface in
   one move. It does not deliver: the label browser, server-side pushdown, the histogram,
   `index/stats` pre-flight, or an in-app time picker.

There is also a genuine disagreement between the research and the scope critic on ordering. The
research says land a ~11 PW read-only window as the **opening** item of v2. The critic says **after**
§8.3's merged view, on the grounds that merged local+remote is the only capability on the list
Grafana Explore does not already have — Explore ships live tail, log context, TXT/JSON/CSV download,
level filters, client-side search, per-field filtering, four dedup modes, line wrap with JSON
prettify, sort order and a log-volume histogram today. Note two of Tailhawk's *v3* items are
catch-up to Explore, not lead.

### Decisions — settled 2026-07-29 by the owner

**Tailhawk is a Loki client.** The `logcli` viewer path is rejected: a dependency on another CLI
binary contradicts the "single self-contained exe, no runtime deps, copy-and-run" promise that is
core product identity, and it externalises exactly the part that makes Tailhawk worth using over
Explore. This overrides the scope critic's recommendation in §8, which was argued primarily on cost.

**Cost is not a constraint.** This is a side project running alongside other work, with no delivery
date. The "86–110% of the v2 budget" objection therefore does not bind — there is no fixed budget to
consume and nothing is being displaced. The two cost rounds still need reconciling before `PLAN.md`
§2.3b gets a number, but the number is no longer a gate on proceeding.

**Staging: client-lite first, then the full client.** The rationale is not cost — it is that
client-lite exercises only the parts of the design that *survived* re-verification, and defers the
part that was found unsound.

| Stage | In | Out |
|---|---|---|
| **1 — client-lite, tail included** | `query_range` GET; hand-typed stream selector; one materialised window to a CLEF spill driven by the existing index and grid; `ExtentState` load-older/load-newer; time range (relative presets + absolute); all chips client-side; token from env var or `--token-from-file`, **memory only, never persisted**; **poll-based follow with the settling band** | Credential store, pushdown, tail WebSocket, histogram, label browser |
| **2** | Credential subsystem on the §7 rules (origin-keyed, Credential Manager, no file); timeline histogram; **tail WebSocket as an accelerator over the poll** | — |
| **3** | Chip pushdown with the pushdown/residual split; label and field browser; `index/stats` pre-flight | — |

> **Amended 2026-08-27 — see the superseded-order note in §8 below.** Follow moved from stage 2 into
> stage 1 and the WebSocket from stage 3 to stage 2, because the owner has made tailing the point of
> the exercise rather than a refinement of it.
>
> **Follow arrives as a poll, and that is the design, not a shortcut.** §6 already settles it:
> polling is the correctness mechanism and the tail WebSocket is an accelerator *allowed to fail* —
> `/tail` drops entries under load with a documented admission of loss, `max_concurrent_tail_requests`
> defaults to 10 per tenant so a few workstation tabs can starve a colleague's live tail, long-lived
> sockets die silently on proxy idle timeouts, and out-of-order ingestion lands records behind the
> tail cursor. A poll on the settling band is what makes the pane *correct*; the socket only makes it
> *quicker*. §9 also records that nobody has established whether the Grafana datasource proxy
> forwards a WebSocket upgrade at all — so a stage 1 that depended on the socket would be a stage 1
> that might not work.

**Client-lite is not "security later".** Only the *storage* controls defer, because there is no
stored secret to protect. Everything in §7 that governs the request itself is required from the
first HTTP call: the compiled-in endpoint path allowlist, the SSRF controls (no DNS before
confirmation, deny loopback/link-local/IMDS, address re-check against rebinding), explicit redirect
handling with no credential across an origin change, TLS posture with no insecure toggle in the
TOML, and the response-parse caps. The §13.2 zero-network CI test must be rewritten **before** the
first HTTP code lands, not after.

### Order — **superseded 2026-08-27 by the owner: Loki tailing comes first**

**Tailing Loki is the priority, ahead of §8.3's merged view.** The owner has asked for this
repeatedly and it has been lost repeatedly, because the ordering recorded below argued the opposite
and any session reading this file faithfully re-derived it and handed it back as a recommendation.
That is what happened on the morning of 2026-08-27. **This heading is now the answer; the reasoning
kept beneath it is history, not instruction.**

Two things follow from the override:

- **Following is not a stage-3 luxury.** The staging table above lists *follow* and *tail* as out of
  client-lite. That is reversed: **a source you cannot watch live is not the feature that was
  asked for.** The first useful slice is "point Tailhawk at a Loki selector and watch lines
  arrive", and everything else — the label browser, pushdown, the histogram, `index/stats` —
  ranks behind it.
- **`logcli` remains rejected, and is not a stepping stone.** `logcli query --tail | tailhawk -`
  works today through the stdin pump and is *not* an answer to this request. Do not offer it as
  one.

**What is unchanged, because it is about doing it correctly rather than about when.** §7's
request-level controls are required from the first HTTP call — the endpoint allowlist, the SSRF
defence, redirect handling with no credential across an origin change, the TLS posture with no
insecure toggle in the TOML, and the response-parse caps. And `SPEC.md` §13.2 claims the
zero-network guarantee is "a **testable assertion** in CI"; **it is not — no such assertion exists
today**, and writing it is a prerequisite rather than a delay, because it is what makes the
conditional form of the claim ("no connection unless the user opened a remote source") checkable
at all.

<details>
<summary>The superseded ordering and its four reasons, kept as the record of what was decided on
2026-07-29 and why it was overridden.</summary>

The order was: §8.3 merged view (local only), then Loki client-lite, then §9 trace correlation,
then Loki stages 2–3. The reasons were:

- **It keeps the strong privacy claim alive longer.** While no HTTP code exists, §13.2's CI
  assertion is maximally strong — the process opens no sockets, ever. The first HTTP call weakens it
  permanently to a conditional that is harder to test and easier to regress. Shipping a headline
  differentiator under the strong form of the claim is worth having, and it is a one-way door.
- **The merged view is entirely offline.** Over local sources it needs none of §7, no TLS posture
  and no CI-test rewrite — the largest block of differentiating work carrying zero network risk.
- **Merge is easier to get right against the messy inputs first.** §8.3's hard cases — clock skew,
  RFC 3164 year inference, log4net local-time guessing, the bounded reorder window — all come from
  *local* logs. A Loki source is the clean input (§2). Build the engine against the awkward sources,
  then drop in the well-behaved one; the reverse debugs reorder-window and clock-domain problems
  while simultaneously debugging network paging.
- **The rate mismatch wants an existing merge engine.** ~466 lines/s remote against ~500k lines/s
  local is a 1000:1 drowning ratio needing per-source quotas — far easier to reason about once the
  engine exists and can be measured.

The first reason is the one worth answering rather than dismissing: the privacy claim *is* a
one-way door. The answer is that the door is opened deliberately and the CI assertion is written
to match — see the note above — rather than left unopened to keep an assertion easy.

§9 was placed after client-lite because `trace_id` is sparse in local files and Loki is where
distributed traces actually live. That reasoning survives; §9 still follows.

</details>

The research's original argument for Loki-first (`ExtentState` is a §8.3 prerequisite) is spent:
`ExtentState` lands in v1 regardless (§4), and the spill already exists for stdin, so v1 validates
both load-bearing abstractions before any of this starts.

Independent of the above: the §7 request-level security scaffolding and the §13.2 CI-test rewrite
have no dependency on the merge engine and can be done at any point before the first HTTP call.

## 9. Open questions

~~**Highest value, answer first:** is the Loki HTTP API reachable **directly** from a developer
workstation, or only through Grafana's datasource proxy?~~ **Answered 2026-08-27: directly.**

Every empirical measurement in this document had gone through Grafana's proxy, and the worry was
that proxy-only would mean a datasource-proxy mode with service-account tokens — a different URL
shape, different auth and about +1 PW. None of that is needed. Grafana is not in the path at all.

Three things follow, and the third is a correction to code that is already written:

- **The base URL carries a mount prefix.** Loki sits behind a reverse proxy that matches a prefix,
  strips it, and forwards to the read service — so the configured base is *not* bare
  `scheme://host:port`, and the endpoint path from §3 follows the prefix. **`loki.rs`'s
  `Origin::parse` refuses any path on a base URL** in service of §7's "no config-supplied path
  fragment may ever reach path construction", which as written makes Tailhawk unable to reach this
  deployment at all. The answer is a **bounded mount prefix as its own field** — charset-checked,
  no `..`, no query, no fragment, length-capped — with the `Endpoint` enum still the only authority
  on the endpoint's own path. §7's intent survives: configuration supplies a mount point, never a
  constructed path.
- **The credential is the narrow one, which is the good outcome.** Authentication is OAuth2
  client-credentials with a read-only telemetry scope from the estate's own identity server, not a
  Grafana service-account token — so §7's objection to the proxy route ("swaps a narrow Loki
  credential for a broad, non-expiring Grafana one that also reaches `GET /api/datasources`") does
  not apply. There is **no tenant header**: the deployment runs single-tenant with Loki's own auth
  disabled, so `X-Scope-OrgID` is not sent. Retention is 30 days, which bounds what any time range
  can usefully ask for.
- **The tail's upgrade is untested but unobstructed.** The path is a container platform's ingress,
  then a Kestrel host, then the reverse proxy — all three forward WebSocket upgrades natively — and
  there is no application gateway, CDN or nginx in front, which is where upgrades usually die. It
  still needs the smoke test the next bullet asks for; the risk is now low rather than unknown.

> Endpoint host names and scope names are deliberately **not** written here. This repository is
> public; the concrete addresses live in the owner's own notes.

Then:

- Does the Grafana datasource proxy forward a **WebSocket upgrade** to `/loki/api/v1/tail`?
  Undocumented, untestable through a GET proxy. Smoke-test before committing to the tail endpoint.
- Does the proxy restrict method/path to Loki's **mutation** endpoints? Which RBAC action gates the
  proxy route? Undocumented; the RBAC page 404s.
- Real measured stripped-release binary delta **inside the real tree**, once windows-rs/D2D is
  present. The §5 measurement was a synthetic workspace.
- Is a quoted-literal chip case-**sensitive**? §7.2 is silent. Decides `|=` (exact, cheap,
  bloom-friendly) versus `|~ "(?i)…"`.
- Loki's behaviour for a **numeric** label filter on a missing label. The recommended rule avoids
  depending on it, but it should be confirmed.
- ~~The exact value vocabulary of `detected_level`, needed for the band-enumeration fallback.~~
  **Measured 2026-08-27** against the owner's own deployment, over a day: `trace`, `debug`, `info`,
  `warn`, `error`, `critical` — lower case, six values, and **no `fatal`**. All six already resolve
  through `record.rs`'s `Severity::from_level_text`, which is now asserted by a test named for the
  fact rather than left as a note. The test exists because the deployment's Loki runs a moving image
  tag: a seventh word could appear with no commit anywhere, and an unresolved level silently means
  every severity-banded rule, colour and filter stops applying to those records.

  Measured at the same time, and it settles the shape of the source: the **stream labels are only
  `app`, `environment`, `level`** — plus `__stream_shard__`, which §2's suppression rule already
  names, now confirmed to be really there. Everything else §2 lists arrives as structured metadata,
  which is consistent with §3's 5.4x amplification coming from the metadata rather than from the
  handful of indexed labels, and is why the interner is load-bearing.
- Does WinHTTP replay application-added headers across a redirect? The safe design does not depend on
  the answer and must not be allowed to.
- Are process-spawn sources expressible in imported config at all, given §13.1 rejects executable
  intent at parse time? This already affects planned v2 `docker`/`kubectl`/`az` work, and the cheap
  `logcli` route inherits it.
- Credential behaviour under all three §12.4 tiers and `--stateless`. Round 2 costed the whole
  credential subsystem at 1.5 PW; that does not look like enough.
- Does the merged view need **per-source row quotas**? Measured remote rate ~466 lines/s against a
  local file's ~500k lines/s at §11.3's 50 MB/s target — a 1000:1 drowning ratio.
- Should **merge by `observed_timestamp`** be a first-class §8.3 mode? Loki records carry both
  emitter and collector clocks — a skew-reduction lever no local source offers.
