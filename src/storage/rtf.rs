//! Minimal hand-rolled RTF (Rich Text Format) reader.
//!
//! EasyWorship stores each slide's lyric/caption text as an RTF blob (`resource_text.rtf`).
//! This module extracts it as a sequence of formatted `TextRun`s (bold/italic/underline/
//! strikethrough/font/size/color) instead of throwing that formatting away — the plain-text
//! projection callers used before (`strip_rtf`) only ever kept the words.
//!
//! This is not a general-purpose RTF engine: no tables, embedded objects, lists, or footnotes —
//! just the character/paragraph formatting and font/color tables real EasyWorship exports
//! actually use for slide text, which is the only content this app ever needs to read back out
//! of an RTF blob.

use std::collections::HashMap;

use crate::core::models::TextRun;

/// Destination groups whose text must never reach the document (font/color tables are handled
/// specially below since we DO want their content, just not as visible text).
const RTF_SKIP_DESTINATIONS: &[&str] = &[
    "stylesheet", "info", "generator", "pntext", "pntxta", "pntxtb",
    "listtable", "listoverridetable", "rsidtbl", "themedata", "colorschememapping",
    "latentstyles", "xmlnstbl", "datastore", "fldinst", "nonshppict", "shp", "shpinst",
    "shptxt", "field", "bkmkstart", "bkmkend", "pict", "object", "objdata",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RtfRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub font_family: Option<String>,
    pub font_size_pt: Option<f64>,
    pub color: Option<String>,
}

#[derive(Debug, Default)]
pub struct RtfDocument {
    pub runs: Vec<RtfRun>,
    /// Paragraph alignment at the first text encountered ("left"/"center"/"right"/"justify"),
    /// used as a whole-block approximation since this app's `TextParagraphStyle` is one style
    /// per text element, not per RTF paragraph.
    pub alignment: Option<String>,
}

impl RtfDocument {
    /// Flattens all runs' text back into one plain string, collapsing the runs of blank
    /// paragraph lines RTF exporters commonly leave behind (matches the cleanup the old
    /// plain-text-only `strip_rtf` applied, so callers that just want text see the same result).
    pub fn plain_text(&self) -> String {
        let joined: String = self.runs.iter().map(|r| r.text.as_str()).collect();
        joined
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Converts the parsed runs into this app's `TextRun` model, applying `TextRun::default()`
    /// for any attribute RTF never specified (color/font/size), and dropping empty runs.
    pub fn to_text_runs(&self) -> Vec<TextRun> {
        self.runs
            .iter()
            .filter(|r| !r.text.is_empty())
            .map(|r| {
                let mut tr = TextRun { text: r.text.clone(), ..Default::default() };
                tr.bold = r.bold;
                tr.italic = r.italic;
                tr.underline = r.underline;
                tr.strike = r.strike;
                if let Some(ref c) = r.color {
                    tr.color = c.clone();
                }
                if let Some(ref f) = r.font_family {
                    tr.font_family = f.clone();
                }
                if let Some(sz) = r.font_size_pt {
                    tr.font_size_pt = sz;
                }
                tr
            })
            .collect()
    }
}

#[derive(Clone, PartialEq)]
struct CharState {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    font_index: Option<i32>,
    font_half_pt: Option<i32>,
    color_index: Option<i32>,
}

impl Default for CharState {
    fn default() -> Self {
        Self { bold: false, italic: false, underline: false, strike: false, font_index: None, font_half_pt: None, color_index: None }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Dest {
    Normal,
    Skip,
    FontTable,
    ColorTable,
}

fn resolve_run(text: String, st: &CharState, font_table: &HashMap<i32, String>, color_table: &[(u8, u8, u8)]) -> RtfRun {
    RtfRun {
        text,
        bold: st.bold,
        italic: st.italic,
        underline: st.underline,
        strike: st.strike,
        font_family: st.font_index.and_then(|idx| font_table.get(&idx).cloned()),
        font_size_pt: st.font_half_pt.map(|hp| hp as f64 / 2.0),
        color: st.color_index.and_then(|idx| {
            if idx <= 0 {
                None // \cf0 is the "auto" placeholder color, not an explicit choice
            } else {
                color_table.get(idx as usize).map(|&(r, g, b)| format!("#{:02x}{:02x}{:02x}", r, g, b))
            }
        }),
    }
}

/// Decodes a single cp1252 byte (the overwhelmingly common RTF codepage for Western text,
/// declared via `\ansicpg1252`) to its Unicode codepoint. Only the 0x80-0x9F range actually
/// differs from Latin-1/ISO-8859-1 (smart quotes, em/en dashes, ellipsis, etc.) — everything
/// else maps 1:1.
fn cp1252_to_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}', 0x82 => '\u{201A}', 0x83 => '\u{0192}', 0x84 => '\u{201E}',
        0x85 => '\u{2026}', 0x86 => '\u{2020}', 0x87 => '\u{2021}', 0x88 => '\u{02C6}',
        0x89 => '\u{2030}', 0x8A => '\u{0160}', 0x8B => '\u{2039}', 0x8C => '\u{0152}',
        0x8E => '\u{017D}', 0x91 => '\u{2018}', 0x92 => '\u{2019}', 0x93 => '\u{201C}',
        0x94 => '\u{201D}', 0x95 => '\u{2022}', 0x96 => '\u{2013}', 0x97 => '\u{2014}',
        0x98 => '\u{02DC}', 0x99 => '\u{2122}', 0x9A => '\u{0161}', 0x9B => '\u{203A}',
        0x9C => '\u{0153}', 0x9E => '\u{017E}', 0x9F => '\u{0178}',
        _ => byte as char, // 0x00-0x7F ASCII and 0xA0-0xFF match Latin-1 directly
    }
}

/// Parses an RTF blob into formatted runs plus a best-guess paragraph alignment.
pub fn parse_rtf(rtf: &str) -> RtfDocument {
    let bytes = rtf.as_bytes();
    let mut i = 0usize;

    let mut char_stack: Vec<CharState> = vec![CharState::default()];
    let mut dest_stack: Vec<Dest> = vec![Dest::Normal];
    let mut current_state = CharState::default();

    let mut font_table: HashMap<i32, String> = HashMap::new();
    let mut color_table: Vec<(u8, u8, u8)> = Vec::new();
    let mut ft_current_index: Option<i32> = None;
    let mut ft_current_name = String::new();
    let mut ct_pending: (Option<u8>, Option<u8>, Option<u8>) = (None, None, None);

    let mut runs: Vec<RtfRun> = Vec::new();
    let mut current_text = String::new();
    let mut alignment: Option<String> = None;

    macro_rules! flush {
        () => {
            if !current_text.is_empty() {
                runs.push(resolve_run(std::mem::take(&mut current_text), &current_state, &font_table, &color_table));
            }
        };
    }

    while i < bytes.len() {
        let dest = *dest_stack.last().unwrap();
        match bytes[i] {
            b'{' => {
                char_stack.push(current_state.clone());
                let mut new_dest = dest;
                let mut j = i + 1;
                if rtf[j..].starts_with("\\*") {
                    new_dest = Dest::Skip;
                    j += 2;
                }
                if bytes.get(j) == Some(&b'\\') {
                    let word_start = j + 1;
                    let mut w = word_start;
                    while w < bytes.len() && bytes[w].is_ascii_alphabetic() {
                        w += 1;
                    }
                    if w > word_start {
                        match &rtf[word_start..w] {
                            "fonttbl" => {
                                new_dest = Dest::FontTable;
                                ft_current_index = None;
                                ft_current_name.clear();
                            }
                            "colortbl" => {
                                new_dest = Dest::ColorTable;
                                ct_pending = (None, None, None);
                            }
                            word if new_dest != Dest::Skip && RTF_SKIP_DESTINATIONS.contains(&word) => {
                                new_dest = Dest::Skip;
                            }
                            _ => {}
                        }
                    }
                }
                dest_stack.push(new_dest);
                i += 1;
            }
            b'}' => {
                if dest == Dest::FontTable {
                    if let Some(idx) = ft_current_index.take() {
                        let name = ft_current_name.trim().to_string();
                        if !name.is_empty() {
                            font_table.entry(idx).or_insert(name);
                        }
                    }
                    ft_current_name.clear();
                }
                flush!();
                if let Some(prev) = char_stack.pop() {
                    current_state = prev;
                }
                dest_stack.pop();
                if dest_stack.is_empty() {
                    dest_stack.push(Dest::Normal);
                }
                i += 1;
            }
            b'\\' => {
                let next = bytes.get(i + 1).copied();
                match next {
                    Some(b'\\') | Some(b'{') | Some(b'}') => {
                        let ch = next.unwrap() as char;
                        push_char(dest, ch, &mut current_text, &mut ft_current_name);
                        i += 2;
                    }
                    Some(b'\'') => {
                        if i + 4 <= bytes.len() {
                            if let Ok(byte_val) = u8::from_str_radix(&rtf[i + 2..i + 4], 16) {
                                push_char(dest, cp1252_to_char(byte_val), &mut current_text, &mut ft_current_name);
                            }
                            i += 4;
                        } else {
                            i += 2;
                        }
                    }
                    Some(c) if c.is_ascii_alphabetic() => {
                        let word_start = i + 1;
                        let mut w = word_start;
                        while w < bytes.len() && bytes[w].is_ascii_alphabetic() {
                            w += 1;
                        }
                        let word = rtf[word_start..w].to_string();
                        let mut p = w;
                        let mut negative = false;
                        if p < bytes.len() && bytes[p] == b'-' {
                            negative = true;
                            p += 1;
                        }
                        let digits_start = p;
                        while p < bytes.len() && bytes[p].is_ascii_digit() {
                            p += 1;
                        }
                        let param: Option<i32> = if p > digits_start {
                            rtf[digits_start..p].parse::<i32>().ok().map(|v| if negative { -v } else { v })
                        } else {
                            None
                        };
                        let mut end = p;
                        if end < bytes.len() && bytes[end] == b' ' {
                            end += 1;
                        }

                        if word == "u" {
                            // \uN unicode escape: N is a signed decimal codepoint, followed by
                            // exactly one ASCII fallback character (default \ucN skip count = 1)
                            // for readers that don't understand \u.
                            if let Some(n) = param {
                                let code = if n < 0 { n + 65536 } else { n };
                                if let Some(ch) = char::from_u32(code as u32) {
                                    push_char(dest, ch, &mut current_text, &mut ft_current_name);
                                }
                            }
                            i = (end + 1).min(bytes.len());
                            continue;
                        }

                        match word.as_str() {
                            "par" | "line" => {
                                if dest == Dest::Normal {
                                    current_text.push('\n');
                                }
                            }
                            "tab" => {
                                if dest == Dest::Normal {
                                    current_text.push('\t');
                                }
                            }
                            "b" => {
                                flush!();
                                current_state.bold = param != Some(0);
                            }
                            "i" => {
                                flush!();
                                current_state.italic = param != Some(0);
                            }
                            "ul" => {
                                flush!();
                                current_state.underline = param != Some(0);
                            }
                            "ulnone" => {
                                flush!();
                                current_state.underline = false;
                            }
                            "strike" => {
                                flush!();
                                current_state.strike = param != Some(0);
                            }
                            "f" => {
                                if dest == Dest::FontTable {
                                    ft_current_index = param;
                                } else if dest == Dest::Normal {
                                    flush!();
                                    current_state.font_index = param;
                                }
                            }
                            "fs" => {
                                if dest == Dest::Normal {
                                    flush!();
                                    current_state.font_half_pt = param;
                                }
                            }
                            "cf" => {
                                if dest == Dest::Normal {
                                    flush!();
                                    current_state.color_index = param;
                                }
                            }
                            "red" => {
                                if dest == Dest::ColorTable {
                                    ct_pending.0 = param.map(|v| v.clamp(0, 255) as u8);
                                }
                            }
                            "green" => {
                                if dest == Dest::ColorTable {
                                    ct_pending.1 = param.map(|v| v.clamp(0, 255) as u8);
                                }
                            }
                            "blue" => {
                                if dest == Dest::ColorTable {
                                    ct_pending.2 = param.map(|v| v.clamp(0, 255) as u8);
                                }
                            }
                            "qc" => {
                                if alignment.is_none() {
                                    alignment = Some("center".to_string());
                                }
                            }
                            "ql" => {
                                if alignment.is_none() {
                                    alignment = Some("left".to_string());
                                }
                            }
                            "qr" => {
                                if alignment.is_none() {
                                    alignment = Some("right".to_string());
                                }
                            }
                            "qj" => {
                                if alignment.is_none() {
                                    alignment = Some("justify".to_string());
                                }
                            }
                            _ => {}
                        }
                        i = end;
                    }
                    Some(_) => {
                        // Unrecognized control symbol (\~, \_, \-, \: etc.) — no text effect.
                        i += 2;
                    }
                    None => {
                        i += 1;
                    }
                }
            }
            c => {
                match dest {
                    Dest::FontTable => {
                        if c == b';' {
                            if let Some(idx) = ft_current_index.take() {
                                let name = ft_current_name.trim().to_string();
                                if !name.is_empty() {
                                    font_table.entry(idx).or_insert(name);
                                }
                            }
                            ft_current_name.clear();
                        } else if c != b'\r' && c != b'\n' {
                            ft_current_name.push(c as char);
                        }
                    }
                    Dest::ColorTable => {
                        if c == b';' {
                            let (r, g, b) = ct_pending;
                            color_table.push((r.unwrap_or(0), g.unwrap_or(0), b.unwrap_or(0)));
                            ct_pending = (None, None, None);
                        }
                    }
                    Dest::Skip => {}
                    Dest::Normal => {
                        if c != b'\r' && c != b'\n' {
                            current_text.push(c as char);
                        }
                    }
                }
                i += 1;
            }
        }
    }

    flush!();

    RtfDocument { runs, alignment }
}

fn push_char(dest: Dest, ch: char, current_text: &mut String, ft_current_name: &mut String) {
    match dest {
        Dest::Normal => current_text.push(ch),
        Dest::FontTable => ft_current_name.push(ch),
        Dest::ColorTable | Dest::Skip => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_with_no_formatting() {
        let doc = parse_rtf(r"{\rtf1\ansi\deff0 Hello world\par Second line}");
        assert_eq!(doc.plain_text(), "Hello world\nSecond line");
    }

    #[test]
    fn bold_run_is_isolated_via_paired_on_off_control_words() {
        // The single space right after a control word is RTF's own delimiter syntax and
        // is consumed, not rendered — so "AAA"/"BBB"/"CCC" (no natural-language spaces)
        // keeps this trace unambiguous.
        let doc = parse_rtf(r"{\rtf1 AAA\b BBB\b0 CCC}");
        let runs = doc.to_text_runs();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "AAA");
        assert!(!runs[0].bold);
        assert_eq!(runs[1].text, "BBB");
        assert!(runs[1].bold);
        assert_eq!(runs[2].text, "CCC");
        assert!(!runs[2].bold);
    }

    #[test]
    fn italic_run_is_isolated_via_group_scoping() {
        // The other common real-world RTF style: formatting scoped to a `{...}` group
        // instead of paired on/off control words. Exercises char_stack push/pop restore.
        let doc = parse_rtf(r"{\rtf1 AAA{\i BBB}CCC}");
        let runs = doc.to_text_runs();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "AAA");
        assert!(!runs[0].italic);
        assert_eq!(runs[1].text, "BBB");
        assert!(runs[1].italic);
        assert_eq!(runs[2].text, "CCC");
        assert!(!runs[2].italic);
    }

    #[test]
    fn font_table_resolves_font_names() {
        let rtf = r"{\rtf1\ansi{\fonttbl{\f0\fswiss\fcharset0 Arial;}{\f1\froman Times New Roman;}}\f1\fs32 Hello}";
        let doc = parse_rtf(rtf);
        let runs = doc.to_text_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "Hello");
        assert_eq!(runs[0].font_family, "Times New Roman");
        assert_eq!(runs[0].font_size_pt, 16.0); // \fs32 is half-points -> 16pt
    }

    #[test]
    fn color_table_resolves_hex_colors() {
        let rtf = r"{\rtf1{\colortbl;\red255\green0\blue0;\red0\green128\blue255;}\cf1 Red\cf2  Blue}";
        let doc = parse_rtf(rtf);
        let runs = doc.to_text_runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "Red");
        assert_eq!(runs[0].color, "#ff0000");
        assert_eq!(runs[1].text, " Blue");
        assert_eq!(runs[1].color, "#0080ff");
    }

    #[test]
    fn hex_escape_decodes_smart_quotes_via_cp1252() {
        // \'92 is a right single quotation mark (’) in cp1252 — this is the exact
        // character class the old naive stripper mishandled.
        let doc = parse_rtf(r"{\rtf1\ansi\ansicpg1252 It\'92s working}");
        assert_eq!(doc.plain_text(), "It\u{2019}s working");
    }

    #[test]
    fn unicode_escape_with_ascii_fallback_is_skipped() {
        // Same right single quote as the cp1252 test above, via the \u escape path
        // instead — codepoint 8217, followed by the mandatory one-char ASCII fallback
        // ('?') that must be consumed, not emitted. Built by concatenating pieces
        // because a literal backslash-u immediately followed by 4 digits in one source
        // string can be silently rewritten by tooling before it reaches this file.
        let escape = format!("{}{}{}", '\\', 'u', 8217);
        let rtf = format!("{{\\rtf1 It{}?s working}}", escape);
        let doc = parse_rtf(&rtf);
        assert_eq!(doc.plain_text(), "It\u{2019}s working");
    }

    #[test]
    fn destination_groups_are_never_emitted_as_text() {
        let rtf = r"{\rtf1{\*\generator Some Generator App;}{\info{\author Jane Doe}}Visible text only}";
        let doc = parse_rtf(rtf);
        assert_eq!(doc.plain_text(), "Visible text only");
    }

    #[test]
    fn alignment_is_captured_from_first_paragraph() {
        let doc = parse_rtf(r"{\rtf1\pard\qc Centered text\par}");
        assert_eq!(doc.alignment.as_deref(), Some("center"));
    }

    #[test]
    fn blank_paragraph_lines_are_collapsed_in_plain_text() {
        let doc = parse_rtf(r"{\rtf1 First\par\par\par Second}");
        assert_eq!(doc.plain_text(), "First\nSecond");
    }
}
