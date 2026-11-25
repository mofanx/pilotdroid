#!/usr/bin/env python3
"""som.py unit tests — deterministic numbering, attribute mapping,
degenerate trees, and the shared numbering space between
describe_screen and screenshot_annotated (PIL path)."""
import io
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..",
                                "compat"))
import som  # noqa: E402

XML = """<?xml version='1.0'?>
<hierarchy rotation="0">
<node index="0" text="" resource-id="" class="android.widget.FrameLayout"
      package="p" content-desc="" checkable="false" checked="false"
      clickable="false" enabled="true" focusable="false" focused="false"
      scrollable="false" long-clickable="false" password="false"
      selected="false" bounds="[0,0][1080,2400]">
  <node index="0" text="API Demos" resource-id="android:id/title"
        class="android.widget.TextView" package="p" content-desc=""
        checkable="false" checked="false" clickable="false" enabled="true"
        focusable="false" focused="false" scrollable="false"
        long-clickable="false" password="false" selected="false"
        bounds="[40,80][400,160]"/>
  <node index="1" text="" resource-id="" class="android.widget.ListView"
        package="p" content-desc="" checkable="false" checked="false"
        clickable="true" enabled="true" focusable="true" focused="false"
        scrollable="true" long-clickable="false" password="false"
        selected="false" bounds="[0,200][1080,2200]">
    <node index="0" text="Accessibility" resource-id=""
          class="android.widget.TextView" package="p" content-desc=""
          checkable="false" checked="false" clickable="true"
          enabled="true" focusable="false" focused="false"
          scrollable="false" long-clickable="false" password="false"
          selected="false" bounds="[20,300][1060,380]"/>
  </node>
</node>
</hierarchy>"""


class FakeDev:
    def __init__(self, xml=XML, shot=None):
        self._xml, self._shot = xml, shot

    def dump_hierarchy(self):
        return self._xml

    def screenshot(self, quality=80, scale=1.0):
        return self._shot


class TestEnumerate(unittest.TestCase):
    def test_doc_order_dense_ids_and_filter(self):
        els = som._enumerate(XML)
        # layout-only outer FrameLayout (no attrs/flags) is dropped;
        # ListView kept via clickable+scrollable flags alone.
        self.assertEqual([e["id"] for e in els], [1, 2, 3])
        self.assertEqual(els[0]["text"], "API Demos")
        self.assertEqual(els[1]["class"], "android.widget.ListView")
        self.assertEqual(els[2]["text"], "Accessibility")
        self.assertTrue(els[2]["clickable"])

    def test_center_and_bounds(self):
        els = som._enumerate(XML)
        self.assertEqual(els[0]["bounds"], (40, 80, 400, 160))
        self.assertEqual(els[0]["center"], [220, 120])

    def test_empty_tree_degrades(self):
        els = som._enumerate("<hierarchy/>")
        self.assertEqual(els, [])


class TestDescribeScreen(unittest.TestCase):
    def test_text_map_shape(self):
        out = som.describe_screen(FakeDev())
        self.assertEqual(len(out["elements"]), 3)
        lines = out["text"].splitlines()
        self.assertIn("3 elements", lines[0])
        self.assertIn("#1 [220,120] \"API Demos\"", lines[1])
        self.assertIn("C", lines[3])  # clickable flag rendered

    def test_empty_tree_text(self):
        out = som.describe_screen(FakeDev("<hierarchy/>"))
        self.assertEqual(out["elements"], [])
        self.assertIn("0 elements", out["text"])


class TestScreenshotAnnotated(unittest.TestCase):
    def _jpeg(self, w=400, h=600):
        from PIL import Image
        buf = io.BytesIO()
        Image.new("RGB", (w, h), (255, 255, 255)).save(buf, "JPEG")
        return buf.getvalue()

    def test_same_numbering_space_marks_image(self):
        try:
            from PIL import Image  # noqa: F401
        except ImportError:
            self.skipTest("no Pillow")
        dev = FakeDev(shot=self._jpeg())
        data = som.screenshot_annotated(dev)
        img = Image.open(io.BytesIO(data))
        self.assertEqual(img.size, (400, 600))
        # boxes/labels actually painted → some non-white pixels exist
        self.assertTrue(any(p != (255, 255, 255)
                            for p in img.getdata()))

    def test_inverted_and_zero_area_bounds_no_crash(self):
        """MIUI emits inverted bounds (e.g. [92,3236][217,3200] on
        bottom-clipped nodes — 27/97 real elements). Drawing must
        normalize or skip, never raise ValueError."""
        try:
            from PIL import Image  # noqa: F401
        except ImportError:
            self.skipTest("no Pillow")
        xml = """<hierarchy>
<node text="inv" bounds="[92,600][217,550]" clickable="true"
      class="android.widget.TextView" resource-id="" content-desc=""
      package="p" checkable="false" checked="false" enabled="true"
      focusable="false" focused="false" scrollable="false"
      long-clickable="false" password="false" selected="false" index="0"/>
<node text="zero" bounds="[10,10][10,200]" clickable="true"
      class="android.widget.TextView" resource-id="" content-desc=""
      package="p" checkable="false" checked="false" enabled="true"
      focusable="false" focused="false" scrollable="false"
      long-clickable="false" password="false" selected="false" index="1"/>
<node text="ok" bounds="[20,20][120,80]" clickable="true"
      class="android.widget.TextView" resource-id="" content-desc=""
      package="p" checkable="false" checked="false" enabled="true"
      focusable="false" focused="false" scrollable="false"
      long-clickable="false" password="false" selected="false" index="2"/>
</hierarchy>"""
        dev = FakeDev(xml, shot=self._jpeg())
        data = som.screenshot_annotated(dev)  # must not raise
        img = Image.open(io.BytesIO(data))
        self.assertTrue(any(p != (255, 255, 255) for p in img.getdata()))
        # numbering still dense over the same enumerate (incl. skipped)
        self.assertEqual([e["id"] for e in som._enumerate(xml)],
                         [1, 2, 3])

    def test_zero_height_and_both_axes_inverted_no_crash(self):
        """Zero-HEIGHT boxes (t==b — horizontal line, nothing to mark)
        and BOTH-axis inverted bounds must neither raise nor mis-draw."""
        try:
            from PIL import Image
        except ImportError:
            self.skipTest("no Pillow")
        xml = """<hierarchy>
<node text="zheight" bounds="[50,300][400,300]" clickable="true"
      class="android.widget.TextView" resource-id="" content-desc=""
      package="p" checkable="false" checked="false" enabled="true"
      focusable="false" focused="false" scrollable="false"
      long-clickable="false" password="false" selected="false" index="0"/>
<node text="bothinv" bounds="[300,400][100,200]" clickable="true"
      class="android.widget.TextView" resource-id="" content-desc=""
      package="p" checkable="false" checked="false" enabled="true"
      focusable="false" focused="false" scrollable="false"
      long-clickable="false" password="false" selected="false" index="1"/>
</hierarchy>"""
        dev = FakeDev(xml, shot=self._jpeg())
        data = som.screenshot_annotated(dev)  # must not raise
        img = Image.open(io.BytesIO(data))
        # bothinv normalizes to [100,200][300,400] → rectangle drawn
        self.assertTrue(any(p != (255, 255, 255) for p in img.getdata()))
        self.assertEqual([e["id"] for e in som._enumerate(xml)], [1, 2])

    def test_deterministic_ids_match_describe(self):
        try:
            from PIL import Image  # noqa: F401
        except ImportError:
            self.skipTest("no Pillow")
        dev = FakeDev(shot=self._jpeg())
        ids_a = [e["id"] for e in som.describe_screen(dev)["elements"]]
        # annotate calls _enumerate on the same dump → identical ids
        ids_b = [e["id"] for e in som._enumerate(XML)]
        self.assertEqual(ids_a, ids_b)

    def test_states_non_default_and_simple_class(self):
        xml = (
            '<hierarchy><node text="cb" resource-id="" '
            'class="android.widget.CheckBox" clickable="true" '
            'checked="true" focused="true" enabled="true" '
            'content-desc="" bounds="[0,0][10,10]"/>'
            '<node text="off" resource-id="" class="w" '
            'clickable="true" enabled="false" content-desc="" '
            'bounds="[0,20][10,30]"/></hierarchy>'
        )
        dev = FakeDev(xml=xml)
        els = som.describe_screen(dev)["elements"]
        self.assertEqual(els[0]["checked"], True)
        self.assertEqual(els[0]["focused"], True)
        self.assertNotIn("enabled", els[0])      # default → omitted
        self.assertNotIn("selected", els[0])
        self.assertEqual(els[1]["enabled"], False)
        # class 口径统一为简单名（与 list_elements 一致）
        self.assertEqual(els[0]["class"], "CheckBox")


if __name__ == "__main__":
    unittest.main()
