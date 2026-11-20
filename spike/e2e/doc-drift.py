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

print(f"{'PASS' if not fails else 'FAIL'}: {len(fails)} check(s) failed" +
      (f" {fails}" if fails else ""))
sys.exit(1 if fails else 0)
