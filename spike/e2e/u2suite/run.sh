#!/usr/bin/env bash
# Verbatim upstream uiautomator2 mobile_tests against u2shim.
# Prereqs: `pilotdroid-cli -s $PILOTDROID_SERIAL setup` (agent +
# forwards), ApiDemos installed (`pm install --bypass-low-target-sdk-
# block` on API34+, tap through the MIUI ReviewPermissionsActivity gate
# once — see ../RESULTS-M17-u2suite.md §4-E2).
set -e
cd "$(dirname "$0")"
# Device-state preamble (forensics: RESULTS-M17 §8 — a dozing device /
# expanded NotificationShade empties the a11y tree and masquerades as
# test failures). Wake + collapse the shade; conftest's
# _device_fixture_guard then fails fast if the fixture is still broken.
SERIAL="${PILOTDROID_SERIAL:-ba0d072d}"
adb -s "$SERIAL" shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
adb -s "$SERIAL" shell cmd statusbar collapse >/dev/null 2>&1 || true
adb -s "$SERIAL" shell svc power stayon true >/dev/null 2>&1 || true
PYTHONPATH=. exec python3 -m pytest suite -v --timeout "${U2TEST_TIMEOUT:-120}" "$@"
