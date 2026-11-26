#!/usr/bin/env bash
# install-smoke.sh — post-publish distribution smoke (run AFTER <VER> publish)
#
# 前置：v<VER> release 已 publish（非 draft）+ PyPI 上 pilotdroid==<VER> 可见。
# 本脚本验证分发叙事端到端可用——之前所有 e2e 都跑 dev-tree，安装产物从未
# 实测过（M14 发现：whl 仅 u2shim+agent_client，无 CLI 二进制无 agent.jar，
# 纯 pip 用户必须另装 pilotdroid-cli；M15 起 whl 自带 lazy-bootstrap CLI）。
#
# 用法: spike/e2e/install-smoke.sh [serial]   每步打印 PASS/FAIL，末尾汇总。
set -u
# 被测发布版本；下个 milestone 只需改这一处（或 env 覆盖）。
VER="${PILOTDROID_SMOKE_VER:-0.8.1}"
SERIAL="${1:-}"
ADB=(adb); [ -n "$SERIAL" ] && ADB+=( -s "$SERIAL" )
V="$(mktemp -d)"; trap 'rm -rf "$V"' EXIT
pass=0; fail=0
ok()  { echo "PASS: $1"; pass=$((pass+1)); }
bad() { echo "FAIL: $1"; fail=$((fail+1)); }

echo "=== 0. PyPI index 可见性轮询（publish 后索引传播有延迟）==="
python3 -m venv "$V/venv"
for i in $(seq 1 12); do
  if "$V/venv/bin/pip" index versions pilotdroid 2>/dev/null | grep -qw "$VER"; then
    ok "pilotdroid==$VER visible on PyPI index (try $i)"; break
  fi
  [ "$i" = 12 ] && bad "pilotdroid==$VER not visible after ~60s"
  sleep 5
done

echo "=== 1. pip install pilotdroid (venv) ==="
"$V/venv/bin/pip" -q install "pilotdroid==$VER" \
  && ok "pip install pilotdroid==$VER" || bad "pip install pilotdroid==$VER"

echo "=== 2. import u2shim ==="
"$V/venv/bin/python" -c "from pilotdroid import u2shim" \
  && ok "import pilotdroid.u2shim" || bad "import pilotdroid.u2shim"

echo "=== 2b. shim 符号面（xpath-lite 打包回归防）==="
# xpath-lite 首现于 post-0.6.2 的 M23 批次（≥0.6.3 的 wheel 才含
# Device.xpath/_xp_eval）——对 ≤0.6.2 已发布版本无条件断言会误红。
# 版本门槛化：VER≥0.6.3 断言存在性（拦打包回归），否则 SKIP。
if [ "$(printf '0.6.3\n%s\n' "$VER" | sort -V | head -1)" = "0.6.3" ]; then
  "$V/venv/bin/python" - <<'PY' && ok "Device.xpath + _xp_eval present" || bad "shim xpath-lite symbols missing"
import xml.etree.ElementTree as ET
from pilotdroid import u2shim
assert callable(getattr(u2shim.Device, "xpath", None)), "Device.xpath missing"
root = ET.fromstring('<hierarchy><node text="OK" bounds="[0,0][1,1]"/></hierarchy>')
assert u2shim._xp_eval(root, "OK"), "_xp_eval shorthand miss"
PY
else
  echo "SKIP: Device.xpath check gated on VER>=0.6.3 (VER=$VER lacks xpath-lite)"
fi

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

echo "=== 4. release 二进制 --version（linux x86_64，对照步）==="
# 对照：直接 gh 拉裸二进制验证 asset 本体可用；真用户路径是 3b/5 的
# console script（走 bootstrap）。v0.6.0 draft 已删——勿回退到旧 tag。
if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
  gh release download "v$VER" -p 'pilotdroid-cli-linux-x86_64' -D "$V" --clobber \
    && chmod +x "$V/pilotdroid-cli-linux-x86_64" \
    && "$V/pilotdroid-cli-linux-x86_64" --version \
    && ok "cli --version" || bad "cli download/--version"
else
  echo "SKIP: 非 linux-x86_64 host，改手动验证对应平台 asset"
fi

echo "=== 5. CLI setup + serve MCP 握手（需设备）==="
# 真用户路径：venv console script（bootstrap 缓存命中后零下载 execv）。
CLI="$V/venv/bin/pilotdroid-cli"
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
