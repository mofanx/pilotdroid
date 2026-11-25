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
    pub text: String,
    pub desc: String,
    pub rid: String,
    pub class: String,
    pub clickable: bool,
    pub bounds: Option<[i32; 4]>,
    pub center: Option<[i32; 2]>,
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
            text: a.get("text").cloned().unwrap_or_default(),
            desc: a.get("content-desc").cloned().unwrap_or_default(),
            rid: a.get("resource-id").cloned().unwrap_or_default(),
            class: a.get("class").cloned().unwrap_or_default(),
            clickable: a.get("clickable").is_some_and(|v| v == "true"),
            bounds,
            // div_euclid = Python // (floor) — matches on negative sums.
            center: bounds.map(|b| [(b[0] + b[2]).div_euclid(2), (b[1] + b[3]).div_euclid(2)]),
        });
    }
    out
}

/// `describe_screen` payload — `{elements, text}` identical to the
/// Python sibling's contract.
pub fn describe(xml: &str, max_label: Option<usize>) -> Value {
    let els = enumerate(xml);
    let ml = max_label.unwrap_or(MAX_LABEL);
    let mut lines = vec![format!(
        "# screen: {} elements (id [cx,cy] label rid class C=clickable)",
        els.len()
    )];
    for e in &els {
        let mut label = if e.text.is_empty() { &e.desc } else { &e.text }.clone();
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
        parts.push(e.class.rsplit('.').next().unwrap_or("").to_string());
        if e.clickable {
            parts.push("C".into());
        }
        lines.push(parts.join(" "));
    }
    json!({
        "elements": els.iter().map(|e| json!({
            "id": e.id, "text": e.text, "desc": e.desc, "rid": e.rid,
            "class": e.class, "clickable": e.clickable,
            "bounds": e.bounds, "center": e.center,
        })).collect::<Vec<_>>(),
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
