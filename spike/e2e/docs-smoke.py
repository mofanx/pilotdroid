#!/usr/bin/env python3
"""docs-smoke — README's copy-paste surface must stay literally runnable.

Checks (exit 0 = PASS):
  1. the mcpServers ```json block parses and points at a real binary +
     real subcommand (command "pilotdroid-cli", args ["serve", ...]);
  2. `pilotdroid-cli serve --help` exits 0 and documents `-s/--device`
     (the multi-device arg the README tells users to add);
  3. every inline `pilotdroid-cli <sub>` mention in the README names a
     subcommand that `pilotdroid-cli --help` actually lists.

Binary resolution: $PILOTDROID_CLI, else ./target/debug/pilotdroid-cli,
else PATH. Usage: python3 docs-smoke.py [path/to/README.md]
"""
import json, os, re, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
README = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "README.md")

fails = []


def rep(name, ok, detail=""):
    if not ok:
        fails.append(name)
    print(f"{'PASS' if ok else 'FAIL'} {name}: {detail}")


def cli_path():
    for c in (os.environ.get("PILOTDROID_CLI"),
              os.path.join(ROOT, "target", "debug", "pilotdroid-cli"),
              shutil.which("pilotdroid-cli")):
        if c and os.path.exists(c):
            return c
    return None


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=30)


def main():
    text = open(README, encoding="utf-8").read()

    # 1. mcpServers block: parse + point at the real binary/subcommand.
    blocks = re.findall(r"```json\s*\n(.*?)```", text, re.S)
    mcp = None
    for b in blocks:
        try:
            doc = json.loads(b)
        except json.JSONDecodeError:
            continue
        if "mcpServers" in doc:
            mcp = doc["mcpServers"]
    rep("mcpServers_block_present", mcp is not None)
    entry = (mcp or {}).get("pilotdroid", {})
    rep("mcpServers_command", entry.get("command") == "pilotdroid-cli",
        f"command={entry.get('command')!r}")
    args = entry.get("args", [])
    rep("mcpServers_args", args[:1] == ["serve"], f"args={args!r}")

    cli = cli_path()
    rep("cli_binary_found", cli is not None, cli or "not found")
    if not cli:
        print("DOCS-SMOKE", "FAIL")
        sys.exit(1)

    # 2. serve is a real subcommand exposing -s/--device.
    h = run([cli, "serve", "--help"])
    rep("serve_help", h.returncode == 0, h.stderr.strip()[:80])
    rep("serve_serial_opt", "--device" in h.stdout or "-s" in h.stdout)

    # 3. inline `pilotdroid-cli <sub>` mentions vs real subcommands.
    top = run([cli, "--help"])
    subs = set(re.findall(r"^  ([a-z-]+)\s", top.stdout, re.M))
    mentioned = set(re.findall(r"`pilotdroid-cli ([a-z-]+)", text))
    bad = mentioned - subs
    rep("readme_subcommands_exist", not bad,
        f"mentioned={sorted(mentioned)} unknown={sorted(bad)}")

    # 4. doc-drift: README tool count ↔ serve.rs, shim methods ↔ SUPPORTED.md.
    dd = run([sys.executable, os.path.join(os.path.dirname(
        os.path.abspath(__file__)), "doc-drift.py")])
    rep("doc_drift", dd.returncode == 0,
        (dd.stdout + dd.stderr).strip().splitlines()[-1])

    print("DOCS-SMOKE", "PASS" if not fails else "FAIL")
    sys.exit(0 if not fails else 1)


if __name__ == "__main__":
    main()
