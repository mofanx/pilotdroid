#!/usr/bin/env bash
# fixture-doctor.sh — one-command ApiDemos fixture restore for the
# verbatim u2 suite. Idempotent: early-OK when io.appium.android.apis
# is already installed.
#
# Pipeline:
#   1. `pm path` check → exit 0 if fixture present.
#   2. Locate/download ApiDemos-debug.apk (cached under ~/.cache).
#   3. `adb install --bypass-low-target-sdk-block` while the pilotdroid
#      agent's a11y dump races the MIUI `AdbInstallActivity` 10s
#      auto-deny countdown and inject_tap's 「继续安装」
#      (forensics: RESULTS-M17-u2suite.md §附 — the INSTALL_FAILED_
#      USER_RESTRICTED cause is the untapped dialog, not the
#      'USB install' toggle; FLAG_SECURE blocks screencap but not
#      a11y/tap).
#   4. First `monkey` launch flushes the one-shot LBE ReviewPermissions
#      page — agent a11y taps 「确定」 if it appears.
#
# Prereqs: agent running + `adb forward tcp:17932
# localabstract:pilotdroid_agent` (pilotdroid-cli -s $SERIAL setup).
set -euo pipefail
cd "$(dirname "$0")"

SERIAL="${PILOTDROID_SERIAL:-ba0d072d}"
PKG="io.appium.android.apis"
APK_URL="https://github.com/appium/java-client/raw/v7.3.0/src/test/java/io/appium/java_client/ApiDemos-debug.apk"
APK_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/pilotdroid/fixture/ApiDemos-debug.apk"
AGENT_DIR="../../../agent/scripts"
PORT="${PILOTDROID_PORT:-17932}"

say() { printf '[fixture-doctor] %s\n' "$*"; }

# --- 1. idempotent check ----------------------------------------------------
if adb -s "$SERIAL" shell pm path "$PKG" 2>/dev/null | grep -q package:; then
    say "OK: $PKG already installed — nothing to do"
    exit 0
fi
say "$PKG missing — restoring"

# --- 2. apk ------------------------------------------------------------------
mkdir -p "$(dirname "$APK_CACHE")"
if [ ! -f "$APK_CACHE" ]; then
    say "downloading ApiDemos-debug.apk → $APK_CACHE"
    curl -sfL -o "$APK_CACHE" "$APK_URL"
fi

# --- 3. install with a11y auto-confirm --------------------------------------
# forward is idempotent to set up; adb forward fails loudly if a
# *different* rule owns the port, so only add when absent.
if ! adb -s "$SERIAL" forward --list | grep -q "tcp:$PORT "; then
    adb -s "$SERIAL" forward "tcp:$PORT" localabstract:pilotdroid_agent >/dev/null
fi

adb -s "$SERIAL" push "$APK_CACHE" /data/local/tmp/fixture.apk >/dev/null

say "installing (agent a11y will auto-confirm the 10s dialog)…"
adb -s "$SERIAL" shell pm install -r -t --bypass-low-target-sdk-block \
    /data/local/tmp/fixture.apk &
INSTALL_PID=$!

PYTHONPATH="$AGENT_DIR" python3 - "$PORT" <<'PYEOF'
import re, sys, time
from agent_client import Agent

port = int(sys.argv[1])
try:
    c = Agent(port=port); c.connect()
except Exception as e:
    print(f"agent unreachable ({e}) — cannot auto-confirm; tap "
          f"「继续安装」on the device NOW (10s countdown)", flush=True)
    sys.exit(0)  # install may still succeed if user taps manually

deadline = time.time() + 20
while time.time() < deadline:
    try:
        xml = c.dump()['payload'].get('xml', '')
    except Exception:
        time.sleep(0.15); continue
    if 'securitycenter' not in xml:
        time.sleep(0.15); continue
    hit = None
    for m in re.finditer(r'text="([^"]*)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
        if m.group(1) == '继续安装':
            hit = m; break
    if hit:
        x = (int(hit.group(2)) + int(hit.group(4))) // 2
        y = (int(hit.group(3)) + int(hit.group(5))) // 2
        print(f"tapping 继续安装 @({x},{y})", flush=True)
        c.inject_tap(x, y, wait_finish=True)
        break
    time.sleep(0.15)
PYEOF

wait "$INSTALL_PID" || true
if ! adb -s "$SERIAL" shell pm path "$PKG" 2>/dev/null | grep -q package:; then
    say "FAIL: install did not land — re-run after checking the device dialog"
    exit 1
fi
say "installed OK"

# --- 4. flush the one-shot LBE ReviewPermissions gate ------------------------
say "first launch — flushing LBE review if present"
adb -s "$SERIAL" shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 \
    >/dev/null 2>&1 || true

PYTHONPATH="$AGENT_DIR" python3 - "$PORT" <<'PYEOF'
import re, sys, time
from agent_client import Agent

try:
    c = Agent(port=int(sys.argv[1])); c.connect()
except Exception:
    sys.exit(0)

# ReviewPermissionsActivity is MULTI-PAGE (clipboard/sensors page,
# then sms/contacts/location page …) — loop taps until lbe leaves
# the foreground or we run out of pages/attempts.
deadline = time.time() + 20
page = 0
while time.time() < deadline and page < 8:
    try:
        xml = c.dump()['payload'].get('xml', '')
    except Exception:
        time.sleep(0.3); continue
    if 'lbe.security.miui' not in xml:
        break
    hit = None
    for m in re.finditer(r'text="([^"]*)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
        if m.group(1) == '确定':
            hit = m; break
    if not hit:
        print("LBE page without 确定 — leaving for user", flush=True)
        break
    x = (int(hit.group(2)) + int(hit.group(4))) // 2
    y = (int(hit.group(3)) + int(hit.group(5))) // 2
    print(f"LBE review page {page} — tapping 确定 @({x},{y})", flush=True)
    c.inject_tap(x, y, wait_finish=True)
    page += 1
    time.sleep(0.8)
print("done", flush=True)
PYEOF

say "fixture restored: $PKG"
