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
import re, sys, os

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
IGNORE = {"close"}

serve = open(os.path.join(ROOT, "crates/pilotdroid-cli/src/serve.rs")).read()
readme = open(os.path.join(ROOT, "README.md")).read()
shim = open(os.path.join(ROOT, "compat/u2shim.py")).read()
supported = open(os.path.join(ROOT, "compat/SUPPORTED.md")).read()

fails = []


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

print(f"{'PASS' if not fails else 'FAIL'}: {len(fails)} check(s) failed" +
      (f" {fails}" if fails else ""))
sys.exit(1 if fails else 0)
