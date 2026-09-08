#!/usr/bin/env bash
set -euo pipefail
cd "${1:-$(dirname "$0")/../..}"
app="$PWD/app_flutter/build/macos/Build/Products/Release/NekoSend.app"
evidence="$PWD/dist/macos-evidence"
database="$HOME/Library/Application Support/NekoSend/lan_chat.db"
mkdir -p "$evidence"
# Only run on the disposable CI user, never against a developer's real history.
test "${GITHUB_ACTIONS:-}" = true
test ! -e "$database"
"$app/Contents/MacOS/NekoSend" > "$evidence/launch.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
ready=0
for attempt in $(seq 1 60); do
  kill -0 "$pid"
  if test -f "$database" && test "$(sqlite3 "$database" 'SELECT platform FROM local_profile;' 2>/dev/null || true)" = macos; then
    if test -n "$(sqlite3 "$database" 'SELECT default_receive_ref FROM app_settings;' 2>/dev/null || true)"; then
      ready=1
      break
    fi
  fi
  sleep 1
done
test "$ready" = 1
sleep 5
kill -0 "$pid"
sqlite3 "$database" 'PRAGMA user_version; SELECT device_name, platform FROM local_profile; SELECT default_receive_ref FROM app_settings;' > "$evidence/bootstrap.txt"
# The real app must start its LAN TCP listener, not merely create a database.
lsof -nP -a -p "$pid" -iTCP -sTCP:LISTEN > "$evidence/listeners.txt"
screencapture -x "$evidence/desktop.png" || true
