# pilotdroid (python)

uiautomator2-compatible Python client over the pilotdroid device agent.
Persistent socket, no per-op uiautomator server — ~10x device-op latency vs
stock u2 (see repo `spike/e2e/RESULTS-*.md`).

mcp-name: io.github.mofanx/pilotdroid

## Install

```bash
pip install ./python            # local checkout
pip install msgpack             # sole runtime dep (auto-installed by pip)
```

## Deploy agent + connect

```bash
./agent/scripts/deploy-agent.sh <serial>   # pushes jar, starts app_process agent
adb forward tcp:17932 localabstract:pilotdroid_agent
```

```python
import pilotdroid
d = pilotdroid.connect(port=17932, serial="ba0d072d")

d(text="OK").click()
d(className="android.widget.TextView", index=1).exists   # property, u2-style
d(scrollable=True).scroll.forward()
d.app_start("com.android.settings"); d.app_current()
d.set_text("你好")        # ACTION_SET_TEXT — unicode works, no IME needed
d.screenshot("shot.jpg", scale=0.5)
```

## Supported selector keys

`text, resourceId, description, className, packageName, scrollable, index`
(AND semantics; `index` = nth match). `xpath` is **host-side
evaluation** (xpath-lite, upgraded Track B: one a11y dump + a
stdlib-only evaluator per query — text/`@id`/`%fuzzy%`/`//`,`/` steps,
axes `child`/`descendant`/`ancestor`/`parent`/`following-sibling`/
`preceding-sibling`, predicates `[@a='v']`/`contains()`/`and`/`or`/
`[n]`/union `|`, `parent()`/`scroll_to`; `last()`/`position()`/
`text()`/non-sibling `following::` etc raise `NotImplementedError`.
Not the u2 plugin pipeline).

See `compat/u2shim.py` docstring for the full covered API surface.
