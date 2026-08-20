#!/usr/bin/env bash
# The full gate, with the dogfood instance got out of the way first.
#
# **Run this rather than `cargo test` directly.** A running Tailhawk breaks the shell suite in two
# ways, and both look like flakiness rather than a cause:
#
#   * it holds `target/release/tailhawk.exe` open, so a rebuild fails with a bare
#     "Access is denied" that names nothing;
#   * it competes for CPU and the filesystem, and several shell tests are timing criteria — a
#     watched folder adopting a file, a filter following growth. Measured 2026-08-20: with the
#     dogfood instance up, 4 to 7 of them fail and the run takes 1.33 s; with it stopped, all 85
#     pass in 0.55 s. The failing set differs run to run, which is exactly what makes it read as a
#     flake and sent one session chasing a regression that was not there.
#
# It restarts the instance afterwards, because `docs/HANDOFF.md` wants the newest working build up
# and tailing the activity log at all times.
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 1

"$root/tools/dogfood.sh" --stop >/dev/null 2>&1

fail=0
step() {
  echo
  echo "=== $1 ==="
  shift
  if "$@"; then
    echo "ok"
  else
    echo "FAILED"
    fail=1
  fi
}

step "fmt" cargo fmt -p tailhawk -p tailhawk-core -- --check
step "clippy" cargo clippy --release -p tailhawk -p tailhawk-core --all-targets -- -D warnings

# The suite, kept in a file so the known timing flake can be told apart from a real failure.
echo
echo "=== test ==="
if cargo test --release --workspace > "$root/target/check-test.log" 2>&1; then
  echo "ok"
else
  # `semantic::tests::a_screenful_costs_a_fraction_of_a_frame` is a timing criterion documented in
  # `CLAUDE.md`: it fails under the full parallel suite on a loaded machine and passes alone. The
  # rule there is to rerun it alone before blaming a change, so that is done here rather than left
  # to whoever reads the log — but **only** when it is the sole failure. Anything alongside it is a
  # real failure and the gate stays red.
  failed=$(grep -cE '^test .* FAILED' "$root/target/check-test.log" || true)
  flake=$(grep -cE '^test semantic::tests::a_screenful_costs_a_fraction_of_a_frame \.\.\. FAILED' \
    "$root/target/check-test.log" || true)
  if [ "$failed" = "1" ] && [ "$flake" = "1" ]; then
    echo "the known timing flake failed under the parallel suite; rerunning it alone"
    if cargo test --release -p tailhawk-core a_screenful_costs_a_fraction_of_a_frame >/dev/null 2>&1
    then
      echo "ok (the flake passes alone, as CLAUDE.md says it does)"
      "$root/tools/agentlog.sh" INFO test \
        "the screenful timing flake failed under the parallel suite and passed alone — not a regression"
    else
      echo "FAILED — it fails alone too, so this is real"
      fail=1
    fi
  else
    echo "FAILED"
    grep -E '^test .* FAILED' "$root/target/check-test.log" || true
    fail=1
  fi
  echo "full output: target/check-test.log"
fi

echo
if [ "$fail" -eq 0 ]; then
  "$root/tools/agentlog.sh" INFO test "gate green: fmt, clippy and the full workspace suite"
  echo "=== gate green ==="
else
  "$root/tools/agentlog.sh" WARN test "gate FAILED — see the step marked FAILED above"
  echo "=== gate FAILED ==="
fi

# Back up on the binary that was just checked, whatever the result: the owner watches the log
# through it, and leaving it down is worse than leaving it running a build that failed clippy.
"$root/tools/dogfood.sh" >/dev/null 2>&1 && echo "dogfood instance back up"

exit "$fail"
