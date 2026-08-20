#!/usr/bin/env bash
# Keep the newest working Tailhawk up, tailing this session's activity log.
#
# The owner watches `logs/agent.log` through Tailhawk itself, so the build under development is
# also the tool supervising the build. Run this after every green build: it retires only the
# instance already pointed at the activity log — matched on its command line, so a window the
# owner opened on a real log is never touched — and starts the freshly built binary in its place.
#
#     tools/dogfood.sh          start the newest build on the activity log
#     tools/dogfood.sh --stop   retire it, so `cargo build` can replace the exe
#
# `--stop` is not optional politeness: a running instance holds `tailhawk.exe` open and the build
# fails with "Access is denied" rather than anything that names the cause.
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log="$root/logs/agent.log"
exe="$root/target/release/tailhawk.exe"

stop_only=0
[ $# -gt 0 ] && [ "$1" = "--stop" ] && stop_only=1

if [ "$stop_only" -eq 0 ]; then
  [ -x "$exe" ] || { echo "no release binary at $exe — build first" >&2; exit 1; }
  [ -f "$log" ] || { echo "no activity log at $log" >&2; exit 1; }
fi

# Only the instance tailing the activity log. `CommandLine` is the discriminator; a window on any
# other file has a different one and survives.
mapfile -t stale < <(
  powershell -NoProfile -Command \
    "Get-CimInstance Win32_Process -Filter \"Name='tailhawk.exe'\" |
       Where-Object { \$_.CommandLine -match 'agent\.log' } |
       ForEach-Object { \$_.ProcessId }" 2>/dev/null | tr -d '\r'
)
for pid in "${stale[@]}"; do
  [ -n "$pid" ] && taskkill //PID "$pid" //F >/dev/null 2>&1
done

if [ "$stop_only" -eq 1 ]; then
  echo "dogfood instance stopped"
  exit 0
fi

(nohup "$exe" "$log" >/dev/null 2>&1 &)
sleep 2

if powershell -NoProfile -Command \
     "if (Get-CimInstance Win32_Process -Filter \"Name='tailhawk.exe'\" |
            Where-Object { \$_.CommandLine -match 'agent\.log' }) { exit 0 } else { exit 1 }" \
     >/dev/null 2>&1; then
  "$root/tools/agentlog.sh" INFO note "dogfood instance restarted on the freshly built binary, tailing logs/agent.log"
  echo "tailhawk is up on logs/agent.log"
else
  echo "tailhawk did not come up" >&2
  exit 1
fi
