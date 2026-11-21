#!/usr/bin/env bash
# install-smoke.sh — post-publish distribution smoke (run AFTER v0.6.0 publish)
#
# 前置：v0.6.0 release 已 publish（非 draft）+ PyPI 上 pilotdroid==0.6.0 可见。
# 本脚本验证分发叙事端到端可用——之前所有 e2e 都跑 dev-tree，安装产物从未
# 实测过（M14 发现：whl 仅 u2shim+agent_client，无 CLI 二进制无 agent.jar，
# 纯 pip 用户必须另装 pilotdroid-cli）。
#
# 用法: spike/e2e/install-smoke.sh [serial]   每步打印 PASS/FAIL，末尾汇总。
set -u
SERIAL="${1:-}"
ADB=(adb); [ -n "$SERIAL" ] && ADB+=( -s "$SERIAL" )
V="$(mktemp -d)"; trap 'rm -rf "$V"' EXIT
pass=0; fail=0
ok()  { echo "PASS: $1"; pass=$((pass+1)); }
bad() { echo "FAIL: $1"; fail=$((fail+1)); }

echo "=== 1. pip install pilotdroid (venv) ==="
python3 -m venv "$V/venv" && "$V/venv/bin/pip" -q install "pilotdroid==0.6.0" \
  && ok "pip install pilotdroid==0.6.0" || bad "pip install pilotdroid==0.6.0"

echo "=== 2. import u2shim ==="
"$V/venv/bin/python" -c "from pilotdroid import u2shim" \
  && ok "import pilotdroid.u2shim" || bad "import pilotdroid.u2shim"

echo "=== 3. pip-only 用户的错误提示可行动（应指到 pilotdroid-cli 而非 pilotdroid setup）==="
"$V/venv/bin/python" - <<'PY' && ok "connect error message actionable" || bad "connect error message still dead-ends"
import inspect
from pilotdroid import u2shim
src = inspect.getsource(u2shim._detect_port)
assert "pilotdroid-cli" in src, "error path lacks pilotdroid-cli pointer"
assert "run `pilotdroid setup`" not in src, "dead-end hint still present"
PY

echo "=== 3b. bootstrap 真链：venv 内 pilotdroid-cli --version ==="
# 真用户旅程：pip 装的 console script 首跑下载→SHA256SUMS 校验→
# ~/.cache 缓存→os.execv。这是 whl lazy-bootstrap 的唯一端到端验
# （单测全 mock，真 URL 仅此处）。step4 的 gh download 仅作对照。
if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
  "$V/venv/bin/pilotdroid-cli" --version \
    && ok "bootstrap download→verify→exec" || bad "bootstrap chain"
else
  echo "SKIP: bootstrap 下载路径按 host platform 取 asset，本机非 linux-x86_64"
fi

echo "=== 4. release 二进制 --version（linux x86_64）==="
if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
  gh release download v0.6.0 -p 'pilotdroid-cli-linux-x86_64' -D "$V" --clobber \
    && chmod +x "$V/pilotdroid-cli-linux-x86_64" \
    && "$V/pilotdroid-cli-linux-x86_64" --version \
    && ok "cli --version" || bad "cli download/--version"
else
  echo "SKIP: 非 linux-x86_64 host，改手动验证对应平台 asset"
fi

echo "=== 5. CLI setup + serve MCP 握手（需设备）==="
CLI="$V/pilotdroid-cli-linux-x86_64"
if [ -n "$SERIAL" ] && "${ADB[@]}" get-state >/dev/null 2>&1 && [ -x "$CLI" ]; then
  "$CLI" -s "$SERIAL" setup && ok "cli setup" || bad "cli setup"
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    | "$CLI" -s "$SERIAL" serve 2>/dev/null | grep -q '"tools"' \
    && ok "serve MCP handshake + tools/list" || bad "serve MCP handshake"
else
  echo "SKIP: 无 serial/设备或 CLI 未就绪"
fi

echo "=== summary: $pass pass / $fail fail ==="
[ "$fail" -eq 0 ]
