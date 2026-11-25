#!/usr/bin/env python3
"""OCR host spike (M32 t3) — describe_screen ocr:true feasibility.

Pipeline (host only — no agent changes):
  1. serve `screenshot` -> local png/jpeg file
  2. local `tesseract <img> stdout --psm 11 tsv` -> word boxes
  3. merge word boxes -> line blocks (same block/par/line)
  4. fuse with the accessibility tree (`list_elements`):
       * tree elements whose bounds contain an OCR block centre and
         carry no text get `ocr_text` filled in;
       * OCR blocks with no containing tree element become synthetic
         `ocr_only` elements (index continues past the tree range) so a
         blind agent can number-tap canvas/image text.

Run:  `python3 ocr_spike.py`            # screenshot current screen
      `python3 ocr_spike.py img.png`    # reuse an existing screenshot
"""
import json
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loop import Serve

MIN_CONF = 40.0


def ocr_blocks(png):
    """Tesseract TSV -> merged line blocks [{text,bounds,conf}]."""
    out = subprocess.run(
        ["tesseract", png, "stdout", "--psm", "11", "tsv"],
        capture_output=True, text=True).stdout
    words = {}
    for ln in out.splitlines()[1:]:
        c = ln.split("\t")
        if len(c) < 12 or c[0] != "5":          # level 5 == word
            continue
        try:
            conf = float(c[10])
        except ValueError:
            continue
        txt = c[11].strip()
        if conf < MIN_CONF or not any(ch.isalnum() for ch in txt):
            continue
        key = (c[2], c[3], c[4])                # block,par,line
        x, y, w, h = map(int, c[6:10])
        wds = words.setdefault(key, [])
        wds.append((x, y, w, h, conf, txt))
    blocks = []
    for ws in words.values():
        ws.sort()
        l = min(w[0] for w in ws)
        t = min(w[1] for w in ws)
        r = max(w[0] + w[2] for w in ws)
        b = max(w[1] + w[3] for w in ws)
        blocks.append({
            "text": " ".join(w[5] for w in ws),
            "bounds": [l, t, r, b],
            "conf": round(sum(w[4] for w in ws) / len(ws), 1),
        })
    return blocks


def _contains(outer, pt):
    return outer[0] <= pt[0] <= outer[2] and outer[1] <= pt[1] <= outer[3]


def describe_screen_ocr(srv, shot=None):
    """list_elements + screenshot + tesseract -> fused element list."""
    els = srv.tool("list_elements", {"interactive_only": False})["elements"]
    if shot is None:
        shot = tempfile.mktemp(suffix=".png", prefix="ocr-spike-")
        r = srv.tool("screenshot", {"path": shot, "quality": 95})
        if r.get("format") not in ("jpeg", "png"):
            raise RuntimeError(f"screenshot failed: {r}")
    for blk in ocr_blocks(shot):
        cx = (blk["bounds"][0] + blk["bounds"][2]) // 2
        cy = (blk["bounds"][1] + blk["bounds"][3]) // 2
        blk_area = max(1, (blk["bounds"][2] - blk["bounds"][0])
                        * (blk["bounds"][3] - blk["bounds"][1]))
        # candidate host: smallest-area textless node containing the
        # block centre whose area is ≤4x the block (a real label-sized
        # widget, not a screen-sized container).
        host = None
        for e in els:
            if e.get("text") or not _contains(e["bounds"], (cx, cy)):
                continue
            area = max(1, (e["bounds"][2] - e["bounds"][0])
                          * (e["bounds"][3] - e["bounds"][1]))
            if area > blk_area * 4:
                continue
            if host is None or area < host[0]:
                host = (area, e)
        if host is not None:
            host[1]["ocr_text"] = blk["text"]
            host[1]["ocr_conf"] = blk["conf"]
            continue
        # skip when a text-bearing node already claims this point
        claimed = any(_contains(e["bounds"], (cx, cy))
                      and e.get("text") for e in els)
        if not claimed:
            blk["i"] = len(els)
            blk["ocr_only"] = True
            blk["center"] = [cx, cy]
            els.append(blk)
    return {"count": len(els), "elements": els}


def main():
    srv = Serve()
    try:
        shot = sys.argv[1] if len(sys.argv) > 1 else None
        res = describe_screen_ocr(srv, shot)
    finally:
        srv.close()
    ocr = [e for e in res["elements"] if e.get("ocr_only")]
    filled = [e for e in res["elements"] if e.get("ocr_text")]
    print(json.dumps({"count": res["count"],
                      "ocr_only": len(ocr), "ocr_filled": len(filled)},
                     indent=1))
    for e in res["elements"]:
        tag = "OCR " if e.get("ocr_only") else ("+ocr" if e.get("ocr_text") else "tree")
        print(f"{tag} i={e.get('i', '?'):>3} {e.get('bounds')} "
              f"{e.get('text', '')[:50] or e.get('ocr_text', '')[:50]}")


if __name__ == "__main__":
    main()
