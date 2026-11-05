//! Rough text measurement for laying out imported slides without a font engine: Helvetica's
//! advance widths (which Arial and Liberation Sans share), scaled for other common families, and
//! greedy word wrapping on top. Good to a line or so for ordinary paragraphs.

/// Helvetica's advance widths for ASCII 32..=126, in thousandths of an em.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // '0'..'?'
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // '@'..'O'
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 'P'..'_'
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // '`'..'o'
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // 'p'..'~'
];

/// How wide a family sets text against Helvetica (its first name in a CSS stack, any case);
/// `None` for a fixed-pitch family (every character 0.6 em).
fn family_factor(family: &str) -> Option<f64> {
    let first = family.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '\'' || c == '"').to_ascii_lowercase();
    let has = |names: &[&str]| names.iter().any(|n| first.contains(n));
    // Fixed pitch by name: whole words only ("Fira Code", "DejaVu Sans Mono", but not "Encode Sans"
    // or "Monotype Corsiva").
    let words: Vec<&str> = first.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier" | "consolas" | "menlo" | "inconsolata" | "typewriter")) {
        return None;
    }
    Some(if has(&["calibri", "carlito", "candara", "corbel"]) {
        0.9
    } else if has(&["narrow", "condensed", "arial nar"]) {
        0.82
    } else if has(&["verdana", "dejavu sans", "bitstream vera"]) {
        1.12
    } else if has(&["times", "liberation serif", "cambria", "caladea", "garamond"]) || first == "serif" {
        0.9
    } else {
        // Arial, Liberation Sans, Helvetica and their kin; Tahoma, Segoe UI, Open Sans, Lato and
        // Georgia set about as wide.
        1.0
    })
}

/// About how wide `text` is, in ems of its font.
pub fn text_width_em(text: &str, family: &str, bold: bool) -> f64 {
    let Some(factor) = family_factor(family) else { return text.chars().count() as f64 * 0.6 };
    let base: f64 = text.chars().map(|c| match c as u32 {
        32..=126 => HELVETICA[(c as u32 - 32) as usize] as f64 / 1000.0,
        // CJK, Hangul and full-width forms set about an em per character.
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF => 1.0,
        _ => 0.556,
    }).sum();
    base * factor * if bold { 1.07 } else { 1.0 }
}

/// A piece of a paragraph set in one font: its text, family, boldness and size (any unit, the
/// same as `width` in `wrapped_lines`).
pub struct Span<'a> {
    pub text: &'a str,
    pub family: &'a str,
    pub bold: bool,
    pub size: f64,
}

/// How many lines a paragraph wraps to in `width`, breaking at spaces (and inside a word too
/// long for a line on its own).
pub fn wrapped_lines(spans: &[Span], width: f64) -> usize {
    let mut lines = 1;
    let mut x = 0.0;
    let mut pending_space = 0.0;
    for span in spans {
        let space = text_width_em(" ", span.family, span.bold) * span.size;
        for (i, word) in span.text.split(' ').enumerate() {
            if i > 0 {
                pending_space += space;
            }
            if word.is_empty() {
                continue;
            }
            let w = text_width_em(word, span.family, span.bold) * span.size;
            if x > 0.0 && x + pending_space + w > width {
                lines += 1;
                x = 0.0;
            } else {
                x += pending_space;
            }
            pending_space = 0.0;
            x += w;
            // A word wider than the line breaks wherever it must.
            while x > width && width > 0.0 {
                lines += 1;
                x -= width;
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_widths_and_family_scaling() {
        assert!((text_width_em("Hill", "Arial", false) - (0.722 + 0.222 * 3.0)).abs() < 1e-9);
        assert!(text_width_em("Hill", "'Carlito', 'Calibri', sans-serif", false) < text_width_em("Hill", "Arial", false));
        assert!((text_width_em("abc", "Courier New", true) - 1.8).abs() < 1e-9);
        assert!((text_width_em("abc", "Fira Code", false) - 1.8).abs() < 1e-9);
        assert!(text_width_em("iii", "Encode Sans", false) < 1.0 && text_width_em("iii", "Monotype Corsiva", false) < 1.0);
    }

    #[test]
    fn wraps_at_spaces_and_inside_overlong_words() {
        let span = |text| Span { text, family: "Arial", bold: false, size: 10.0 };
        // "aaaa" is 22.24 wide at size 10, a space 2.78.
        assert_eq!(wrapped_lines(&[span("aaaa aaaa")], 50.0), 1);
        assert_eq!(wrapped_lines(&[span("aaaa aaaa")], 40.0), 2);
        assert_eq!(wrapped_lines(&[span("aaaaaaaaaaaaaaaa")], 40.0), 3);
        assert_eq!(wrapped_lines(&[span("")], 40.0), 1);
    }
}
