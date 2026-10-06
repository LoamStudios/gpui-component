//! Greedy line wrapping at word boundaries, measured one character at a time,
//! for text whose wrap points the kit decides itself: the editor's soft wraps
//! and inline flows that mix text with fixed-width elements.
//!
//! Ported from gpui's `LineWrapper`, which gpui removed when its text system
//! moved to Parley.

use std::{collections::HashMap, iter, sync::Arc};

use gpui::{Font, Pixels, TextSystem, px};

/// Wraps lines of text in one font and size to a width.
pub(crate) struct LineWrapper {
    text_system: Arc<TextSystem>,
    font: Font,
    font_size: Pixels,
    cached_ascii_char_widths: [Option<Pixels>; 128],
    cached_other_char_widths: HashMap<char, Pixels>,
}

impl LineWrapper {
    /// The largest indent a wrapped line carries over.
    pub(crate) const MAX_INDENT: u32 = 256;

    pub(crate) fn new(font: Font, font_size: Pixels, text_system: Arc<TextSystem>) -> Self {
        Self {
            text_system,
            font,
            font_size,
            cached_ascii_char_widths: [None; 128],
            cached_other_char_widths: HashMap::default(),
        }
    }

    /// The boundaries at which `fragments` wrap to fit `wrap_width`, in order.
    pub(crate) fn wrap_line<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
    ) -> impl Iterator<Item = Boundary> + 'a {
        let mut width = px(0.);
        let mut first_non_whitespace_ix = None;
        let mut indent = None;
        let mut last_candidate_ix = 0;
        let mut last_candidate_width = px(0.);
        let mut last_wrap_ix = 0;
        let mut prev_c = '\0';
        let mut index = 0;
        let mut candidates = fragments
            .iter()
            .flat_map(move |fragment| fragment.wrap_boundary_candidates())
            .peekable();
        iter::from_fn(move || {
            for candidate in candidates.by_ref() {
                let ix = index;
                index += candidate.len_utf8();
                let mut new_prev_c = prev_c;
                let item_width = match candidate {
                    WrapBoundaryCandidate::Char { character: c } => {
                        if c == '\n' {
                            continue;
                        }

                        if Self::is_word_char(c) {
                            if prev_c == ' ' && c != ' ' && first_non_whitespace_ix.is_some() {
                                last_candidate_ix = ix;
                                last_candidate_width = width;
                            }
                        } else {
                            // CJK may not be space separated, e.g.: `Hello world你好世界`
                            if c != ' ' && first_non_whitespace_ix.is_some() {
                                last_candidate_ix = ix;
                                last_candidate_width = width;
                            }
                        }

                        if c != ' ' && first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = Some(ix);
                        }

                        new_prev_c = c;

                        self.width_for_char(c)
                    }
                    WrapBoundaryCandidate::Element {
                        width: element_width,
                        ..
                    } => {
                        if prev_c == ' ' && first_non_whitespace_ix.is_some() {
                            last_candidate_ix = ix;
                            last_candidate_width = width;
                        }

                        if first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = Some(ix);
                        }

                        element_width
                    }
                };

                width += item_width;
                if width > wrap_width && ix > last_wrap_ix {
                    if let (None, Some(first_non_whitespace_ix)) = (indent, first_non_whitespace_ix)
                    {
                        indent = Some(
                            Self::MAX_INDENT.min((first_non_whitespace_ix - last_wrap_ix) as u32),
                        );
                    }

                    if last_candidate_ix > 0 {
                        last_wrap_ix = last_candidate_ix;
                        width -= last_candidate_width;
                        last_candidate_ix = 0;
                    } else {
                        last_wrap_ix = ix;
                        width = item_width;
                    }

                    if let Some(indent) = indent {
                        width += self.width_for_char(' ') * indent as f32;
                    }

                    return Some(Boundary {
                        ix: last_wrap_ix,
                        next_indent: indent.unwrap_or(0),
                    });
                }

                prev_c = new_prev_c;
            }

            None
        })
    }

    /// Whether `c` is part of a word that should not be split by a wrap.
    pub(crate) fn is_word_char(c: char) -> bool {
        // ASCII alphanumeric characters, for English, numbers: `Hello123`, etc.
        c.is_ascii_alphanumeric() ||
        // Latin script in Unicode for French, German, Spanish, etc.
        // Latin-1 Supplement
        // https://en.wikipedia.org/wiki/Latin-1_Supplement
        matches!(c, '\u{00C0}'..='\u{00FF}') ||
        // Latin Extended-A
        // https://en.wikipedia.org/wiki/Latin_Extended-A
        matches!(c, '\u{0100}'..='\u{017F}') ||
        // Latin Extended-B
        // https://en.wikipedia.org/wiki/Latin_Extended-B
        matches!(c, '\u{0180}'..='\u{024F}') ||
        // Cyrillic for Russian, Ukrainian, etc.
        // https://en.wikipedia.org/wiki/Cyrillic_script_in_Unicode
        matches!(c, '\u{0400}'..='\u{04FF}') ||

        // Vietnamese (https://vietunicode.sourceforge.net/charset/)
        matches!(c, '\u{1E00}'..='\u{1EFF}') || // Latin Extended Additional
        matches!(c, '\u{0300}'..='\u{036F}') || // Combining Diacritical Marks

        // Bengali (https://en.wikipedia.org/wiki/Bengali_(Unicode_block))
        matches!(c, '\u{0980}'..='\u{09FF}') ||

        // Some other known special characters that should be treated as word characters,
        // e.g. `a-b`, `var_name`, `I'm`/`won’t`, '@mention`, `#hashtag`, `100%`, `3.1415`,
        // `2^3`, `a~b`, `a=1`, `Self::new`, etc. Trailing punctuation like `,`, `.`, `:`, `;`
        // is included so it stays attached to the preceding word when wrapping.
        matches!(c, '-' | '_' | '.' | '\'' | '’' | '‘' | '$' | '%' | '@' | '#' | '^' | '~' | ',' | '=' | ':' | ';') ||
        // Closing punctuation never starts a line (UAX #14 LB13: no break
        // before `!`, `)`, `]`, `}`, closing quotes or an ellipsis) — `plz!`,
        // `see)`, `quoted”` wrap as one word instead of orphaning the mark on
        // the next line. `/` and `?` stay break opportunities so long paths
        // and URLs (`a/b`, `foo?b=2`) can wrap.
        matches!(c, '!' | ')' | ']' | '}' | '"' | '”' | '»' | '…') ||
        // `⋯` character is special used in Zed, to keep this at the end of the line.
        matches!(c, '⋯') ||

        // Non-breaking glue characters
        matches!(c, '\u{202F}' | '\u{00A0}' | '\u{2011}')
    }

    fn width_for_char(&mut self, c: char) -> Pixels {
        if (c as u32) < 128 {
            if let Some(cached_width) = self.cached_ascii_char_widths[c as usize] {
                return cached_width;
            }
            let width = self.measure_char(c);
            self.cached_ascii_char_widths[c as usize] = Some(width);
            width
        } else if let Some(cached_width) = self.cached_other_char_widths.get(&c) {
            *cached_width
        } else {
            let width = self.measure_char(c);
            self.cached_other_char_widths.insert(c, width);
            width
        }
    }

    fn measure_char(&self, c: char) -> Pixels {
        let mut buffer = [0; 4];
        self.text_system
            .line_width(c.encode_utf8(&mut buffer), &self.font, self.font_size)
    }
}

/// A part of a line to wrap: text, or an element of a fixed width.
#[derive(Clone, Copy)]
pub(crate) enum LineFragment<'a> {
    Text {
        text: &'a str,
    },
    Element {
        width: Pixels,
        /// The bytes of the line the element stands for.
        len_utf8: usize,
    },
}

impl<'a> LineFragment<'a> {
    pub(crate) fn text(text: &'a str) -> Self {
        LineFragment::Text { text }
    }

    pub(crate) fn element(width: Pixels, len_utf8: usize) -> Self {
        LineFragment::Element { width, len_utf8 }
    }

    fn wrap_boundary_candidates(&self) -> impl Iterator<Item = WrapBoundaryCandidate> + use<'a> {
        let fragment = *self;
        let text = match fragment {
            LineFragment::Text { text } => text,
            LineFragment::Element { .. } => "\0",
        };
        text.chars().map(move |character| match fragment {
            LineFragment::Element { width, len_utf8 } => {
                WrapBoundaryCandidate::Element { width, len_utf8 }
            }
            LineFragment::Text { .. } => WrapBoundaryCandidate::Char { character },
        })
    }
}

enum WrapBoundaryCandidate {
    Char { character: char },
    Element { width: Pixels, len_utf8: usize },
}

impl WrapBoundaryCandidate {
    fn len_utf8(&self) -> usize {
        match self {
            WrapBoundaryCandidate::Char { character } => character.len_utf8(),
            WrapBoundaryCandidate::Element { len_utf8, .. } => *len_utf8,
        }
    }
}

/// Where a line wraps.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Boundary {
    /// The byte index the next line starts at.
    pub(crate) ix: usize,
    /// How many characters the next line is indented by.
    pub(crate) next_indent: u32,
}
