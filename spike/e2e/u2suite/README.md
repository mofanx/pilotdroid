# u2suite — verbatim openatx/uiautomator2 `mobile_tests/` vs u2shim

Upstream pin: `openatx/uiautomator2` @ `96f581d3b95efdf16105b767a5d5401c73f7ec0e`
("fix: remove directory at u2.jar path...", 2026-11-17).
`suite/` is a byte-verbatim copy of `mobile_tests/` at that commit
(30 test functions / 8 files; `skip_test_image.py` is not collected by
pytest filename convention — upstream-disabled).

## Seam (the ONLY non-verbatim piece)
`uiautomator2/__init__.py` maps `import uiautomator2` → `u2shim` and
pins `connect()`'s serial to `$PILOTDROID_SERIAL` (default ba0d072d) —
the dev box has two adb devices attached; the upstream suite assumes a
single emulator. Nothing else is patched: all M17 G-gap fixes live in
`compat/u2shim.py` itself.

## Run
```bash
PILOTDROID_SERIAL=ba0d072d ./run.sh            # full suite
PYTHONPATH=. python3 -m pytest suite/test_simple.py -v   # one file
```

## Environment deps (not shim gaps)
- ApiDemos `io.appium.android.apis` must be installed
  (targetSdk 19 → API34 needs `pm install --bypass-low-target-sdk-block`,
  plus one manual「确定」tap on MIUI's ReviewPermissionsActivity —
  `com.lbe.security.miui`, see RESULTS-M17 §4-E2).
- adb forward tcp:17932(+17933) → localabstract:pilotdroid_agent(_media).

Verdicts/attribution: ../RESULTS-M17-u2suite.md.
