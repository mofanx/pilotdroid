# coding: utf-8

import adbutils
import pytest

import uiautomator2 as u2


# --- pilotdroid harness guard (not upstream): fail FAST on a broken
# device fixture instead of burning 20s-per-test wait timeouts and
# being misread as code regressions. Forensics: RESULTS-M17 §8 —
# watch_context went 3/3-stable-fail because io.appium.android.apis
# got uninstalled AND the device kept dozing into the MIUI pictorial
# lockscreen with an expanded NotificationShade (empty a11y tree).
_FIXTURE_PKG = "io.appium.android.apis"


def _fixture_precheck(serial):
    """Returns a broken-reason string, or None when the fixture is sane."""
    import os
    import subprocess

    def sh(*args):
        try:
            r = subprocess.run(["adb", "-s", serial, "shell"] + list(args),
                               capture_output=True, text=True, timeout=15)
            return r.stdout, r.returncode
        except Exception as e:
            return "", f"adb error: {e}"

    out, rc = sh("pm", "path", _FIXTURE_PKG)
    if rc != 0 or "package:" not in out:
        return (f"{_FIXTURE_PKG} not installed (pm path rc={rc}) — "
                f"run `spike/e2e/u2suite/fixture-doctor.sh` to restore "
                f"(auto-confirms the MIUI AdbInstallActivity dialog "
                f"via agent a11y)")
    out, _ = sh("dumpsys", "power")
    for line in out.splitlines():
        if "mWakefulness=" in line:
            state = line.split("=", 1)[1].strip()
            if state != "Awake":
                return (f"device not awake (mWakefulness={state}) — "
                        f"a11y tree is empty under doze; run.sh wakes "
                        f"the device, or press power manually")
            break
    out, _ = sh("dumpsys", "window")
    # NotificationShade the WINDOW exists in every dumpsys listing — the
    # failure signature is it HOLDING mCurrentFocus (expanded shade) or
    # the keyguard being up at all (a11y tree then reports the
    # lockscreen, not the app).
    if ("isKeyguardShowing=true" in out
            or any("NotificationShade" in l
                   for l in out.splitlines()
                   if "mCurrentFocus" in l or "mFocusedWindow" in l)):
        return ("keyguard/NotificationShade covering screen (MIUI "
                "pictorial lockscreen) — unlock and collapse the shade "
                "first (run.sh preamble does KEYCODE_WAKEUP + statusbar "
                "collapse)")
    return None


@pytest.fixture(scope="session", autouse=True)
def _device_fixture_guard():
    """Session-level precheck: fixture-broken reports in <1s."""
    import os
    serial = os.environ.get("PILOTDROID_SERIAL", "ba0d072d")
    reason = _fixture_precheck(serial)
    if reason:
        pytest.exit(f"fixture broken: {reason}", returncode=2)


@pytest.fixture(scope="module")
def d(device):
    _d = device
    _d.settings['operation_delay'] = (0.2, 0.2)
    _d.settings['operation_delay_methods'] = ['click', 'swipe']
    return _d


@pytest.fixture
def package_name():
    return "io.appium.android.apis"


@pytest.fixture(scope="function")
def dev(d: u2.Device, package_name) -> u2.Device: # type: ignore
    d.watcher.reset()
    
    d.app_start(package_name, stop=True)
    yield d


# run parallel
# py.test --tx "3*popen" --dist=load test_device.py -q --tb=line

#def read_device_list() -> list:
#    return [v.serial for v in adbutils.adb.device_list()]


#def pytest_configure(config):
#     # read device list if we are on the master
#     if not hasattr(config, "slaveinput"):
#        config.devlist = read_device_list()


# def pytest_configure_node(node):
#     # the master for each node fills slaveinput dictionary
#     # which pytest-xdist will transfer to the subprocess
#     serial = node.slaveinput["serial"] = node.config.devlist.pop()
#     node.config.devlist.insert(0, serial)


@pytest.fixture(scope="session")
def device(request):
    return u2.connect()
    # slaveinput = getattr(request.config, "slaveinput", None)
    # if slaveinput is None: # single-process execution
    #     serial = read_device_list()[0]
    # else: # running in a subprocess here
    #     serial = slaveinput["serial"]
    # print("SERIAL:", serial)
    # return u2.connect(serial)
