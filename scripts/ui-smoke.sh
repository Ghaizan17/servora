#!/usr/bin/env bash
# Navigation smoke test against the preview harness.
#
# Loads the real frontend in headless Chrome, clicks every sidebar entry in both
# directions, and fails if any page comes up empty, shows raw markup, leaves a
# stale header or does not update the hash.
#
# Usage: scripts/ui-smoke.sh [port]
set -euo pipefail
PORT="${1:-5178}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

chrome=$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)
if [ -z "$chrome" ]; then
    echo "ui-smoke: no Chrome/Chromium found" >&2
    exit 2
fi

if ! curl -fsS "http://localhost:$PORT/dev/preview.html" >/dev/null 2>&1; then
    echo "ui-smoke: start the dev server first: npx vite --port $PORT" >&2
    exit 2
fi

out=$(mktemp)
trap 'rm -f "$out"' EXIT
"$chrome" --headless=new --disable-gpu --virtual-time-budget=20000 \
    --dump-dom "http://localhost:$PORT/dev/preview.html?smoke=1" > "$out" 2>/dev/null || true

if ! grep -q 'id="smoke-result"' "$out"; then
    echo "ui-smoke: FAIL — the harness did not run the smoke pass" >&2
    exit 1
fi

report=$(awk '/id="smoke-result"/{flag=1} flag{print} /<\/pre>/{if(flag) exit}' "$out" \
    | sed -e 's/<[^>]*>//g' -e 's/\r//g' \
    | sed -e 's/^.*SMOKE/SMOKE/' -e '/^SMOKE/,$!d')

if printf '%s' "$report" | grep -q "SMOKE PASS"; then
    printf '%s\n' "$report"
    exit 0
fi

echo "ui-smoke: FAIL" >&2
printf '%s\n' "$report" | sed -e 's/^SMOKE FAIL//' -e '/^[[:space:]]*$/d' >&2
exit 1
