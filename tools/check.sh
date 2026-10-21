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

# **The flakes `CLAUDE.md` documents, by name.** Each fails under the full parallel suite on a loaded
# machine and passes when run alone: two are timing criteria, one depends on when a write became
# visible, and two lose a race for an exclusive handle on a spill nothing should be holding.
# `CLAUDE.md`'s rule is to rerun the named test alone before blaming a change for it, so the script
# does that rather than leaving it to whoever reads the log.
#
# **Only these names, and only when every failure is one of them.** Anything else alongside them is
# a real failure and the gate stays red. Until 2026-10-21 this handled the screenful timing
# criterion alone and only as the sole failure, so the other four — all documented in `CLAUDE.md`,
# one of them met twice in one day — took the gate red and had to be argued about by hand.
FLAKES=(
  'semantic::tests::a_screenful_costs_a_fraction_of_a_frame'
  'paint::tests::the_layout_cost_does_not_scale_with_the_number_of_visible_columns'
  'stdin::tests::a_spill_is_an_ordinary_file_to_the_rest_of_the_product'
  'stdin::tests::reaping_removes_an_orphan_and_leaves_a_live_spill_alone'
  'stdin::tests::reaping_removes_an_orphan_spill_directory_and_leaves_a_live_one_alone'
)

# The suite, kept in a file so the known flakes can be told apart from a real failure.
echo
echo "=== test ==="
if cargo test --release --workspace > "$root/target/check-test.log" 2>&1; then
  echo "ok"
else
  failures=$(grep -E '^test .* FAILED' "$root/target/check-test.log" \
    | sed 's/^test //; s/ \.\.\. FAILED.*//' | sort -u)
  unknown=0
  for name in $failures; do
    known=0
    for flake in "${FLAKES[@]}"; do
      [ "$name" = "$flake" ] && known=1
    done
    [ "$known" = "0" ] && unknown=1
  done
  if [ -n "$failures" ] && [ "$unknown" = "0" ]; then
    echo "every failure is a documented flake; rerunning each alone"
    alone_ok=1
    for name in $failures; do
      # The bare test name, because `cargo test` filters on a substring and the module path is not
      # one the filter matches across both crates.
      if cargo test --release --workspace "${name##*::}" >/dev/null 2>&1; then
        echo "  $name passes alone, as CLAUDE.md says it does"
      else
        echo "  $name fails alone too, so this is real"
        alone_ok=0
      fi
    done
    if [ "$alone_ok" = "1" ]; then
      echo "ok (every failure was a documented flake and passed alone)"
      "$root/tools/agentlog.sh" INFO test \
        "documented flakes failed under the parallel suite and passed alone: $(echo $failures)"
    else
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
