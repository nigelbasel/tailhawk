#!/usr/bin/env bash
# Append one line to the agent activity log.
#
#   tools/agentlog.sh INFO  edit "cell.rs — hoisted the ASCII singleton test"
#   tools/agentlog.sh WARN  test "3 failing after the anchor change"
#   tools/agentlog.sh ERROR build "link failed: missing Win32_System_Ole feature"
#
# The log lives at logs/agent.log and is **not** committed — it is a running
# record of what the agent did, for the owner to tail while work happens.
#
# The format is deliberately an ordinary log shape rather than JSON, because the
# point of it is to be read by Tailhawk itself: ISO-8601 UTC with milliseconds,
# a level, a short action word, and free text. That exercises timestamp parsing,
# level colouring and long-line handling on real content the owner cares about.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log="$root/logs/agent.log"
mkdir -p "$root/logs"

level="${1:-INFO}"
action="${2:-note}"
shift 2 || true
message="$*"

# %3N is milliseconds. GNU date only; this repo is developed under Git Bash.
printf '%s %-5s %-8s %s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)" \
    "$level" \
    "$action" \
    "$message" >> "$log"
