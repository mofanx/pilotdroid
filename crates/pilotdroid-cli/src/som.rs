//! SoM-lite perception, host side (MCP surface of `compat/som.py` —
//! same numbering space: dense 1-based ids in document order over
//! "useful" nodes only).
//!
//! `describe()` serializes the a11y dump into a token-budget map —
//! id ↔ center ↔ label/rid/class/C — and `annotate()` paints the same
//! numbered boxes on the screenshot JPEG, so an agent can ground
//! "tap #12" through either surface. Both enumerate via [`enumerate`],
//! so id stability between them is by construction (same guarantee as
//! the Python sibling: `describe_screen`/`screenshot_annotated`).

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{ImageFormat, Rgb, RgbImage};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const ATTRS: [&str; 3] = ["text", "content-desc", "resource-id"];
const FLAGS: [&str; 5] = [
    "clickable",
    "scrollable",
    "checkable",
    "focusable",
    "long-clickable",
];
const PALETTE: [[u8; 3]; 6] = [
    [234, 67, 53],
    [52, 168, 83],
    [66, 133, 244],
    [251, 188, 4],
    [171, 71, 188],
    [0, 172, 193],
];
const MAX_LABEL: usize = 28;

/// One numbered element — same fields as `compat/som.py._enumerate`.
#[derive(Debug, Clone, PartialEq)]
pub struct El {
    pub id: usize,
    /// Interactive-space index — the `list_elements`/`tap_index`/
    /// `set_text_index`/`verify_after.match.i` numbering
    /// (clickable‖editable‖scrollable over ALL tree nodes, counted
    /// even when the node fails the `useful` filter). `None` for
    /// non-interactive and `ocr_only` elements — those are tappable
    /// only by `center` (no stale-snapshot guard).
    pub i: Option<usize>,
    pub text: String,
    pub desc: String,
    pub rid: String,
    pub class: String,
    pub clickable: bool,
    pub bounds: Option<[i32; 4]>,
    pub center: Option<[i32; 2]>,
    /// Widget state flags (dump attrs直通) — checkbox/switch/radio
    /// loops are provable through these (blind-drive B4 gap).
    pub checked: bool,
    pub selected: bool,
    pub enabled: bool,
    pub focused: bool,
    /// OCR fusion markers (`describe_screen ocr:true`): `ocr_text`
    /// fills a textless tree node that geometrically hosts a text
    /// block; `ocr_only` marks a synthetic element with no tree node
    /// (canvas/image text) — tap by `center`.
    pub ocr_text: Option<String>,
    pub ocr_only: bool,
    pub conf: Option<f32>,
}

/// XML entity decode for attribute values (`&amp;`/`&lt;`/`&gt;`/
/// `&quot;`/`&apos;`/numeric refs). UiAutomator escapes `>` too, so a
/// raw `>` inside a value is not a tag terminator concern.
fn decode_entities(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        for nc in it.by_ref() {
            if nc == ';' || name.len() > 8 {
                break;
            }
            name.push(nc);
        }
        let decoded = match name.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(d) => out.push(d),
            None => {
                out.push('&');
                out.push_str(&name);
                out.push(';');
            }
        }
    }
    out
}

/// Parse a `<node ...>` tag starting at `start` (index of `<`) —
/// returns (attrs, index of the closing `>`). Attribute values are
/// quoted; `>` inside a value is entity-escaped by the dumper so the
/// first unquoted `>`/self-close ends the tag.
fn parse_tag(s: &str, start: usize) -> Option<(BTreeMap<String, String>, usize)> {
    let b = s.as_bytes();
    let mut i = start + 5; // past "<node"
    let mut m = BTreeMap::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        if b[i] == b'>' || b[i] == b'/' {
            while i < b.len() && b[i] != b'>' {
                i += 1;
            }
            return (i < b.len()).then_some((m, i));
        }
        let ns = i;
        while i < b.len() && b[i] != b'=' && b[i] != b'>' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            // Malformed attr (no '=') — skip the tag rather than guess.
            while i < b.len() && b[i] != b'>' {
                i += 1;
            }
            return (i < b.len()).then_some((m, i));
        }
        let name = s[ns..i].to_string();
        i += 1;
        if i >= b.len() {
            return None;
        }
        let q = b[i];
        if q != b'"' && q != b'\'' {
            return None;
        }
        i += 1;
        let vs = i;
        while i < b.len() && b[i] != q {
            i += 1;
        }
        m.insert(name, decode_entities(&s[vs..i]));
        i += 1;
    }
}

fn parse_bounds(s: &str) -> Option<[i32; 4]> {
    let nums: Vec<i32> = s
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse().ok())
        .collect();
    (nums.len() == 4).then(|| [nums[0], nums[1], nums[2], nums[3]])
}

/// Filtered `<node>` list in document order with dense 1-based ids —
/// bit-identical semantics to `compat/som.py._enumerate`: a node counts
/// when it carries text/desc/rid or any interactive flag.
pub fn enumerate(xml: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut next_i = 0;
    let mut off = 0;
    while let Some(rel) = xml[off..].find("<node") {
        let st = off + rel;
        // `<node` must be followed by a boundary char (ws, `/`, `>`).
        match xml.as_bytes().get(st + 5) {
            Some(c) if !(c.is_ascii_whitespace() || *c == b'/' || *c == b'>') => {
                off = st + 5;
                continue;
            }
            None => break,
            _ => {}
        }
        let Some((a, end)) = parse_tag(xml, st) else {
            break;
        };
        off = end + 1;
        let cls = a.get("class").map(String::as_str).unwrap_or("");
        let clickable = a.get("clickable").is_some_and(|v| v == "true");
        let scrollable = a.get("scrollable").is_some_and(|v| v == "true");
        // Same interactive predicate as facade's list_elements filter
        // (editable = simple class EditText) — counted over ALL nodes,
        // before the `useful` early-continue: an interactive node with
        // no label/rid/flag still consumes an `i` (e.g. a bare
        // EditText), so describe `i` stays aligned with tap_index.
        let editable = cls.rsplit('.').next().unwrap_or(cls) == "EditText";
        let i = pilotdroid_agent::is_interactive(clickable, editable, scrollable).then(|| {
            let n = next_i;
            next_i += 1;
            n
        });
        let useful = ATTRS
            .iter()
            .any(|k| a.get(*k).is_some_and(|v| !v.is_empty()))
            || FLAGS.iter().any(|k| a.get(*k).is_some_and(|v| v == "true"));
        if !useful {
            continue;
        }
        let bounds = a.get("bounds").and_then(|v| parse_bounds(v));
        out.push(El {
            id: out.len() + 1,
            i,
            text: a.get("text").cloned().unwrap_or_default(),
            desc: a.get("content-desc").cloned().unwrap_or_default(),
            rid: a.get("resource-id").cloned().unwrap_or_default(),
            class: a.get("class").cloned().unwrap_or_default(),
            clickable,
            checked: a.get("checked").is_some_and(|v| v == "true"),
            selected: a.get("selected").is_some_and(|v| v == "true"),
            // absent `enabled` is the common fixture shorthand — true.
            enabled: a.get("enabled").map_or(true, |v| v != "false"),
            focused: a.get("focused").is_some_and(|v| v == "true"),
            bounds,
            // div_euclid = Python // (floor) — matches on negative sums.
            center: bounds.map(|b| [(b[0] + b[2]).div_euclid(2), (b[1] + b[3]).div_euclid(2)]),
            ocr_text: None,
            ocr_only: false,
            conf: None,
        });
    }
    out
}

/// `describe_screen` payload — `{elements, text}` identical to the
/// Python sibling's contract.
pub fn describe(xml: &str, max_label: Option<usize>) -> Value {
    emit(&enumerate(xml), max_label)
}

/// One merged OCR line block — host-side tesseract output already
/// merged word→line (same shape as `ocr_spike.py`).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrBlock {
    pub text: String,
    pub bounds: [i32; 4],
    pub conf: f32,
}

/// Token-budget guard for the OCR tier: cap appended elements at the
/// highest-confidence blocks (status-bar noise is low-conf in practice).
const MAX_OCR_BLOCKS: usize = 40;

/// `describe_screen ocr:true` — same payload with canvas/image text
/// fused in: a block whose centre lands inside a textless,
/// label-sized tree node back-fills `ocr_text`; an unclaimed block
/// becomes a synthetic `ocr_only` element (tap by `center`).
pub fn describe_ocr(xml: &str, max_label: Option<usize>, blocks: &[OcrBlock]) -> Value {
    let mut els = enumerate(xml);
    let mut blks: Vec<&OcrBlock> = blocks.iter().collect();
    blks.sort_by(|a, b| b.conf.total_cmp(&a.conf));
    blks.truncate(MAX_OCR_BLOCKS);
    for blk in blks {
        let cx = (blk.bounds[0] + blk.bounds[2]).div_euclid(2);
        let cy = (blk.bounds[1] + blk.bounds[3]).div_euclid(2);
        let blk_area = i64::from((blk.bounds[2] - blk.bounds[0]).max(1))
            * i64::from((blk.bounds[3] - blk.bounds[1]).max(1));
        // Candidate host: smallest-area textless node containing the
        // block centre whose area is ≤4x the block (a real label-sized
        // widget, not a screen-sized container).
        let mut host: Option<usize> = None;
        for (i, e) in els.iter().enumerate() {
            let Some(b) = e.bounds else { continue };
            if !e.text.is_empty() || !contains(e.bounds, cx, cy) {
                continue;
            }
            let area = i64::from((b[2] - b[0]).max(1)) * i64::from((b[3] - b[1]).max(1));
            if area > blk_area * 4 {
                continue;
            }
            if host.map_or(true, |h| {
                let hb = els[h].bounds.unwrap();
                i64::from((hb[2] - hb[0]).max(1)) * i64::from((hb[3] - hb[1]).max(1)) > area
            }) {
                host = Some(i);
            }
        }
        if let Some(i) = host {
            els[i].ocr_text = Some(blk.text.clone());
            els[i].conf = Some(blk.conf);
            continue;
        }
        // skip when a text-bearing node already claims this point
        let claimed = els
            .iter()
            .any(|e| !e.text.is_empty() && contains(e.bounds, cx, cy));
        if !claimed {
            let bounds = blk.bounds;
            els.push(El {
                id: els.len() + 1,
                i: None, // ocr_only: no a11y node → tap by `center` only
                text: blk.text.clone(),
                desc: String::new(),
                rid: String::new(),
                class: String::new(),
                clickable: false,
                checked: false,
                selected: false,
                enabled: true,
                focused: false,
                bounds: Some(bounds),
                center: Some([cx, cy]),
                ocr_text: None,
                ocr_only: true,
                conf: Some(blk.conf),
            });
        }
    }
    emit(&els, max_label)
}

fn contains(bounds: Option<[i32; 4]>, x: i32, y: i32) -> bool {
    let Some(b) = bounds else { return false };
    b[0] <= x && x <= b[2] && b[1] <= y && y <= b[3]
}

fn emit(els: &[El], max_label: Option<usize>) -> Value {
    let ml = max_label.unwrap_or(MAX_LABEL);
    let mut lines = vec![format!(
        "# screen: {} elements (id [cx,cy] label rid class C=clickable \
         K=checked S=selected F=focused D=disabled OCR=ocr-only \
         i<n>=list_elements/tap_index index)",
        els.len()
    )];
    for e in els {
        let mut label = if e.text.is_empty() { &e.desc } else { &e.text }.clone();
        if label.is_empty() {
            if let Some(t) = &e.ocr_text {
                label = t.clone();
            }
        }
        if label.chars().count() > ml {
            label = label.chars().take(ml - 1).collect::<String>() + "…";
        }
        let mut parts = vec![
            format!("#{}", e.id),
            e.center
                .map(|c| format!("[{},{}]", c[0], c[1]))
                .unwrap_or_else(|| "[-,-]".into()),
        ];
        if !label.is_empty() {
            parts.push(format!("\"{label}\""));
        }
        if !e.rid.is_empty() {
            parts.push(e.rid.clone());
        }
        let cls = e.class.rsplit('.').next().unwrap_or("");
        if !cls.is_empty() {
            parts.push(cls.to_string());
        }
        if e.clickable {
            parts.push("C".into());
        }
        if e.checked {
            parts.push("K".into());
        }
        if e.selected {
            parts.push("S".into());
        }
        if e.focused {
            parts.push("F".into());
        }
        if !e.enabled {
            parts.push("D".into());
        }
        if e.ocr_only {
            parts.push("OCR".into());
        }
        if let Some(i) = e.i {
            parts.push(format!("i{i}"));
        }
        lines.push(parts.join(" "));
    }
    // Same field convention as list_elements: simple class name;
    // checked/selected/focused emitted only when true; `enabled` only
    // when false (disabled). Keeps the token budget honest.
    json!({
        "elements": els.iter().map(|e| {
            let mut j = json!({
                "id": e.id, "text": e.text, "desc": e.desc, "rid": e.rid,
                "class": e.class.rsplit('.').next().unwrap_or(&e.class),
                "clickable": e.clickable,
                "bounds": e.bounds, "center": e.center,
            });
            let o = j.as_object_mut().unwrap();
            if let Some(i) = e.i { o.insert("i".into(), json!(i)); }
            if e.checked { o.insert("checked".into(), json!(true)); }
            if e.selected { o.insert("selected".into(), json!(true)); }
            if e.focused { o.insert("focused".into(), json!(true)); }
            if !e.enabled { o.insert("enabled".into(), json!(false)); }
            if let Some(t) = &e.ocr_text {
                o.insert("ocr_text".into(), json!(t));
            }
            if e.ocr_only { o.insert("ocr".into(), json!(true)); }
            if let Some(c) = e.conf {
                o.insert("ocr_conf".into(), json!(c));
            }
            j
        }).collect::<Vec<_>>(),
        "text": lines.join("\n"),
    })
}

// ---- annotation (image crate; agent.jar stays untouched) ---------------

fn put(img: &mut RgbImage, x: i32, y: i32, c: [u8; 3]) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, Rgb(c));
    }
}

/// Inclusive rectangle outline, `w` px thick drawn inward — matches
/// PIL `rectangle([l,t,r,b], width=3)` edge semantics.
fn rect_outline(img: &mut RgbImage, l: i32, t: i32, r: i32, b: i32, c: [u8; 3], w: i32) {
    for i in 0..w {
        for x in l..=r {
            put(img, x, t + i, c);
            put(img, x, b - i, c);
        }
        for y in t..=b {
            put(img, l + i, y, c);
            put(img, r - i, y, c);
        }
    }
}

fn fill_rect(img: &mut RgbImage, l: i32, t: i32, r: i32, b: i32, c: [u8; 3]) {
    for y in t..=b {
        for x in l..=r {
            put(img, x, y, c);
        }
    }
}

/// 5x7 digit glyphs, LSB = leftmost pixel.
const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];

fn draw_digits(img: &mut RgbImage, x: i32, y: i32, s: &str, c: [u8; 3]) {
    for (i, ch) in s.chars().enumerate() {
        let Some(d) = ch.to_digit(10) else { continue };
        let g = &DIGITS[d as usize];
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << col) != 0 {
                    put(img, x + i as i32 * 6 + col, y + row as i32, c);
                }
            }
        }
    }
}

/// `screenshot_annotated` payload bytes — the same numbered ids drawn
/// on the JPEG (scale must be 1.0: bounds are display pixels). Bounds
/// are normalized and zero-area boxes skipped — real dumps carry
/// inverted/degenerate bounds (MIUI: 27/97 measured; PIL raised
/// ValueError on y1<y0, this impl just clips).
pub fn annotate(jpeg: &[u8], xml: &str, quality: u8) -> Result<Vec<u8>> {
    let img = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .context("decode screenshot jpeg")?
        .to_rgb8();
    let els = enumerate(xml);
    let mut img = img;
    let mut skipped = 0u32;
    for e in &els {
        let Some(b) = e.bounds else {
            skipped += 1;
            continue;
        };
        let (l, r) = (b[0].min(b[2]), b[0].max(b[2]));
        let (t, bot) = (b[1].min(b[3]), b[1].max(b[3]));
        if r <= l || bot <= t {
            skipped += 1;
            continue;
        }
        let color = PALETTE[(e.id - 1) % PALETTE.len()];
        rect_outline(&mut img, l, t, r, bot, color, 3);
        let tag = e.id.to_string();
        // PIL tag geometry: bg rect [l, t-14 → max(14,t)], text (l+4, t-12).
        let tw = tag.len() as i32 * 6 - 1;
        fill_rect(&mut img, l, (t - 14).max(0), l + tw + 8, t.max(14), color);
        draw_digits(&mut img, l + 4, (t - 12).max(0) + 1, &tag, [255, 255, 255]);
    }
    let mut buf = Vec::with_capacity(jpeg.len());
    JpegEncoder::new_with_quality(&mut buf, quality)
        .encode_image(&img)
        .context("encode annotated jpeg")?;
    if skipped > 0 {
        eprintln!("pilotdroid-som: {skipped} elements skipped (no/degenerate bounds)");
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = concat!(
        r#"<?xml version='1.0'?><hierarchy>"#,
        r#"<node index="0" text="设置" resource-id="com.x:id/t" "#,
        r#"class="android.widget.TextView" clickable="true" content-desc="" "#,
        r#"bounds="[10,20][110,60]"/>"#,
        // layout-only node → filtered out (no attrs, no flags)
        r#"<node index="1" text="" resource-id="" class="android.widget.FrameLayout" "#,
        r#"clickable="false" content-desc="" bounds="[0,0][1440,3200]">"#,
        r#"<node index="0" text="" resource-id="com.x:id/e" "#,
        r#"class="android.widget.EditText" clickable="true" content-desc="输入框" "#,
        r#"bounds="[10,80][300,140]"/></node>"#,
        // inverted y bounds + scrollable flag (degenerate kept for describe)
        r#"<node index="2" text="" resource-id="" class="android.widget.ListView" "#,
        r#"clickable="false" scrollable="true" content-desc="" bounds="[92,236][217,200]"/>"#,
        r#"</hierarchy>"#
    );

    #[test]
    fn enumerate_filters_and_numbers_densely() {
        let els = enumerate(XML);
        assert_eq!(els.len(), 3);
        assert_eq!(els[0].id, 1);
        assert_eq!(els[0].text, "设置");
        assert_eq!(els[0].center, Some([60, 40]));
        // FrameLayout dropped → ids stay dense (EditText = #2, ListView = #3)
        assert_eq!(els[1].rid, "com.x:id/e");
        assert!(!els[2].clickable);
        assert_eq!(els[2].bounds, Some([92, 236, 217, 200]));
        // i space: all three are interactive → dense 0,1,2 aligned
        // with tap_index ordering (document order).
        assert_eq!(
            els.iter().map(|e| e.i).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2)]
        );
    }

    /// Interactive-space `i` must track the FULL node list, not the
    /// `useful` subset: text-only/label nodes carry no `i`, and a
    /// non-useful interactive node (bare EditText: editable but no
    /// label/rid/flag) still consumes an index — that's exactly the
    /// describe-id ≠ list-i divergence the field exists to bridge.
    #[test]
    fn i_maps_to_list_elements_space() {
        let xml = concat!(
            r#"<hierarchy>"#,
            // #0: text-only label — useful, NOT interactive → id 1, i None
            r#"<node text="标题" class="w.TextView" bounds="[0,0][10,10]"/>"#,
            // #1: bare EditText — editable → interactive i=0, but NOT
            //     useful (no label/rid/flag) → dropped from output
            r#"<node class="android.widget.EditText" bounds="[0,10][10,20]"/>"#,
            // #2: clickable — id 2, i=1 (bare EditText consumed i=0)
            r#"<node text="OK" class="w.Button" clickable="true" bounds="[0,20][10,30]"/>"#,
            r#"</hierarchy>"#
        );
        let els = enumerate(xml);
        assert_eq!(els.len(), 2);
        assert_eq!((els[0].id, els[0].i), (1, None));
        // list_elements interactive space: [EditText(i0), Button(i1)]
        // — the Button's describe id is 2 but its tap_index index is 1.
        assert_eq!((els[1].id, els[1].i), (2, Some(1)));
        let v = describe(xml, None);
        assert!(v["text"].as_str().unwrap().contains("OK\" Button C i1"));
        assert_eq!(v["elements"][1]["i"], json!(1));
        assert!(v["elements"][0].get("i").is_none());
    }

    #[test]
    fn enumerate_decodes_entities() {
        let xml = r#"<hierarchy><node text="a &amp; b &lt;c&gt;" resource-id="" class="w" clickable="true" content-desc="" bounds="[0,0][1,1]"/></hierarchy>"#;
        let els = enumerate(xml);
        assert_eq!(els[0].text, "a & b <c>");
    }

    #[test]
    fn describe_text_format_matches_python() {
        let v = describe(XML, None);
        let text = v["text"].as_str().unwrap();
        assert!(text.starts_with("# screen: 3 elements"));
        assert!(text.contains("#1 [60,40] \"设置\" com.x:id/t TextView C"));
        // desc fallback label, simple class name, no rid → no rid field
        assert!(text.contains("#2 [155,110] \"输入框\" com.x:id/e EditText C"));
        assert!(v["elements"].as_array().unwrap().len() == 3);
    }

    #[test]
    fn label_truncation() {
        let xml = r#"<hierarchy><node text="abcdefghijklmnopqrstuvwxyzabcdef" resource-id="" class="w" clickable="true" content-desc="" bounds="[0,0][1,1]"/></hierarchy>"#;
        let v = describe(xml, Some(10));
        assert!(v["text"].as_str().unwrap().contains("abcdefghi…"));
    }

    #[test]
    fn states_emit_non_default_only() {
        let xml = concat!(
            r#"<hierarchy>"#,
            r#"<node text="cb" resource-id="" class="android.widget.CheckBox" "#,
            r#"clickable="true" checked="true" focused="true" enabled="true" "#,
            r#"content-desc="" bounds="[0,0][10,10]"/>"#,
            r#"<node text="off" resource-id="" class="w" clickable="true" "#,
            r#"enabled="false" content-desc="" bounds="[0,20][10,30]"/>"#,
            r#"</hierarchy>"#
        );
        let v = describe(xml, None);
        let els = v["elements"].as_array().unwrap();
        assert_eq!(els[0]["checked"], true);
        assert_eq!(els[0]["focused"], true);
        // enabled-by-default must NOT appear (token budget)
        assert!(els[0].get("enabled").is_none());
        assert!(els[0].get("selected").is_none());
        assert_eq!(els[1]["enabled"], false);
        assert!(els[1].get("checked").is_none());
        // compact-line flags
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("#1 [5,5] \"cb\" CheckBox C K F"), "{text}");
        assert!(text.contains("#2 [5,25] \"off\" w C D"), "{text}");
    }

    #[test]
    fn describe_ocr_fuses_blocks() {
        // canvas-text node: a textless label-sized View hosts a block
        // centre → ocr_text back-fill; a free-floating block becomes an
        // ocr_only element; a block inside a text node is deduped.
        let xml = concat!(
            r#"<hierarchy>"#,
            r#"<node text="" resource-id="" class="android.view.View" "#,
            r#"clickable="true" content-desc="" bounds="[50,50][200,100]"/>"#,
            r#"<node text="title" resource-id="" class="w" "#,
            r#"clickable="false" content-desc="" bounds="[0,0][400,40]"/>"#,
            r#"</hierarchy>"#
        );
        let blocks = vec![
            OcrBlock {
                text: "canvas label".into(),
                bounds: [60, 55, 190, 90],
                conf: 88.0,
            },
            OcrBlock {
                text: "float text".into(),
                bounds: [500, 500, 700, 560],
                conf: 90.0,
            },
            OcrBlock {
                text: "inside titled".into(),
                bounds: [10, 5, 60, 35],
                conf: 92.0,
            },
        ];
        let v = describe_ocr(xml, None, &blocks);
        let els = v["elements"].as_array().unwrap();
        assert_eq!(els.len(), 3, "host back-fill + 1 ocr_only + dedup");
        assert_eq!(els[0]["ocr_text"], "canvas label");
        let ocr = &els[2];
        assert_eq!(ocr["ocr"], true);
        assert_eq!(ocr["text"], "float text");
        assert_eq!(ocr["center"], json!([600, 530]));
        assert!(v["text"].as_str().unwrap().contains("OCR"));
    }

    #[test]
    fn annotate_handles_inverted_and_zero_bounds() {
        // 2x2 white jpeg → annotate must not panic/raise on the
        // inverted + zero-area nodes (PIL used to ValueError here).
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, 90)
            .encode_image(&RgbImage::from_pixel(64, 64, Rgb([200, 200, 200])))
            .unwrap();
        let out = annotate(&buf, XML, 80).unwrap();
        assert!(out.len() > 100);
        let back = image::load_from_memory_with_format(&out, ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8();
        // #1 box at [10,20][110,60] red — clipped to the 64px image.
        let px = back.get_pixel(12, 22);
        assert!(px[0] > 150 && px[1] < 120, "box edge not red: {px:?}");
    }
}
