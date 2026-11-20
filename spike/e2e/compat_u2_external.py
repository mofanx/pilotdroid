#!/usr/bin/env python3
"""M13 external drop-in evidence: third-party u2 scripts run VERBATIM
against u2shim — the only seam is `sys.modules['uiautomator2'] = u2shim`.

Suite: Themis benchmark (github.com/the-themis-benchmarks/home,
ESEC/FSE 2021 — real-world crash-repro scripts written for stock
uiautomator2, each paired with a version-locked debug APK).
Scripts live verbatim in ./themis/ (sha256 logged per run); APKs are
NOT committed — fetch via the manifest's repo path.

Per-script verdicts (the compat claim is about API compatibility, NOT
bug reproduction):
  ok          script ran to its end — every u2 call executed
  unsupported hit a shim-declared-unsupported API (xpath etc.)
  miss        LookupError — selector found no node (API fine, UI state
              differed: same as u2's UiObjectNotFoundError outcome)
  timeout     exceeded --timeout s (scripts poll app state in while-loops)
  error       any other exception (traceback tail recorded)

Usage: compat_u2_external.py <serial> [--apk-dir DIR] [--timeout S] [--only SUBSTR]
"""
import hashlib, json, os, re, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../.."))
THEMIS = os.path.join(HERE, "themis")
GH = "repos/the-themis-benchmarks/home/contents/"

# script filename -> repo APK path (version-locked pairing)
MANIFEST = {
    "APhotoManager_script-#116.py":
        "APhotoManager/APhotoManager-0.6.4.180314-debug-#116.apk",
    "geohashdroid_script-#73.py": "geohashdroid/geohashdroid-0.9.4-#73.apk",
    "sunflower_script-#239.py": "sunflower/sunflower-0.1.6-#239.apk",
    "ActivityDiary_script-#285.py":
        "ActivityDiary/ActivityDiary-1.4.0-debug-#285.apk",
    "Phonograph_script-#112.py": "Phonograph/Phonograph-0.15.0-#112.apk",
    "AmazeFileManager_script-#1232.py":
        "AmazeFileManager/AmazeFileManager-3.2.1-#1232.apk",
    "AmazeFileManager_script-#3333.py":
        "AmazeFileManager/AmazeFileManager-3.7.0-#3333.apk",
    "Markor_script-#1698.py": "Markor/markor-2.8.5-#1698.apk",
    "Markor_script-#264.py": "Markor/markor-0.3.9-#264.apk",
    "Scarlet-Notes_script-#114.py":
        "Scarlet-Notes/Scarlet-Notes-6.9.5-#114.apk",
    "and-bible_script-#375.py": "and-bible/and-bible-build-309-#375.apk",
    "commons_script-#1581.py": "commons/commons-2.7.1-#1581.apk",
    "AnkiDroid_script-#5756.py": "AnkiDroid/AnkiDroid-debug-2.9.4-#5756.apk",
    "osmeditor4android_script-#637.py":
        "osmeditor4android/osmeditor4android-debug-#637.apk",
}

CHILD = r"""
import runpy, sys, os
sys.path.insert(0, os.environ["PILOTDROID_COMPAT"])
import u2shim
sys.modules["uiautomator2"] = u2shim
script = sys.argv[1]
sys.argv = [sys.argv[1], sys.argv[2]]
runpy.run_path(script, run_name="__main__")
"""

UNSUPPORTED = ("unsupported keys", "not supported", "unsupported")

# MIUI interstitials that stock-AVD scripts never anticipate. The
# babysitter must NOT hold an agent control conn (single-client agent:
# a second conn steals the script's session — observed ConnectionError
# ping-pong). Detection via `dumpsys window` focus pkg + `input tap`
# at measured dialog coords — pure adb, zero agent surface:
_BABY_TAPS = [
    # focused window "pkg/Activity" substring -> (x, y) on 1440x3200
    ("packageinstaller", (414, 2920)),      # install confirm: 继续安装
    ("securitycenter", (414, 2920)),        # MIUI AdbInstallActivity
    ("GrantPermissionsActivity", (720, 2690)),  # runtime grant: 始终允许
    ("ReviewPermissionsActivity", (1038, 3046)),  # old-sdk review: 确定
]


def _babysit(serial, stop):
    """Environment babysitter: taps MIUI install-confirm and
    old-targetSdk permission-review screens so verbatim scripts can
    proceed. Restricted to system-installer focused windows — app
    dialogs are never touched (would corrupt bug-repro semantics)."""
    last = 0.0
    while not stop.is_set():
        try:
            out = sh(serial, "shell", "dumpsys", "window",
                     timeout=20).stdout
        except Exception:
            stop.wait(0.8)
            continue
        m = re.search(r"mCurrentFocus=Window\{[^ ]+ u0 ([^ ]+)\}", out)
        focus = m.group(1) if m else ""
        for pat, (x, y) in _BABY_TAPS:
            if pat in focus and time.time() - last > 2.0:
                sh(serial, "shell", "input", "tap", str(x), str(y))
                last = time.time()
                break
        stop.wait(0.8)


def sh(serial, *args, timeout=120):
    return subprocess.run(["adb", "-s", serial] + list(args),
                          capture_output=True, text=True, timeout=timeout)


def ensure_agent(serial):
    """Service may have been stopped by a previous script's
    d.service('uiautomator').stop() — relaunch if pgrep misses."""
    out = sh(serial, "shell", "pgrep -f pilotdroid.Agent").stdout.strip()
    if out:
        return
    sh(serial, "shell",
       "nohup setsid env CLASSPATH=/data/local/tmp/pilotdroid/agent.jar "
       "app_process /data/local/tmp/pilotdroid pilotdroid.Agent "
       "</dev/null >/data/local/tmp/pilotdroid/agent.log 2>&1 &")
    for _ in range(40):
        time.sleep(0.25)
        if sh(serial, "shell", "pgrep -f pilotdroid.Agent").stdout.strip():
            return


def install(serial, apk, script, timeout_s=180):
    """MIUI blocks `adb install` (INSTALL_FAILED_USER_RESTRICTED) and API34
    rejects targetSdk<23 — push + `pm install --bypass-low-target-sdk-block`;
    the resident babysitter taps the MIUI '继续安装' confirm.
    Scripts are version-locked to their APK — pm `-d` downgrade is not
    honored on API34, so uninstall the target pkg first."""
    m = re.search(r'app_start\("([^"]+)"', open(script).read())
    if m:
        sh(serial, "shell", "pm", "uninstall", m.group(1))
    sh(serial, "push", apk, "/data/local/tmp/themis.apk", timeout=120)
    p = subprocess.Popen(
        ["adb", "-s", serial, "shell", "pm", "install", "-r",
         "--bypass-low-target-sdk-block", "/data/local/tmp/themis.apk"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        out = p.communicate(timeout=timeout_s)[0]
    except subprocess.TimeoutExpired:
        p.kill()
        return False, "install timeout"
    return "Success" in out, out.strip().splitlines()[-1][:100]


def classify(rc, out):
    tail = out.strip().splitlines()[-8:]
    tb = "\n".join(tail)
    if rc == 0:
        return "ok", ""
    if any(k in tb for k in UNSUPPORTED) or "xpath" in tb.lower():
        m = re.search(r"(unsupported keys \[[^\]]*\]|xpath[\w: .]*|"
                      r"(?:AttributeError|ValueError)[^\n]*)", tb)
        return "unsupported", (m.group(0) if m else tail[-1])[:120]
    if "LookupError" in tb or "UiObjectNotFound" in tb:
        return "miss", tail[-1][:120]
    return "error", tail[-1][:120] if tail else "no output"


def run(serial, apk_dir, timeout_s, only):
    env = dict(os.environ, PILOTDROID_COMPAT=os.path.join(ROOT, "compat"))
    stop = threading.Event()
    baby = threading.Thread(target=_babysit, args=(serial, stop),
                            daemon=True)
    baby.start()
    rows = []
    try:
        rows = _run_all(serial, apk_dir, timeout_s, only, env)
    finally:
        stop.set()
    return rows


def _run_all(serial, apk_dir, timeout_s, only, env):
    rows = []
    for fname, apk_repo in MANIFEST.items():
        if only and only not in fname:
            continue
        script = os.path.join(THEMIS, fname)
        sha = hashlib.sha256(open(script, "rb").read()).hexdigest()[:12]
        apk = os.path.join(apk_dir, os.path.basename(apk_repo))
        ensure_agent(serial)
        ins, ins_err = install(serial, apk, script)
        t0 = time.time()
        if not ins:
            rows.append({"script": fname, "sha": sha, "verdict": "error",
                         "fail": f"install: {ins_err}", "steps": 0,
                         "elapsed_s": 0})
            print(f"  error      {fname} :: install: {ins_err}")
            continue
        try:
            p = subprocess.run(
                [sys.executable, "-c", CHILD, script, serial],
                capture_output=True, text=True, timeout=timeout_s, env=env)
            out = p.stdout + p.stderr
            verdict, fail = classify(p.returncode, out)
        except subprocess.TimeoutExpired as e:
            out = (e.stdout or b"").decode("utf-8", "replace") \
                if isinstance(e.stdout, bytes) else (e.stdout or "")
            verdict, fail = "timeout", ""
        steps = len(re.findall(r"success", out, re.I))
        el = round(time.time() - t0, 1)
        rows.append({"script": fname, "sha": sha, "verdict": verdict,
                     "fail": fail, "steps": steps, "elapsed_s": el})
        print(f"  {verdict:12s} {fname} :: steps={steps} "
              f"{el}s {fail[:70]}")
    # restore: stop service loop leaves agent dead on some paths
    ensure_agent(serial)
    return rows


def main():
    serial = sys.argv[1] if len(sys.argv) > 1 else "ba0d072d"
    apk_dir = "/tmp/themis-apks"
    timeout_s = 240
    only = None
    for i, a in enumerate(sys.argv[2:], 2):
        if a == "--apk-dir": apk_dir = sys.argv[i + 1]
        elif a == "--timeout": timeout_s = int(sys.argv[i + 1])
        elif a == "--only": only = sys.argv[i + 1]
    rows = run(serial, apk_dir, timeout_s, only)
    agg = {}
    for r in rows:
        agg[r["verdict"]] = agg.get(r["verdict"], 0) + 1
    res = {"suite": "themis-benchmarks/home", "device": serial,
           "total": len(rows), "verdicts": agg, "rows": rows}
    outp = os.path.join(HERE, "e2e-m13-themis.json")
    json.dump(res, open(outp, "w"), indent=1, ensure_ascii=False)
    print(f"\nsummary: {agg} -> {outp}")
    sys.exit(0)


if __name__ == "__main__":
    main()
