#!/usr/bin/env bash
# Spike 01: device identity + environment
# Usage: ./01-env-info.sh [serial]
set -euo pipefail
DEV="${1:-${DEV:-}}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")

adb "${SEL[@]}" shell '
echo "=== model ===";      getprop ro.product.model; getprop ro.product.brand; getprop ro.product.device
echo "=== android ===";    getprop ro.build.version.release; getprop ro.build.version.sdk; getprop ro.build.version.security_patch
echo "=== selinux ===";    getenforce
echo "=== id ===";         id
echo "=== abi ===";        getprop ro.product.cpu.abi; getprop ro.product.cpu.abilist
echo "=== display ===";    wm size; wm density
echo "=== app_process ==="; which app_process; ls -la /system/bin/app_process*
echo "=== java runtime ==="; which dalvikvm; getprop persist.sys.dalvik.vm.lib.2 2>/dev/null; getprop dalvik.vm.usejitprofiles 2>/dev/null
'
