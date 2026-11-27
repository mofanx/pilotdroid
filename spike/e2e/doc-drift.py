#!/usr/bin/env python3
"""doc-drift — public surface claims in docs must match the code that ships.

Checks (exit 0 = PASS):
  1. serve tools: every `N tools`/`N 工具` figure in README.md equals the
     number of {"name":...} entries in serve.rs's tools() JSON array;
  2. shim methods: every public method on Device/_Selector/_Watcher/_Scroll
     in compat/u2shim.py is named somewhere in compat/SUPPORTED.md —
     the file that defines our drop-in promise ("表外调用按显式拒绝").
     Methods in IGNORE are shim-internal extras, not u2 surface:
       close — Device.close() disposes the agent conn (shim lifecycle)
  3. python/README.md xpath wording must agree with SUPPORTED.md —
     the wheel's long description is the PyPI store-front: when
     SUPPORTED marks xpath as the host-side approximation subset
     (近似子集), python/README.md must not keep the stale
     "not supported" wording and must name the approximation
     (xpath-lite/approximat/近似). Forensics: M23 found this line
     stale while SUPPORTED/README had already moved on — a structural
     miss this check now pins.
"""
import re, sys, os, subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
IGNORE = {"close"}

serve = open(os.path.join(ROOT, "crates/pilotdroid-cli/src/serve.rs")).read()
readme = open(os.path.join(ROOT, "README.md")).read()
shim = open(os.path.join(ROOT, "compat/u2shim.py")).read()
supported = open(os.path.join(ROOT, "compat/SUPPORTED.md")).read()

fails = []

changelog = open(os.path.join(ROOT, "CHANGELOG.md")).read()


def rep(name, ok, detail=""):
    if not ok:
        fails.append(name)
    print(f"{'PASS' if ok else 'FAIL'} {name}: {detail}")


# 1. tools() body spans from `fn tools()` to its closing `\n    ])` — count entries.
body = serve[serve.index("fn tools()"):]
body = body[:body.index("\n    ])")]
tools = set(re.findall(r'\{"name":"([a-z_]+)"', body))
claims = [int(n) for n in re.findall(r"(\d+)\s*(?:tools?|工具)\b", readme)]
rep("tools-claims", all(n == len(tools) for n in claims) and claims,
    f"serve.rs={len(tools)} tools; README claims={claims or 'NONE'}")

# 2. public methods (top-level `def` at class indent, minus dunders) ↔ SUPPORTED.
missing = []
for m in re.finditer(r"class (\w+)[^:]*:\n(.*?)(?=\nclass |\Z)", shim, re.S):
    names = set(re.findall(r"^    def ([a-z]\w*)\(", m.group(2), re.M))
    names |= set(re.findall(r"^    @property\n    def (\w+)\(", m.group(2), re.M))
    for n in names - IGNORE:
        if n not in supported:
            missing.append(f"{m.group(1)}.{n}")
rep("shim-methods", not missing, f"missing from SUPPORTED.md: {missing or 'none'}")

# 3. python/README.md xpath 口径 ↔ SUPPORTED.md（wheel 长描述不带假话）。
pyreadme = open(os.path.join(ROOT, "python/README.md")).read()
lite = "近似子集" in supported or "approximat" in supported.lower()
neg = re.search(r"xpath[^\n]{0,80}not supported", pyreadme, re.I)
pos = re.search(r"xpath[^\n]{0,80}(lite|approximat|近似)", pyreadme, re.I)
if lite:
    rep("pyreadme-xpath", pos is not None and neg is None,
        f"SUPPORTED=approximation; pyREADME stale-neg={bool(neg)} "
        f"approx-claim={bool(pos)}")
else:
    rep("pyreadme-xpath", True, "SUPPORTED has no approximation row; skipped")

# 4. install-smoke VER pin must equal the packaged version — a lagging
# default silently smoke-tests the PREVIOUS release (v0.6.2 stayed the
# default after 0.6.3 shipped; M25 added the PUBLISH step, this gate
# turns the human step into a CI check).
smoke = open(os.path.join(ROOT, "spike/e2e/install-smoke.sh")).read()
pyver = re.search(r'^version\s*=\s*"([^"]+)"',
                  open(os.path.join(ROOT, "python/pyproject.toml")).read(),
                  re.M)
sver = re.search(r'VER="\$\{PILOTDROID_SMOKE_VER:-([^}]+)\}"', smoke)
rep("smoke-ver", bool(pyver and sver and sver.group(1) == pyver.group(1)),
    f"install-smoke VER={sver.group(1) if sver else 'NONE'} vs "
    f"pyproject={pyver.group(1) if pyver else 'NONE'}")

# 5. CHANGELOG [Unreleased] must carry >=1 bullet once commits exist
# past the latest release tag — release notes are sourced from this
# hand-updated section and it silently lags (M26: 7 commits landed
# post-tag with zero Unreleased coverage; caught only in review, this
# gate turns the recurrence into a CI check). `git describe` can fail
# on shallow clones that never fetched tags — there we cannot prove
# staleness, so we note-and-pass rather than break CI.
tag_r = subprocess.run(["git", "-C", ROOT, "describe", "--tags",
                        "--abbrev=0"], capture_output=True, text=True)
if tag_r.returncode != 0 or not tag_r.stdout.strip():
    rep("changelog-unreleased", True,
        "no tag reachable (shallow clone?) — skipped")
else:
    tag = tag_r.stdout.strip()
    log_r = subprocess.run(["git", "-C", ROOT, "log",
                            f"{tag}..HEAD", "--oneline"],
                           capture_output=True, text=True)
    n_new = len([l for l in log_r.stdout.splitlines() if l.strip()])
    hdr = re.search(r"^## \[Unreleased\]\s*$", changelog, re.M)
    if not hdr:
        # v0.8.3 fold removed the header wholesale → this gate crashed
        # on the split; the convention is the empty header stays.
        # Match must be the header LINE — a prose mention (e.g. inside
        # a bullet) must not satisfy it (t3 review catch).
        rep("changelog-unreleased", False,
            "missing '## [Unreleased]' header — release fold must keep "
            "the empty section header")
    else:
        sec = changelog[hdr.end():].split("\n## ", 1)[0]
        n_bullets = len(re.findall(r"^\s*[-*] ", sec, re.M))
        # Release-window exemption: once Unreleased is folded into
        # `## [<current pyproject version>]` the coverage moved with it —
        # the tag just doesn't exist yet (v0.6.4 bump commit hit FAIL here).
        folded = bool(pyver and re.search(
            rf"^## \[{re.escape(pyver.group(1))}\]", changelog, re.M))
        rep("changelog-unreleased",
            n_new == 0 or n_bullets >= 1 or folded,
            f"{n_new} commit(s) since {tag}; Unreleased bullets={n_bullets}"
            + ("; folded into current version" if folded else ""))

print(f"{'PASS' if not fails else 'FAIL'}: {len(fails)} check(s) failed" +
      (f" {fails}" if fails else ""))
sys.exit(1 if fails else 0)
