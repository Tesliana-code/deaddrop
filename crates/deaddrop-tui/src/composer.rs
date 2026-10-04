//! The compose box as rows on screen. Presentation only: the draft is never
//! changed to make it fit.
//!
//! The box starts one row tall, grows with the wrapped draft up to
//! [`MAX_ROWS`], then scrolls so the cursor stays in view.
//!
//! The cursor in a draft counts grapheme clusters — what a reader sees as
//! one character — so editing never lands inside an emoji, a flag or a
//! letter with a combining accent.

use unicode_segmentation::UnicodeSegmentation;

use crate::ui::width;

/// Tallest the compose box gets before it scrolls.
pub const MAX_ROWS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Visual rows. A row may end in spaces that hang past the edge.
    pub rows: Vec<String>,
    /// Where the cursor sits: (row, column).
    pub cursor: (usize, usize),
}

impl View {
    /// Rows the box takes on screen, at most `max`.
    pub fn height(&self, max: usize) -> usize {
        self.rows.len().clamp(1, max.max(1))
    }

    /// First row shown when `height` rows fit: as many as needed to keep the
    /// cursor row on screen.
    pub fn offset(&self, height: usize) -> usize {
        (self.cursor.0 + 1).saturating_sub(height.max(1))
    }
}

/// Graphemes in `text`.
pub fn graphemes(text: &str) -> usize {
    text.graphemes(true).count()
}

/// The byte where grapheme `g` starts (the end, past the last).
fn boundary(text: &str, g: usize) -> usize {
    text.grapheme_indices(true)
        .nth(g)
        .map_or(text.len(), |(i, _)| i)
}

/// Chars before a cursor `cursor` graphemes in, for [`layout_at`].
pub fn cursor_chars(text: &str, cursor: usize) -> usize {
    text[..boundary(text, cursor)].chars().count()
}

/// Insert `s` at the cursor and step past it. A combining mark joins the
/// character before it, so the cursor is recounted, not just advanced.
pub fn insert(text: &mut String, cursor: &mut usize, s: &str) {
    let at = boundary(text, *cursor);
    text.insert_str(at, s);
    *cursor = graphemes(&text[..at + s.len()]);
}

/// Remove the character before the cursor.
pub fn backspace(text: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let (from, to) = (boundary(text, *cursor - 1), boundary(text, *cursor));
    text.replace_range(from..to, "");
    *cursor -= 1;
}

/// Remove the character after the cursor.
pub fn delete(text: &mut String, cursor: usize) {
    let (from, to) = (boundary(text, cursor), boundary(text, cursor + 1));
    text.replace_range(from..to, "");
}

pub fn left(cursor: &mut usize) {
    *cursor = cursor.saturating_sub(1);
}

pub fn right(text: &str, cursor: &mut usize) {
    *cursor = (*cursor + 1).min(graphemes(text));
}

/// The cursor (in graphemes) for a click on visual row `row`, column `col`
/// of the laid-out draft. A click on a character puts the cursor before
/// it; past the end of a row, at the end of that row's text; below the
/// last row, at the end of the draft. Never inside a grapheme.
pub fn cursor_at(draft: &str, cols: usize, row: usize, col: usize) -> usize {
    let chars: Vec<char> = draft.chars().collect();
    let total = chars.len();
    let rows = layout_at(draft, cols, total).rows;
    // Where each cursor position, 0..=total chars in, is drawn.
    let spots: Vec<(usize, usize)> = (0..=total)
        .map(|k| place(draft, &mut rows.clone(), cols.max(1), k))
        .collect();
    let on_row: Vec<usize> = (0..=total).filter(|&k| spots[k].0 == row).collect();
    let k = match on_row.last() {
        None if row > spots[total].0 => total,
        None => 0,
        Some(&last) => {
            // The character under the click starts at the last position
            // not right of the click.
            let mut k = on_row
                .iter()
                .copied()
                .rfind(|&k| spots[k].1 <= col)
                .unwrap_or(on_row[0]);
            // Past the end of a wrapped row: its end, which is drawn at
            // the start of the next row.
            if k == last && k < total && chars[k] != '\n' {
                let w = width(chars[k].encode_utf8(&mut [0; 4]));
                if col >= spots[k].1 + w.max(1) {
                    k += 1;
                }
            }
            k
        }
    };
    graphemes_before(draft, k)
}

/// Graphemes that start before char `k`: a position inside a grapheme
/// rounds forward past it.
fn graphemes_before(text: &str, k: usize) -> usize {
    let mut seen = 0;
    let mut count = 0;
    for g in text.graphemes(true) {
        if seen >= k {
            break;
        }
        count += 1;
        seen += g.chars().count();
    }
    count
}

/// [`layout_at`] with the cursor at the end of the draft.
pub fn layout(draft: &str, cols: usize) -> View {
    layout_at(draft, cols, draft.chars().count())
}

/// Soft-wrap `draft` to `cols` columns, with the cursor `cursor` chars in.
/// Explicit newlines always break; otherwise words move to the next row
/// whole, and words wider than a row are split between characters, never
/// inside one.
pub fn layout_at(draft: &str, cols: usize, cursor: usize) -> View {
    let cols = cols.max(1);
    let mut rows = Vec::new();
    for paragraph in draft.split('\n') {
        let mut row = String::new();
        let mut used = 0;
        for token in tokens(paragraph) {
            let word = token.trim_end_matches(' ');
            let word_cols = width(word);
            if used + word_cols > cols && used > 0 && word_cols <= cols {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            if used + word_cols <= cols {
                row.push_str(token);
                used += width(token);
                continue;
            }
            for c in word.chars() {
                let c_cols = width(c.encode_utf8(&mut [0; 4]));
                if used + c_cols > cols && used > 0 {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                row.push(c);
                used += c_cols;
            }
            let spaces = &token[word.len()..];
            row.push_str(spaces);
            used += spaces.len();
        }
        rows.push(row);
    }
    let cursor = place(draft, &mut rows, cols, cursor);
    View { rows, cursor }
}

/// Where the cursor, `cursor` chars into `draft`, sits among `rows`. Rows
/// hold the draft's chars in order with the newlines left out, so walking
/// the chars walks the rows. A cursor at the end of a full row starts the
/// next one, adding it if the row is the last.
fn place(draft: &str, rows: &mut Vec<String>, cols: usize, cursor: usize) -> (usize, usize) {
    let (mut row, mut taken, mut col) = (0, 0, 0);
    let mut chars = draft.chars().peekable();
    for _ in 0..cursor {
        let Some(c) = chars.next() else { break };
        if c == '\n' {
            (row, taken, col) = (row + 1, 0, 0);
            continue;
        }
        // This row is used up: the char wrapped to the next one.
        if taken == rows[row].chars().count() && row + 1 < rows.len() {
            (row, taken, col) = (row + 1, 0, 0);
        }
        taken += 1;
        col += width(c.encode_utf8(&mut [0; 4]));
    }
    let row_done = taken == rows[row].chars().count();
    match chars.peek() {
        // More of this paragraph follows on the next row.
        Some(c) if *c != '\n' && row_done && row + 1 < rows.len() => (row + 1, 0),
        // At the end of a full row with nothing after: a fresh row.
        None if col >= cols && row + 1 == rows.len() => {
            rows.push(String::new());
            (row + 1, 0)
        }
        _ => (row, col),
    }
}

/// A word with the spaces after it; leading spaces are a token of their own.
fn tokens(paragraph: &str) -> impl Iterator<Item = &str> {
    let mut rest = paragraph;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let word_end = rest.find(' ').unwrap_or(rest.len());
        let end = rest[word_end..]
            .find(|c| c != ' ')
            .map_or(rest.len(), |i| word_end + i);
        let (token, tail) = rest.split_at(end);
        rest = tail;
        Some(token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(view: &View) -> String {
        view.rows.concat()
    }

    #[test]
    fn short_draft_is_one_row() {
        let view = layout("hello danil", 40);
        assert_eq!(view.rows, ["hello danil"]);
        assert_eq!(view.cursor, (0, 11));
        assert_eq!(view.height(MAX_ROWS), 1);
        assert_eq!(layout("", 40).height(MAX_ROWS), 1);
        assert_eq!(layout("", 40).cursor, (0, 0));
    }

    #[test]
    fn long_draft_wraps_on_words_and_keeps_every_char() {
        let draft = "later abilities for o-dzi in wire-u presence capabilities";
        let view = layout(draft, 20);
        assert!(view.rows.len() > 1);
        assert_eq!(joined(&view), draft, "wrapping never changes the draft");
        assert!(view.rows.iter().all(|r| width(r.trim_end()) <= 20));
        assert_eq!(view.rows[0], "later abilities for ");
    }

    #[test]
    fn box_grows_with_the_draft_then_stops() {
        let mut draft = String::new();
        let mut last = 1;
        for _ in 0..40 {
            draft.push_str("word ");
            let height = layout(&draft, 20).height(MAX_ROWS);
            assert!(height >= last, "never shrinks while typing");
            last = height;
        }
        assert_eq!(last, MAX_ROWS);
    }

    #[test]
    fn past_the_max_the_view_follows_the_cursor() {
        let draft = "line\n".repeat(10) + "end";
        let view = layout(&draft, 20);
        assert_eq!(view.rows.len(), 11);
        let height = view.height(MAX_ROWS);
        let offset = view.offset(height);
        assert_eq!(height, MAX_ROWS);
        assert_eq!(offset, 11 - MAX_ROWS);
        assert!((offset..offset + height).contains(&view.cursor.0));
        assert_eq!(view.cursor, (10, 3));
        assert_eq!(layout("short", 20).offset(MAX_ROWS), 0);
    }

    #[test]
    fn deleting_shrinks_the_box_again() {
        let mut draft = "a\n".repeat(8);
        assert_eq!(layout(&draft, 20).height(MAX_ROWS), MAX_ROWS);
        while draft.matches('\n').count() > 1 {
            draft.pop();
        }
        assert_eq!(layout(&draft, 20).height(MAX_ROWS), 2);
        draft.clear();
        assert_eq!(layout(&draft, 20).height(MAX_ROWS), 1);
    }

    #[test]
    fn explicit_newlines_break_rows_and_survive() {
        let draft = "later abilities for o-dzi:\npresence\ncapabilities\n\nmemory";
        let view = layout(draft, 40);
        assert_eq!(
            view.rows,
            [
                "later abilities for o-dzi:",
                "presence",
                "capabilities",
                "",
                "memory"
            ]
        );
        assert_eq!(view.cursor, (4, 6));
        assert_eq!(layout("hi\n", 40).cursor, (1, 0), "cursor after a newline");
    }

    #[test]
    fn wide_glyphs_wrap_whole_and_count_two_columns() {
        let view = layout("🌼🌼🌼🌼🌼", 4);
        assert_eq!(view.rows, ["🌼🌼", "🌼🌼", "🌼"]);
        assert_eq!(view.cursor, (2, 2));
        assert_eq!(layout("ab🌼", 3).rows, ["ab", "🌼"], "never half a glyph");
        let view = layout("привет мир", 6);
        assert_eq!(view.rows, ["привет ", "мир"]);
        assert_eq!(view.cursor, (1, 3));
    }

    #[test]
    fn a_full_row_puts_the_cursor_on_the_next() {
        let view = layout("abcd", 4);
        assert_eq!(view.cursor, (1, 0));
        assert_eq!(view.height(MAX_ROWS), 2);
        let view = layout("abc ", 4);
        assert_eq!(view.cursor, (1, 0), "a trailing space can fill a row");
    }

    #[test]
    fn tiny_widths_do_not_panic_or_loop() {
        for cols in 0..4 {
            for draft in ["", " ", "🌼", "  🌼 a\n\nb  ", "x".repeat(30).as_str()] {
                let view = layout(draft, cols);
                assert_eq!(joined(&view), draft.replace('\n', ""));
                assert!(view.cursor.0 < view.rows.len());
            }
        }
    }

    fn edit(start: &str, cursor: usize) -> (String, usize) {
        (start.to_owned(), cursor)
    }

    #[test]
    fn insertion_and_deletion_happen_at_the_cursor() {
        let (mut t, mut c) = edit("helo", 3);
        insert(&mut t, &mut c, "l");
        assert_eq!((t.as_str(), c), ("hello", 4));
        backspace(&mut t, &mut c);
        assert_eq!((t.as_str(), c), ("helo", 3));
        delete(&mut t, c);
        assert_eq!((t.as_str(), c), ("hel", 3));
        delete(&mut t, c);
        assert_eq!(t, "hel", "delete at the end does nothing");
        c = 0;
        backspace(&mut t, &mut c);
        assert_eq!(
            (t.as_str(), c),
            ("hel", 0),
            "backspace at the start does nothing"
        );
        insert(&mut t, &mut c, "\n");
        assert_eq!((t.as_str(), c), ("\nhel", 1));
    }

    #[test]
    fn the_cursor_steps_over_whole_characters() {
        // š (one char), 🐙, a ZWJ family, a flag, e + combining acute.
        let text = "š🐙👩‍💻🇷🇸e\u{301}";
        assert_eq!(graphemes(text), 5);
        let mut c = 0;
        for _ in 0..10 {
            right(text, &mut c);
        }
        assert_eq!(c, 5, "clamped at the end");
        let (mut t, mut c) = edit(text, 5);
        backspace(&mut t, &mut c);
        assert_eq!(t, "š🐙👩‍💻🇷🇸", "the accented e goes whole");
        backspace(&mut t, &mut c);
        backspace(&mut t, &mut c);
        assert_eq!(t, "š🐙", "flag and ZWJ sequence go whole");
        left(&mut c);
        delete(&mut t, c);
        assert_eq!((t.as_str(), c), ("š", 1));
        for _ in 0..5 {
            left(&mut c);
        }
        assert_eq!(c, 0, "clamped at the start");
        // A combining mark typed after a letter joins it.
        let (mut t, mut c) = edit("e", 1);
        insert(&mut t, &mut c, "\u{301}");
        assert_eq!((graphemes(&t), c), (1, 1));
    }

    #[test]
    fn the_cursor_is_placed_through_soft_wraps_and_newlines() {
        let draft = "abcd efgh\nij";
        // At width 5: "abcd " | "efgh" | "ij".
        let at = |chars| layout_at(draft, 5, chars).cursor;
        assert_eq!(layout_at(draft, 5, 0).rows, ["abcd ", "efgh", "ij"]);
        assert_eq!(at(0), (0, 0));
        assert_eq!(at(2), (0, 2));
        assert_eq!(at(4), (0, 4));
        assert_eq!(at(5), (1, 0), "after the hanging space: the next row");
        assert_eq!(at(7), (1, 2));
        assert_eq!(at(9), (1, 4), "end of a paragraph, before its newline");
        assert_eq!(at(10), (2, 0), "after the newline");
        assert_eq!(at(12), (2, 2));
        // Wide characters count two columns.
        assert_eq!(layout_at("🐙🐙x", 10, 2).cursor, (0, 4));
        assert_eq!(cursor_chars("š🐙👩‍💻", 2), 2);
        assert_eq!(cursor_chars("š🐙👩‍💻", 3), 5);
    }

    #[test]
    fn clicking_a_drawn_position_gives_that_position_back() {
        for draft in [
            "volim hobotncie 🐙",
            "alpha bravo charlie delta echo foxtrot golf hotel",
            "prva\n\ndruga linija ovde\n",
            "šđ🐙 e\u{301} 👩‍💻 kraj",
            "averyveryverylongwordthatmustsplit and more",
        ] {
            let total = graphemes(draft);
            for g in 0..=total {
                let view = layout_at(draft, 12, cursor_chars(draft, g));
                let (row, col) = view.cursor;
                assert_eq!(cursor_at(draft, 12, row, col), g, "{draft:?} at {g}");
            }
        }
    }

    #[test]
    fn clicks_land_on_characters_line_ends_and_past_the_end() {
        let draft = "volim hobotncie 🐙\ndrugi red";
        // On "c" of "hobotncie": before the c.
        let c = "volim hobotn".chars().count();
        assert_eq!(cursor_at(draft, 40, 0, c), graphemes("volim hobotn"));
        // Far right of row 0: end of that line, before the newline.
        assert_eq!(cursor_at(draft, 40, 0, 39), graphemes("volim hobotncie 🐙"));
        // On the emoji's second cell: before the emoji, never inside it.
        let x = crate::ui::width("volim hobotncie ") + 1;
        assert_eq!(cursor_at(draft, 40, 0, x), graphemes("volim hobotncie "));
        // Below everything: the end.
        assert_eq!(cursor_at(draft, 40, 9, 0), graphemes(draft));
        // Past the end of a soft-wrapped row: the row's end.
        let wrapped = "abcdefghij klm";
        assert_eq!(layout_at(wrapped, 5, 0).rows[0], "abcde");
        assert_eq!(cursor_at(wrapped, 5, 0, 9), 5);
    }
}
