# M1 device agent — re-run guide

Device: `ba0d072d` (Xiaomi Mi 11, Android 14). Agent socket:
`localabstract:pilotdroid_agent` (note: NOT `pilotdroid` — that name is
reserved for the Rust host forward).

## Build + deploy + launch

```bash
./deploy-agent.sh ba0d072d          # javac(android-34)+d8, push jar, setsid app_process
./deploy-agent.sh ba0d072d --kill-only   # stop agent
```

## Verify (6 behavior assertions + latency bench)

```bash
./verify-agent.sh ba0d072d 17932    # forward tcp:17932, run assertions
# expect: 6 pass / 0 fail; bench JSON -> agent/data/verify-<serial>.json
```

## Manual probing

```bash
adb -s ba0d072d forward tcp:17932 localabstract:pilotdroid_agent
python3 agent_client.py --port 17932 hello
python3 agent_client.py --port 17932 inject-tap 700 1600
python3 agent_client.py --port 17932 dump > dump.xml
python3 agent_client.py --port 17932 shot shot.jpg
python3 agent_client.py --port 17932 bench 30
python3 agent_client.py --port 17932 stop      # AGENT_STOP
```

## Measured (ba0d072d, verify-ba0d072d.json)

| op | e2e p50 | on-device p50 |
|---|---|---|
| inject tap | 45.0ms | 2.7ms |
| inject swipe (300ms,11ev) | 11.5ms | 8.8ms |
| dump (~10-16KB xml) | 49.9ms | 7.0ms |
| shot jpeg | 100.1ms | ~24ms cap + jpeg |
| ping | 42.0ms | ~0ms |

e2e ≈ on-device + ~40ms adb-over-USB transport floor on MIUI (bare
`adb shell echo` RTT is 20-30ms). BATCH opcode amortizes this.
Side-effect evidence: MIUI logs `MIUIInput: Input event injection ...`
per injected MotionEvent (grep `njection from package` in logcat).
