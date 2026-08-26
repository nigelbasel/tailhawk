# Tailhawk

**A Windows desktop viewer for log files — including files far too large for a text editor — with
structured-log awareness, delivered as a single executable that needs no installation and no
runtime.**

It opens a multi-gigabyte file instantly and in bounded memory, follows it as it grows, detects the
character encoding (including BOM-less UTF-8 and UTF-16), splits records into columns driven by
your *actual* logging configuration, and merges several files into one timestamp-ordered view.
It is read-only, always — that is a tested guarantee — and it makes no network connection of its
own account: no telemetry, no update ping, no font or CDN fetch.

What it is not: an observability platform, a metrics or tracing tool, a log shipper, or a text
editor. For live containerised OpenTelemetry debugging the right tool is the .NET Aspire Dashboard,
and Tailhawk does not compete with it.

## Status

**In development, and not yet released.** There is no download and no installer; the author builds
it and runs it daily against real logs. The renderer, the grid, the filter panel, search,
bookmarks, formats and the dialogs are in and exercised; the parts still outstanding are recorded
at the top of [`docs/HANDOFF.md`](docs/HANDOFF.md).

Windows 10 1809 (build 17763) and later. **x64 is the shipped and supported architecture; ARM64 is
built and published best-effort**, deliberately not claimed as first-class, because the performance
targets have only ever been measured on x64. Linux and macOS are not on the plan
([`docs/SPEC.md`](docs/SPEC.md) §2.1).

## Building

```
cargo build --release -p tailhawk
```

The result is one `tailhawk.exe` of about 2 MB with no side-by-side dependencies — no CRT
redistributable, no runtime shader compiler, nothing to install alongside it. CI asserts all three
on both architectures.

The gates the project holds itself to, and which CI runs on x64 and ARM64:

```
cargo fmt -p tailhawk -p tailhawk-core -- --check
cargo clippy --release -p tailhawk -p tailhawk-core -- -D warnings
cargo test --release --workspace
```

## The documents

Tailhawk is specified before it is written, and the specification is the contract — behaviour is
settled there rather than in code review.

| | |
|---|---|
| [`docs/SPEC.md`](docs/SPEC.md) | What Tailhawk does, and what it deliberately does not. The contract. |
| [`docs/USING.md`](docs/USING.md) | The one-page guide to the viewer as it runs: command line, keys, filters, formats. |
| [`docs/UI-DESIGN.md`](docs/UI-DESIGN.md) | The screen — layout, keys, and the surfaces the spec's V-numbers refer to. |
| [`CLEANROOM.md`](CLEANROOM.md) | The provenance record, and the rule that governs how components may be written. |
| [`docs/RESEARCH.md`](docs/RESEARCH.md) | The field as it stood before any code: the competitors, and what none of them combine. |
| [`docs/PLAN.md`](docs/PLAN.md) · [`docs/EFFORT.md`](docs/EFFORT.md) | The estimates, and the actuals scored against them. |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | Where the work stands today, and what is next. |

### On the clean room

The closest prior art in this category is GPL-licensed — klogg, TailBlazer, SnakeTail — and Tailhawk
ships under MIT OR Apache-2.0. [`CLEANROOM.md`](CLEANROOM.md) is the record that their
implementation source was not read: every core module is registered there with the published
sources it was derived from, the entry is written *before* the module, and CI fails the build if a
module in `crates/tailhawk-core/src/` is named nowhere in it.

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
