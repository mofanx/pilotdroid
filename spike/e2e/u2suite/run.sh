#!/usr/bin/env bash
# Verbatim upstream uiautomator2 mobile_tests against u2shim.
# Prereqs: `pilotdroid-cli -s $PILOTDROID_SERIAL setup` (agent +
# forwards), ApiDemos installed (`pm install --bypass-low-target-sdk-
# block` on API34+, tap through the MIUI ReviewPermissionsActivity gate
# once — see ../RESULTS-M17-u2suite.md §4-E2).
set -e
cd "$(dirname "$0")"
PYTHONPATH=. exec python3 -m pytest suite -v --timeout "${U2TEST_TIMEOUT:-120}" "$@"
