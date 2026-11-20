# Security Policy

## Supported Versions

| Version | Supported |
|---|---|
| 0.5.x | ✅ |

## Reporting a Vulnerability

Please report vulnerabilities via **GitHub private vulnerability reporting**
(Repository → Security → Report a vulnerability) or by opening a
[security advisory](https://github.com/mofanx/pilotdroid/security/advisories/new).
Do NOT file public issues for exploitable flaws.

Expected response: acknowledgement within ~1 week. This is a hobby/research
project — no SLA or bounty is offered.

## Threat-model notes (read before deploying)

- The on-device agent listens on `localabstract:pilotdroid_agent`, reachable
  only to the shell user / `adb forward` — the same exposure model as scrcpy
  and uiautomator2. There is **no additional authentication** on the control
  protocol: anyone who can `adb forward` to your device controls input
  injection and screenshot capture. Treat `adb` access as root-equivalent.
- Single-client semantics (ADR-007): a second control connection replaces
  the first. A hostile local process with adb access can take over the
  session — again identical to adb-level trust.
- The CLI embeds `agent/agent.jar` verbatim; releases ship `SHA256SUMS.txt`
  — verify before deploying artifacts to devices.
