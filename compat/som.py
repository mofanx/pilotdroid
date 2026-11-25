#!/usr/bin/env python3
"""SoM-lite host-side perception for pilotdroid (Track A, shim面先行).

`describe_screen(dev)` serializes the accessibility tree into a
token-budget-friendly map: dense element ids (deterministic doc order)
↔ bounds center ↔ key attributes. `screenshot_annotated(dev)` draws
the same numbered ids on the screenshot JPEG — one numbering space,
shared by both, so an LLM can say "tap #12" and the coordinate is
recoverable from either surface.

Only nodes useful for grounding are numbered: anything carrying
text/content-desc/resource-id, or an interactive flag (clickable,
scrollable, checkable, focusable, long-clickable). Layout-only frames
are omitted — they add tokens, not affordances.

MCP `serve` exposure is a later wave; this module is the host-side
foundation (compat/ ships inside the `pilotdroid` wheel via the
python/pilotdroid symlink, same as u2shim.py).
"""
import io
import re
import xml.etree.ElementTree as ET

_ATTRS = ("text", "content-desc", "resource-id")
_FLAGS = ("clickable", "scrollable", "checkable", "focusable",
          "long-clickable")
_BOUNDS_RE = re.compile(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]")
_MAX_LABEL = 28  # chars kept per text/desc in the text rendering
_PALETTE = ((234, 67, 53), (52, 168, 83), (66, 133, 244),
            (251, 188, 4), (171, 71, 188), (0, 172, 193))


def _parse_bounds(s):
    m = _BOUNDS_RE.fullmatch(s or "")
    if not m:
        return None
    l, t, r, b = (int(g) for g in m.groups())
    return (l, t, r, b)


def _useful(attrib):
    return any(attrib.get(a) for a in _ATTRS) or \
        any(attrib.get(f) == "true" for f in _FLAGS)


def _enumerate(xml):
    """Filtered <node> list in document order with dense 1-based ids.

    Same function backs describe_screen and screenshot_annotated —
    id stability between the two surfaces is by construction.
    """
    out = []
    for node in ET.fromstring(xml).iter("node"):
        a = node.attrib
        if not _useful(a):
            continue
        b = _parse_bounds(a.get("bounds"))
        out.append({
            "id": len(out) + 1,
            "text": a.get("text", ""),
            "desc": a.get("content-desc", ""),
            "rid": a.get("resource-id", ""),
            "class": a.get("class", ""),
            "checked": a.get("checked") == "true",
            "selected": a.get("selected") == "true",
            "enabled": a.get("enabled", "true") != "false",
            "focused": a.get("focused") == "true",
            "clickable": a.get("clickable") == "true",
            "bounds": b,
            "center": [(b[0] + b[2]) // 2, (b[1] + b[3]) // 2] if b else None,
        })
    return out


def describe_screen(dev, max_label=_MAX_LABEL):
    """-> {"elements": [...], "text": "<compact map>"}.

    Text format, one line per element:
      `#12 [540,980] "继续安装" pkg:id/btn cls=android.widget.Button C`
    flags: C=clickable (ids are the SoM handles; centers are tap coords).
    """
    elements = _enumerate(dev.dump_hierarchy())
    lines = []
    for e in elements:
        label = e["text"] or e["desc"]
        if len(label) > max_label:
            label = label[:max_label - 1] + "…"
        parts = [f"#{e['id']}",
                 f"[{e['center'][0]},{e['center'][1]}]" if e["center"]
                 else "[-,-]"]
        if label:
            parts.append(f'"{label}"')
        if e["rid"]:
            parts.append(e["rid"])
        parts.append(e["class"].rsplit(".", 1)[-1])
        if e["clickable"]:
            parts.append("C")
        if e.get("checked"):
            parts.append("K")
        if e.get("selected"):
            parts.append("S")
        if e.get("focused"):
            parts.append("F")
        if not e.get("enabled", True):
            parts.append("D")
        lines.append(" ".join(parts))
    # Same convention as the Rust sibling + list_elements: simple
    # class name; state flags emitted non-default only (true flags, or
    # enabled:false when disabled) — keeps the payload compact.
    for e in elements:
        e["class"] = e["class"].rsplit(".", 1)[-1]
        for k in ("checked", "selected", "focused"):
            if not e.get(k):
                e.pop(k, None)
        if e.get("enabled", True):
            e.pop("enabled", None)
    header = f"# screen: {len(elements)} elements (id [cx,cy] " \
             f"label rid class C=clickable K=checked S=selected " \
             f"F=focused D=disabled)"
    return {"elements": elements,
            "text": "\n".join([header] + lines)}


def screenshot_annotated(dev, path=None, quality=80):
    """JPEG bytes with numbered boxes drawn on the described elements.

    `scale` is fixed at 1.0 — bounds are display pixels; a scaled shot
    would need rescaling math the agent doesn't contractually provide.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        raise ImportError("screenshot_annotated needs Pillow "
                          "(`pip install pillow`)")
    elements = _enumerate(dev.dump_hierarchy())
    img = Image.open(io.BytesIO(dev.screenshot(quality=quality,
                                             scale=1.0))).convert("RGB")
    draw = ImageDraw.Draw(img)
    for e in elements:
        if not e["bounds"]:
            continue
        color = _PALETTE[(e["id"] - 1) % len(_PALETTE)]
        l, t, r, b = e["bounds"]
        # Real dumps carry inverted/degenerate bounds (MIUI emits e.g.
        # [92,3236][217,3200] for bottom-clipped nodes — p2 measured
        # 27/97 on one screen). PIL raises ValueError on y1<y0/x1<x0;
        # normalize, and skip true zero-area boxes (nothing to mark).
        l, r = min(l, r), max(l, r)
        t, b = min(t, b), max(t, b)
        if r <= l or b <= t:
            continue
        draw.rectangle([l, t, r, b], outline=color, width=3)
        tag = str(e["id"])
        tw, th = draw.textlength(tag), 10
        draw.rectangle([l, max(0, t - th - 4),
                        l + tw + 8, max(th + 4, t)],
                       fill=color)
        draw.text((l + 4, max(0, t - th - 2)), tag, fill=(255, 255, 255))
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=quality)
    data = buf.getvalue()
    if path:
        with open(path, "wb") as f:
            f.write(data)
    return data
