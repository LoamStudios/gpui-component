//! A deterministic text system for tests that lay text out, for
//! [`gpui::TestApp::with_text_system`].
//!
//! Every glyph of a font has one advance, so widths are exact multiples of
//! the font size, and soft wrapping is a greedy break at spaces and around
//! ideographs, the way a native backend breaks such text. GPUI's own test
//! text system shapes every document as a single row, so a test of anything
//! that wraps needs this one.
use gpui::{
    Bounds, CaretAffinity, CaretMovement, CaretPosition, Font, FontId, FontMetrics, FontWeight,
    GlyphId, InlineLayout, InlineLayoutRequest, InlineVisualLine, LineLayout, PaintFragment,
    PaintStyle, Pixels, PlatformTextLayout, PlatformTextSystem, Point, PositionedInlineBox,
    RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams, ResolvedDirection, ShapedGlyph,
    Size, TextAlign, TextBoundary, TextDirection, TextLayoutOptions, TextLayoutRequest,
    TextMovement, TextRenderingMode, TextRun, TextSelectionKind, VisualDirection, VisualLine,
    align_inline_boxes, point, px, size,
};
use std::{borrow::Cow, ops::Range, sync::Arc};

/// The proportional family: half an em per glyph, three quarters bold.
pub const BODY: &str = "Body";
/// The monospace family: an em per glyph, an em and a quarter bold.
pub const MONO: &str = "Mono";
const BODY_ID: FontId = FontId(1);
const MONO_ID: FontId = FontId(2);
const BOLD_BODY_ID: FontId = FontId(3);
const BOLD_MONO_ID: FontId = FontId(4);
const UNITS_PER_EM: f32 = 1000.;
const ASCENT: f32 = 800.;
const DESCENT: f32 = 200.;

/// A text system of two families, [`BODY`] and [`MONO`], whose glyphs all
/// have one advance per font.
pub struct WideMonoTextSystem;

impl WideMonoTextSystem {
    /// Advance of one glyph in `font_id`, in em units.
    fn advance_units(font_id: FontId) -> f32 {
        match font_id {
            MONO_ID => 1000.,
            BOLD_MONO_ID => 1250.,
            BODY_ID => 500.,
            BOLD_BODY_ID => 750.,
            _ => 500.,
        }
    }

    /// Width of `text` shaped entirely in `family` at `font_size`.
    pub fn width_of(text: &str, family: &str, font_size: Pixels) -> Pixels {
        let font_id = if family == MONO { MONO_ID } else { BODY_ID };
        font_size * (Self::advance_units(font_id) / UNITS_PER_EM) * text.chars().count() as f32
    }

    /// One cell per character of `text`, each in the font of its run.
    fn text_cells(&self, text: &str, font_size: Pixels, runs: &[TextRun]) -> Vec<Cell> {
        let mut cells = Vec::with_capacity(text.len());
        let mut run_start = 0;
        for (run_ix, run) in runs.iter().enumerate() {
            let run_end = (run_start + run.len).min(text.len());
            let font_id = self.font_id(&run.font).unwrap_or(BODY_ID);
            let advance = font_size * (Self::advance_units(font_id) / UNITS_PER_EM);
            for (ix, ch) in text[run_start..run_end].char_indices() {
                let start = run_start + ix;
                cells.push(Cell {
                    range: start..start + ch.len_utf8(),
                    width: if ch == '\n' { Pixels::ZERO } else { advance },
                    kind: CellKind::Char {
                        ch,
                        run: run_ix,
                        font_id,
                    },
                });
            }
            run_start = run_end;
        }
        cells
    }

    fn layout(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        options: TextLayoutOptions,
        cells: Vec<Cell>,
    ) -> (LineLayout, Arc<FakeTextLayout>) {
        let layout = Arc::new(FakeTextLayout::new(text, font_size, options, cells));

        let mut visual_lines = Vec::with_capacity(layout.rows.len());
        let mut paint_fragments: Vec<PaintFragment> = Vec::new();
        for row in &layout.rows {
            let fragment_start = paint_fragments.len();
            let mut x = Pixels::ZERO;
            let mut current: Option<(usize, FontId, Pixels, Vec<ShapedGlyph>)> = None;
            for cell in &layout.cells[row.cells.clone()] {
                match cell.kind {
                    CellKind::Char { ch, run, font_id } if ch != '\n' => {
                        if current.as_ref().is_some_and(|(r, ..)| *r != run) {
                            let (run, font_id, start, glyphs) = current.take().unwrap();
                            paint_fragments.push(fragment(
                                runs,
                                run,
                                font_id,
                                font_size,
                                start..x,
                                glyphs,
                            ));
                        }
                        let (_, _, _, glyphs) =
                            current.get_or_insert_with(|| (run, font_id, x, Vec::new()));
                        glyphs.push(ShapedGlyph {
                            id: GlyphId(ch as u32),
                            position: point(x, Pixels::ZERO),
                            is_emoji: false,
                        });
                    }
                    _ => {
                        if let Some((run, font_id, start, glyphs)) = current.take() {
                            paint_fragments.push(fragment(
                                runs,
                                run,
                                font_id,
                                font_size,
                                start..x,
                                glyphs,
                            ));
                        }
                    }
                }
                x += cell.width;
            }
            if let Some((run, font_id, start, glyphs)) = current.take() {
                paint_fragments.push(fragment(runs, run, font_id, font_size, start..x, glyphs));
            }
            visual_lines.push(VisualLine {
                text_range: row.text.clone(),
                paint_fragment_range: fragment_start..paint_fragments.len(),
                advance_width: row.advance,
                offset: row.offset,
                direction: ResolvedDirection::LeftToRight,
            });
        }

        let line_layout = LineLayout {
            font_size,
            width: layout.width(),
            ascent: font_size * (ASCENT / UNITS_PER_EM),
            // Native backends report the descent below the baseline as a
            // positive distance.
            descent: font_size * (DESCENT / UNITS_PER_EM),
            visual_lines: visual_lines.into_iter().collect(),
            paint_fragments,
            len: text.len(),
            platform_layout: layout.clone(),
        };
        (line_layout, layout)
    }
}

fn fragment(
    runs: &[TextRun],
    run: usize,
    font_id: FontId,
    font_size: Pixels,
    x_range: Range<Pixels>,
    glyphs: Vec<ShapedGlyph>,
) -> PaintFragment {
    PaintFragment {
        source_run: run,
        font_id,
        font_size,
        glyphs: glyphs.into(),
        x_range,
        style: PaintStyle::from(&runs[run]),
        underline_offset: Some(font_size * 0.1),
        strikethrough_offset: Some(-font_size * 0.3),
    }
}

#[derive(Clone, Debug)]
enum CellKind {
    Char {
        ch: char,
        run: usize,
        font_id: FontId,
    },
    /// An atomic inline element, the index of its request.
    Box(usize),
}

/// A character or an inline element, as placed in a row.
#[derive(Clone, Debug)]
struct Cell {
    range: Range<usize>,
    width: Pixels,
    kind: CellKind,
}

impl Cell {
    fn char(&self) -> Option<char> {
        match self.kind {
            CellKind::Char { ch, .. } => Some(ch),
            CellKind::Box(_) => None,
        }
    }

    fn is_newline(&self) -> bool {
        self.char() == Some('\n')
    }

    fn is_whitespace(&self) -> bool {
        self.char().is_some_and(char::is_whitespace)
    }

    /// Ideographs, and inline elements, may be broken before and after.
    fn breaks_around(&self) -> bool {
        match self.char() {
            Some(ch) => ch.len_utf8() >= 3 && !ch.is_whitespace(),
            None => true,
        }
    }
}

#[derive(Clone, Debug)]
struct Row {
    /// Cells on the row, in `FakeTextLayout::cells`.
    cells: Range<usize>,
    /// Source text on the row, including a hard break that ends it.
    text: Range<usize>,
    /// Width of the row, including the whitespace it ends with.
    advance: Pixels,
    /// Alignment offset of the row.
    offset: Pixels,
}

/// A character placed in the layout.
#[derive(Clone, Debug)]
struct Cluster {
    range: Range<usize>,
    row: usize,
    left: Pixels,
    right: Pixels,
    newline: bool,
}

/// Rows of cells broken greedily at `wrap_width`: after whitespace, and
/// around ideographs and inline elements. Whitespace hangs past the wrap
/// width; a word wider than it overflows its row.
fn break_rows(cells: &[Cell], wrap_width: Option<Pixels>) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut width = Pixels::ZERO;
    let mut ix = 0;
    while ix < cells.len() {
        let cell = &cells[ix];
        if cell.is_newline() {
            rows.push(start..ix + 1);
            start = ix + 1;
            width = Pixels::ZERO;
            ix += 1;
            continue;
        }
        if let Some(wrap_width) = wrap_width
            && !cell.is_whitespace()
            && ix > start
            && width + cell.width > wrap_width
        {
            let opportunity = (start + 1..=ix).rev().find(|&at| {
                let (before, after) = (&cells[at - 1], &cells[at]);
                !after.is_whitespace()
                    && (before.is_whitespace() || before.breaks_around() || after.breaks_around())
            });
            if let Some(at) = opportunity {
                rows.push(start..at);
                start = at;
                width = cells[at..ix].iter().map(|cell| cell.width).sum();
                continue;
            }
        }
        width += cell.width;
        ix += 1;
    }
    rows.push(start..cells.len());
    rows
}

/// A multi-row layout of a document of fixed-advance glyphs, left to
/// right, one cluster per character.
#[derive(Debug)]
struct FakeTextLayout {
    text: String,
    font_size: Pixels,
    cells: Vec<Cell>,
    rows: Vec<Row>,
    /// Characters in text order, which is row order.
    clusters: Vec<Cluster>,
    /// The characters on each row, in `clusters`.
    row_clusters: Vec<Range<usize>>,
    /// The left edge of each cell, in cell order.
    cell_left: Vec<Pixels>,
}

impl FakeTextLayout {
    fn new(text: &str, font_size: Pixels, options: TextLayoutOptions, cells: Vec<Cell>) -> Self {
        let mut rows = Vec::new();
        let mut clusters = Vec::new();
        let mut cell_left = vec![Pixels::ZERO; cells.len()];
        let mut text_start = 0;
        for (row_ix, cell_range) in break_rows(&cells, options.wrap_width)
            .into_iter()
            .enumerate()
        {
            let row_cells = &cells[cell_range.clone()];
            let advance: Pixels = row_cells.iter().map(|cell| cell.width).sum();
            let align_width = options.alignment_width.unwrap_or(advance);
            let offset = match options.text_align {
                TextAlign::Left | TextAlign::Start => Pixels::ZERO,
                TextAlign::Center => (align_width - advance) / 2.,
                TextAlign::Right | TextAlign::End => align_width - advance,
            };
            let text_end = row_cells
                .iter()
                .map(|cell| cell.range.end)
                .max()
                .unwrap_or(text_start);
            let mut x = offset;
            for (ix, cell) in cell_range.clone().zip(row_cells) {
                cell_left[ix] = x;
                if cell.char().is_some() {
                    clusters.push(Cluster {
                        range: cell.range.clone(),
                        row: row_ix,
                        left: x,
                        right: x + cell.width,
                        newline: cell.is_newline(),
                    });
                }
                x += cell.width;
            }
            rows.push(Row {
                cells: cell_range,
                text: text_start..text_end,
                advance,
                offset,
            });
            text_start = text_end;
        }

        let row_clusters = (0..rows.len())
            .map(|row| {
                clusters.partition_point(|c| c.row < row)
                    ..clusters.partition_point(|c| c.row <= row)
            })
            .collect();
        Self {
            text: text.to_owned(),
            font_size,
            cells,
            rows,
            clusters,
            row_clusters,
            cell_left,
        }
    }

    fn width(&self) -> Pixels {
        self.rows
            .iter()
            .map(|row| row.advance)
            .fold(Pixels::ZERO, Pixels::max)
    }

    fn on_row(&self, row: usize) -> &[Cluster] {
        &self.clusters[self.row_clusters[row].clone()]
    }

    fn row_end(&self, row: &Row) -> Pixels {
        row.offset + row.advance
    }

    /// The cluster starting at `index`, and the one ending there.
    fn neighbors(&self, index: usize) -> (Option<&Cluster>, Option<&Cluster>) {
        let next = self.clusters.partition_point(|c| c.range.start < index);
        let after = self.clusters.get(next).filter(|c| c.range.start == index);
        let before = next
            .checked_sub(1)
            .and_then(|ix| self.clusters.get(ix))
            .filter(|c| c.range.end == index && !c.newline);
        (after, before)
    }

    /// The row and x of a caret.
    fn caret_place(&self, caret: CaretPosition) -> (usize, Pixels) {
        let caret = self.normalized_caret(caret);
        match self.neighbors(caret.index) {
            (Some(after), Some(before)) => match caret.affinity {
                CaretAffinity::Downstream => (after.row, after.left),
                CaretAffinity::Upstream => (before.row, before.right),
            },
            (Some(after), None) => (after.row, after.left),
            (None, Some(before)) => (before.row, before.right),
            (None, None) => {
                // No characters: an empty document, an empty last row
                // after a hard break, or text around inline elements.
                let row = self
                    .rows
                    .iter()
                    .rposition(|row| row.text.start <= caret.index)
                    .unwrap_or(0);
                (row, self.rows[row].offset)
            }
        }
    }

    fn row_at(&self, y: Pixels, line_height: Pixels) -> usize {
        let row = (y / line_height).floor().max(0.) as usize;
        row.min(self.rows.len() - 1)
    }

    /// Caret stops on `row`, left to right.
    fn row_stops(&self, row: usize) -> Vec<(CaretPosition, Pixels)> {
        let mut stops = Vec::new();
        for cluster in self.on_row(row) {
            if stops.is_empty() || cluster.newline {
                stops.push((
                    CaretPosition::attached_to_next_cluster(cluster.range.start),
                    cluster.left,
                ));
            }
            if !cluster.newline {
                stops.push((
                    CaretPosition::attached_to_previous_cluster(cluster.range.end),
                    cluster.right,
                ));
            }
        }
        if stops.is_empty() {
            stops.push((
                CaretPosition::attached_to_next_cluster(self.rows[row].text.start),
                self.rows[row].offset,
            ));
        }
        stops
    }

    fn hard_line(&self, index: usize) -> Range<usize> {
        let index = index.min(self.text.len());
        let start = self.text[..index].rfind('\n').map_or(0, |ix| ix + 1);
        let end = self.text[index..]
            .find('\n')
            .map_or(self.text.len(), |ix| index + ix);
        start..end
    }

    fn word_at(&self, index: usize) -> Range<usize> {
        let index = index.min(self.text.len());
        let start = self.text[..index]
            .rfind(char::is_whitespace)
            .map_or(0, |offset| {
                offset + self.text[offset..].chars().next().unwrap().len_utf8()
            });
        let end = self.text[index..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |offset| index + offset);
        start..end
    }

    /// Visible end of `row`: before the hard break that ends it.
    fn row_text_end(&self, row: usize) -> usize {
        let row = &self.rows[row];
        if self.text[row.text.clone()].ends_with('\n') {
            row.text.end - 1
        } else {
            row.text.end
        }
    }
}

impl PlatformTextLayout for FakeTextLayout {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn size(&self) -> Size<Pixels> {
        size(self.width(), self.font_size * self.rows.len() as f32)
    }

    fn byte_index_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        let closest = self
            .caret_from_pixel_point(pixel_point, line_height)
            .unwrap_or_else(|caret| caret)
            .index;
        if pixel_point.y < Pixels::ZERO || pixel_point.y >= line_height * self.rows.len() as f32 {
            return Err(closest);
        }
        let row = self.row_at(pixel_point.y, line_height);
        self.on_row(row)
            .iter()
            .find(|c| pixel_point.x >= c.left && pixel_point.x < c.right)
            .map(|c| c.range.start)
            .ok_or(closest)
    }

    fn caret_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        let row_ix = self.row_at(pixel_point.y, line_height);
        let caret = self
            .row_stops(row_ix)
            .into_iter()
            .min_by(|(_, left), (_, right)| {
                (f32::from(*left) - f32::from(pixel_point.x))
                    .abs()
                    .total_cmp(&(f32::from(*right) - f32::from(pixel_point.x)).abs())
            })
            .map(|(caret, _)| caret)
            .unwrap_or_default();
        let row = &self.rows[row_ix];
        let inside = pixel_point.y >= line_height * row_ix as f32
            && pixel_point.y < line_height * (row_ix + 1) as f32
            && pixel_point.x >= row.offset
            && pixel_point.x < self.row_end(row);
        if inside { Ok(caret) } else { Err(caret) }
    }

    fn caret_bounds(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        if caret.index > self.text.len() {
            return None;
        }
        let (row, x) = self.caret_place(caret);
        Some(Bounds::new(
            point(x, line_height * row as f32),
            size(Pixels::ZERO, line_height),
        ))
    }

    fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
        let mut index = caret.index.min(self.text.len());
        while !self.text.is_char_boundary(index) {
            index -= 1;
        }
        CaretPosition {
            index,
            affinity: caret.affinity,
        }
    }

    fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let caret = self.normalized_caret(caret);
        let index = match direction {
            VisualDirection::Left => self.logical_cluster_before(caret)?.start,
            VisualDirection::Right => self.logical_cluster_after(caret)?.end,
        };
        Some(CaretPosition::attached_to_next_cluster(index))
    }

    fn selection_bounds(
        &self,
        byte_range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        if byte_range.is_empty() {
            return Vec::new();
        }
        let first = self
            .clusters
            .partition_point(|c| c.range.end <= byte_range.start);
        let hits = self.clusters[first..]
            .iter()
            .take_while(|c| c.range.start < byte_range.end);
        let mut bounds: Vec<(usize, Pixels, Pixels)> = Vec::new();
        for c in hits {
            match bounds.last_mut() {
                Some((row, left, right)) if *row == c.row => {
                    *left = (*left).min(c.left);
                    *right = (*right).max(c.right);
                }
                _ => bounds.push((c.row, c.left, c.right)),
            }
        }
        bounds
            .into_iter()
            .map(|(row, left, right)| {
                Bounds::from_corners(
                    point(left, line_height * row as f32),
                    point(right, line_height * (row + 1) as f32),
                )
            })
            .collect()
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        let ix = self
            .clusters
            .partition_point(|c| c.range.end <= caret.index);
        ix.checked_sub(1).map(|ix| self.clusters[ix].range.clone())
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        let ix = self
            .clusters
            .partition_point(|c| c.range.start < caret.index);
        self.clusters.get(ix).map(|c| c.range.clone())
    }

    fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        vertical_navigation_x: Option<Pixels>,
    ) -> CaretMovement {
        use TextBoundary::*;
        use TextDirection::*;

        let caret = self.normalized_caret(caret);
        let (row, x) = self.caret_place(caret);
        let downstream = CaretPosition::attached_to_next_cluster;
        let result = match (movement.direction, movement.boundary) {
            (Left, Cluster) => self
                .adjacent_visual_caret(caret, VisualDirection::Left)
                .unwrap_or(caret),
            (Right, Cluster) => self
                .adjacent_visual_caret(caret, VisualDirection::Right)
                .unwrap_or(caret),
            (Left, Word) => {
                let prefix = &self.text[..caret.index];
                let trimmed = prefix.trim_end_matches(char::is_whitespace);
                downstream(trimmed.rfind(char::is_whitespace).map_or(0, |index| {
                    index + trimmed[index..].chars().next().unwrap().len_utf8()
                }))
            }
            (Right, Word) => {
                let suffix = &self.text[caret.index..];
                let word_end = suffix.find(char::is_whitespace).unwrap_or(suffix.len());
                let rest = &suffix[word_end..];
                downstream(
                    caret.index
                        + word_end
                        + rest
                            .find(|character: char| !character.is_whitespace())
                            .unwrap_or(rest.len()),
                )
            }
            (Up | Down, VisualLine) => {
                let x = vertical_navigation_x.unwrap_or(x);
                let target = match movement.direction {
                    Up => row.checked_sub(1),
                    _ => Some(row + 1).filter(|row| *row < self.rows.len()),
                };
                match target {
                    Some(target) => self
                        .caret_from_pixel_point(point(x, px(target as f32 + 0.5)), px(1.))
                        .unwrap_or_else(|caret| caret),
                    None if movement.direction == Up => downstream(0),
                    None => CaretPosition::attached_to_previous_cluster(self.text.len()),
                }
            }
            (Start, VisualLine) => downstream(self.rows[row].text.start),
            (End, VisualLine) => {
                CaretPosition::attached_to_previous_cluster(self.row_text_end(row))
            }
            (Start, HardLine) => downstream(self.hard_line(caret.index).start),
            (End, HardLine) => {
                CaretPosition::attached_to_previous_cluster(self.hard_line(caret.index).end)
            }
            (Start, Document) => downstream(0),
            (End, Document) => CaretPosition::attached_to_previous_cluster(self.text.len()),
            _ => caret,
        };

        CaretMovement {
            result: self.normalized_caret(result),
            vertical_navigation_x: matches!(movement.direction, Up | Down)
                .then(|| vertical_navigation_x.unwrap_or(x)),
        }
    }

    fn selection_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        let index = self
            .caret_from_pixel_point(pixel_point, line_height)
            .unwrap_or_else(|caret| caret)
            .index;
        match kind {
            TextSelectionKind::Word => self.word_at(index),
            TextSelectionKind::VisualLine => {
                let row = self.row_at(pixel_point.y, line_height);
                self.rows[row].text.start..self.row_text_end(row)
            }
            TextSelectionKind::HardLine => self.hard_line(index),
        }
    }
}

impl PlatformTextSystem for WideMonoTextSystem {
    fn add_fonts(&self, _fonts: Vec<Cow<'static, [u8]>>) -> anyhow::Result<()> {
        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        vec![BODY.into(), MONO.into()]
    }

    fn font_id(&self, descriptor: &Font) -> anyhow::Result<FontId> {
        Ok(
            match (
                descriptor.family.as_ref() == MONO,
                descriptor.weight == FontWeight::BOLD,
            ) {
                (true, true) => BOLD_MONO_ID,
                (true, false) => MONO_ID,
                (false, true) => BOLD_BODY_ID,
                (false, false) => BODY_ID,
            },
        )
    }

    fn font_metrics(&self, _font_id: FontId) -> FontMetrics {
        FontMetrics {
            units_per_em: UNITS_PER_EM as u32,
            ascent: ASCENT,
            descent: -DESCENT,
            line_gap: 0.,
            underline_position: -100.,
            underline_thickness: 50.,
            cap_height: 700.,
            x_height: 500.,
            bounding_box: Bounds {
                origin: point(0., -200.),
                size: size(1000., 1000.),
            },
        }
    }

    fn typographic_bounds(
        &self,
        font_id: FontId,
        _glyph_id: GlyphId,
    ) -> anyhow::Result<Bounds<f32>> {
        Ok(Bounds {
            origin: point(0., 0.),
            size: size(Self::advance_units(font_id), 700.),
        })
    }

    fn advance(&self, font_id: FontId, _glyph_id: GlyphId) -> anyhow::Result<Size<f32>> {
        Ok(size(Self::advance_units(font_id), 0.))
    }

    fn glyph_for_char(&self, _font_id: FontId, ch: char) -> Option<GlyphId> {
        Some(GlyphId(ch as u32))
    }

    fn rasterize_glyph(&self, _params: &RenderGlyphParams) -> anyhow::Result<RasterizedGlyph> {
        Ok(RasterizedGlyph {
            bounds: Default::default(),
            size: Default::default(),
            format: RasterizedGlyphFormat::AlphaMask,
            pixels: Vec::new(),
        })
    }

    fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
        let cells = self.text_cells(request.text, request.font_size, request.runs);
        self.layout(
            request.text,
            request.font_size,
            request.runs,
            request.options,
            cells,
        )
        .0
    }

    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        // Each element is a cell of its width before the character at its
        // index, so the text flows around it.
        let mut cells = Vec::new();
        let mut boxes = request.boxes.iter().enumerate().peekable();
        for cell in self.text_cells(request.text, request.font_size, request.runs) {
            while let Some((ix, inline_box)) =
                boxes.next_if(|(_, inline_box)| inline_box.index <= cell.range.start)
            {
                cells.push(Cell {
                    range: inline_box.index..inline_box.index,
                    width: inline_box.size.width,
                    kind: CellKind::Box(ix),
                });
            }
            cells.push(cell);
        }
        for (ix, inline_box) in boxes {
            cells.push(Cell {
                range: inline_box.index..inline_box.index,
                width: inline_box.size.width,
                kind: CellKind::Box(ix),
            });
        }

        let (layout, fake) = self.layout(
            request.text,
            request.font_size,
            request.runs,
            request.options,
            cells,
        );
        let mut lines = fake
            .rows
            .iter()
            .map(|row| InlineVisualLine {
                origin: point(row.offset, Pixels::ZERO),
                size: size(row.advance, Pixels::ZERO),
                baseline: Pixels::ZERO,
            })
            .collect::<Vec<_>>();
        let mut positioned = Vec::new();
        for (row_ix, row) in fake.rows.iter().enumerate() {
            for ix in row.cells.clone() {
                if let CellKind::Box(request_ix) = fake.cells[ix].kind {
                    let inline_box = request.boxes[request_ix];
                    positioned.push(PositionedInlineBox {
                        id: inline_box.id,
                        line_index: row_ix,
                        bounds: Bounds::new(
                            point(fake.cell_left[ix], Pixels::ZERO),
                            inline_box.size,
                        ),
                    });
                }
            }
        }
        let mut size = size(layout.width, Pixels::ZERO);
        align_inline_boxes(
            &mut lines,
            &mut positioned,
            &mut size,
            request.boxes,
            &[],
            &[],
            request.text_metrics,
            request.line_height,
        );
        InlineLayout {
            layout: Arc::new(layout),
            lines,
            boxes: positioned,
            alignment_offset: Pixels::ZERO,
            size,
        }
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}
