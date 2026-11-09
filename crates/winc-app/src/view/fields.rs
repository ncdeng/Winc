//! 输入框的绘制：会换行的多行输入区、单行的输入条，以及插入点。

use super::*;

/// 多行输入框（[`TextField::multiline`]）按宽度折好的行：[`wrap_area_field`] 排一次，框的高度
/// 按它的行数定，再交给 [`draw_area_field`] 绘制：一帧里只测量一次宽度、只做一次折行。
pub(super) struct AreaWrap {
    /// 排版用的文字：输入法组字中的文字插在光标处。
    display: String,
    /// 组字中的文字的字节数。
    preedit: usize,
    /// 每个显示行的起点（`display` 里的字节偏移）。
    row_starts: Vec<usize>,
    /// 组字文字的起点和终点：（行，x）；没有组字时都是光标的位置。
    preedit_span: ((u32, f32), (u32, f32)),
    /// 每个字素边界的（输入框文字里的字节偏移，x），见 [`FieldView::stops`]。
    stops: Vec<(usize, f32)>,
    /// 每个边界在第几个显示行，见 [`FieldView::rows`]。
    rows: Vec<u32>,
}

impl AreaWrap {
    /// 折行以后有几行。
    pub(super) fn lines(&self) -> usize {
        self.row_starts.len()
    }
}

/// 把多行输入框 `field` 的文字按 `width` 折行：每个字素量出宽度，放不下就换到下一行，换行符
/// 开始新的一行；记下每个字素边界在哪一行、哪个位置（供点击、拖选和上下移动）。
///
/// 有焦点时输入法组字中的文字插在光标处一起折行，后面的字跟着往后排。
pub(crate) fn wrap_area_field(
    app: &App,
    frame: &mut Frame<'_>,
    width: f32,
    field: &mut TextField,
    focused: bool,
) -> AreaWrap {
    let preedit = if focused {
        app.field_preedit.as_str()
    } else {
        ""
    };
    let caret = field.caret();
    let pl = preedit.len();
    // 排版用的文字：组字中的文字插在光标处。记下的边界都换回输入框文字里的位置。
    let display = {
        let text = field.text();
        format!("{}{preedit}{}", &text[..caret], &text[caret..])
    };
    let to_field = |pos: usize| if pos <= caret { pos } else { pos - pl };
    let in_preedit = |pos: usize| pos > caret && pos <= caret + pl;

    // 复用上一帧的两张表，不必每帧重新分配。
    let mut stops = std::mem::take(&mut field.view.stops);
    let mut rows = std::mem::take(&mut field.view.rows);
    stops.clear();
    rows.clear();
    let (mut x, mut row) = (0.0, 0u32);
    // 每个显示行的起点（排版文字里的字节偏移）。
    let mut row_starts = vec![0];
    // 组字文字的起点和终点：（行，x）。
    let (mut pre_start, mut pre_end) = ((0u32, 0.0f32), (0u32, 0.0f32));
    stops.push((0, 0.0));
    rows.push(0);
    for (start, grapheme) in display.grapheme_indices(true) {
        let end = start + grapheme.len();
        if start == caret {
            pre_start = (row, x);
        }
        if grapheme == "\n" {
            row += 1;
            x = 0.0;
            row_starts.push(end);
        } else {
            let w = frame.measure(FontKind::Ui, app.fonts.ui, grapheme);
            if x > 0.0 && x + w > width {
                // 放不下：该字素换到下一行，它前面的边界也算到下一行的开头。
                row += 1;
                x = 0.0;
                row_starts.push(start);
                if !in_preedit(start)
                    && let (Some(last), Some(r)) = (stops.last_mut(), rows.last_mut())
                {
                    *last = (to_field(start), 0.0);
                    *r = row;
                }
                if start == caret {
                    pre_start = (row, 0.0);
                }
            }
            x += w;
        }
        if end == caret + pl {
            pre_end = (row, x);
        }
        if !in_preedit(end) {
            stops.push((to_field(end), x));
            rows.push(row);
        }
    }
    if pl == 0 {
        pre_start = stops
            .iter()
            .zip(&rows)
            .find(|((pos, _), _)| *pos >= caret)
            .map_or((0, 0.0), |((_, x), r)| (*r, *x));
        pre_end = pre_start;
    }
    AreaWrap {
        display,
        preedit: pl,
        row_starts,
        preedit_span: (pre_start, pre_end),
        stops,
        rows,
    }
}

/// 在 `area` 里绘制多行输入框 `field`：选区、文字、组字的下划线和光标，`wrap` 是按 `area` 的
/// 宽度折好的行（见 [`wrap_area_field`]）。
///
/// 文字超出高度时纵向滚动，使光标所在的行始终可见。
///
/// 有焦点时返回光标的矩形，供输入法候选框定位。
pub(crate) fn draw_area_field(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    field: &mut TextField,
    wrap: AreaWrap,
    placeholder: &str,
    focused: bool,
) -> Option<Rect> {
    let p = app.palette;
    let AreaWrap {
        display,
        preedit: pl,
        row_starts,
        preedit_span: (pre_start, pre_end),
        stops,
        rows,
    } = wrap;

    // 纵向滚动：光标或文字变了，光标（组字文字的末尾）所在的行露在区域里；都没变时保持
    // 滚轮设定的滚动量。滚动量不超出文字范围。
    let caret_row = pre_end.0;
    let visible = (area.h / AREA_LINE).floor().max(1.0) as u32;
    let last_row = row_starts.len() as u32 - 1;
    let follow = (field.caret(), display.len());
    let mut scroll = field.view.scroll;
    if field.view.follow != Some(follow) {
        let caret_top = caret_row as f32 * AREA_LINE;
        if caret_top < scroll {
            scroll = caret_top;
        } else if caret_row >= visible
            && caret_top + AREA_LINE > scroll + visible as f32 * AREA_LINE
        {
            scroll = (caret_row + 1 - visible) as f32 * AREA_LINE;
        }
    }
    let max = (last_row + 1).saturating_sub(visible) as f32 * AREA_LINE;
    let scroll = scroll.clamp(0.0, max);
    let top = area.y - scroll;
    field.view = FieldView {
        area,
        origin: area.x,
        scroll,
        stops,
        rows,
        top,
        line_height: AREA_LINE,
        measured: None,
        follow: Some(follow),
        wheeled: None,
    };
    let view = &field.view;
    let band = |row: u32| Rect::new(area.x, top + row as f32 * AREA_LINE, area.w, AREA_LINE);

    if display.is_empty() {
        ui_text(frame, app, area.x, band(0), placeholder, p.text_dim, area);
        // 光标与输入文字后的位置相同，位于文字起点，不向左越出文字区域。
        if focused && app.caret_visible {
            draw_field_caret(frame, p, area.x, band(0), area.inset(-3.0, 0.0));
        }
        return focused.then(|| Rect::new(area.x, area.y + 2.0, 2.0, 16.0));
    }

    // 选区：逐行绘制（有选区时没有组字，边界就是输入框里的位置）。边界按行排列，逐行向下计算。
    let (from, to) = field.selection();
    if from != to {
        let color = if focused {
            p.selection
        } else {
            p.selected_inactive
        };
        let mut start = 0;
        for r in 0..=last_row {
            let end = start + view.rows[start..].partition_point(|&row| row <= r);
            let in_row = &view.stops[start..end];
            start = end;
            let (Some(first), Some(last)) = (in_row.first(), in_row.last()) else {
                continue;
            };
            let (a, b) = (from.max(first.0), to.min(last.0));
            // 这一行里有选中的字，或者选区跨过了这一行末尾的换行符。
            let crosses_end = from <= last.0 && to > last.0;
            if a >= b && !crosses_end {
                continue;
            }
            let x_at = |pos: usize| in_row.iter().find(|s| s.0 >= pos).map_or(last.1, |s| s.1);
            // 选区跨过行尾时多绘制 6 逻辑像素，表示换行符也被选中。
            let tail = if to > last.0 { 6.0 } else { 0.0 };
            let (x0, x1) = (area.x + x_at(a), area.x + x_at(b) + tail);
            let rect = Rect::new(x0, band(r).y + 1.0, (x1 - x0).max(2.0), AREA_LINE - 2.0);
            frame.rect_clipped(rect, color, 3.0, area);
        }
    }

    // 文字：每个显示行一段，从这一行的起点到下一行的起点，去掉行尾的换行符。
    let color = if focused { p.text } else { p.text_muted };
    for (r, &start) in row_starts.iter().enumerate() {
        let end = row_starts.get(r + 1).copied().unwrap_or(display.len());
        let segment = display[start..end].trim_end_matches('\n');
        let line = band(r as u32);
        if line.bottom() > area.y && line.y < area.bottom() {
            ui_text(frame, app, area.x, line, segment, color, area);
        }
    }

    // 组字文字下面一道强调色的下划线（跨行时每行各绘制一段）。
    if pl > 0 {
        for r in pre_start.0..=pre_end.0 {
            let x0 = if r == pre_start.0 { pre_start.1 } else { 0.0 };
            let x1 = if r == pre_end.0 { pre_end.1 } else { area.w };
            let line = band(r);
            let underline = Rect::new(area.x + x0, line.y + line.h / 2.0 + 8.0, x1 - x0, 1.0);
            frame.rect_clipped(underline, p.accent, 0.0, area);
        }
    }

    let line = band(pre_end.0);
    let x = area.x + pre_end.1;
    if focused && app.caret_visible && from == to {
        draw_field_caret(frame, p, x, line, area.inset(-3.0, 0.0));
    }
    focused.then(|| Rect::new(x, line.y + 2.0, 2.0, 16.0))
}

/// 绘制输入光标：在 `row` 内垂直居中，不超出 `clip`。
fn draw_field_caret(frame: &mut Frame<'_>, p: &Palette, x: f32, row: Rect, clip: Rect) {
    let x = x.min(clip.right() - 2.0);
    frame.rect_clipped(
        Rect::new(x + 1.0, row.y + (row.h - 16.0) / 2.0, 1.5, 16.0),
        p.caret,
        0.0,
        clip,
    );
}

/// 绘制浮动条中的一个输入框：选区、文本、输入法组字和光标。
///
/// 文本比 `area` 宽时横向滚动，使光标始终可见。同时把本次绘制的几何信息写入 `field.view`，
/// 供点击和拖选时使用。
///
/// 有焦点时返回光标的矩形，供输入法候选框定位。
/// 遮盖输入框中一个圆点占的宽度（逻辑像素）。
const MASK_W: f32 = 10.0;
/// 遮盖输入框中圆点的直径（逻辑像素）：不随字体变化，圆点大小在各平台上一致。
const MASK_DOT: f32 = 6.0;

/// 从 `x` 起绘制 `count` 个圆点，返回末尾的横坐标。
fn draw_mask(frame: &mut Frame<'_>, x: f32, area: Rect, count: usize, color: Color) -> f32 {
    let y = area.y + (area.h - MASK_DOT) / 2.0;
    for i in 0..count {
        let dot_x = x + i as f32 * MASK_W + (MASK_W - MASK_DOT) / 2.0;
        let dot = Rect::new(dot_x, y, MASK_DOT, MASK_DOT);
        frame.rect_clipped(dot, color, MASK_DOT / 2.0, area);
    }
    x + count as f32 * MASK_W
}

pub(super) fn draw_bar_field(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    field: &mut TextField,
    placeholder: &str,
    focused: bool,
) -> Option<Rect> {
    let p = app.palette;
    let preedit = if focused {
        app.field_preedit.as_str()
    } else {
        ""
    };
    // 遮盖输入框把每个字素绘制为一个圆点，输入法组字中的文本同样遮盖。
    let masked = field.is_masked();
    let count = |text: &str| text.graphemes(true).count();

    // 记录每个字素簇（grapheme）边界相对文本起点的 x 偏移（逻辑像素）。界面字体不做字距调整，
    // 逐个字素量出的宽度之和与整段文本的宽度相同。文字和字号都没变时沿用上一帧量好的结果
    // （`FieldView::measured`）：输入框每帧都画，文字很长时逐个量是主要开销。
    let mut stops = std::mem::take(&mut field.view.stops);
    let size = app.fonts.ui;
    let fresh = field
        .view
        .measured
        .as_ref()
        .is_some_and(|(text, at)| text == field.text() && *at == size);
    if !fresh || stops.is_empty() {
        stops.clear();
        let mut width = 0.0;
        stops.push((0, 0.0));
        for (start, grapheme) in field.text().grapheme_indices(true) {
            width += if masked {
                MASK_W
            } else {
                frame.measure(FontKind::Ui, size, grapheme)
            };
            stops.push((start + grapheme.len(), width));
        }
        field.view.measured = Some((field.text().to_owned(), size));
    }
    let width = stops.last().map_or(0.0, |&(_, x)| x);

    // 仅当光标（连同输入法组字中的文本）超出可见范围时才横向滚动；文本变短时滚动量相应回收。
    let caret_x = stops[stops.partition_point(|&(pos, _)| pos < field.caret())].1;
    let preedit_w = if masked {
        count(preedit) as f32 * MASK_W
    } else {
        frame.measure(FontKind::Ui, app.fonts.ui, preedit)
    };
    let room = area.w - 4.0;
    let mut scroll = field.view.scroll;
    // 滚轮滚动之后，光标和文字都未变化时不跟随光标。
    let here = (field.caret(), field.text().len());
    let wheeled = field
        .view
        .wheeled
        .filter(|&at| at == here && preedit.is_empty());

    if wheeled.is_none() {
        if caret_x + preedit_w - scroll > room {
            scroll = caret_x + preedit_w - room;
        }
        scroll = scroll.min(caret_x);
    }

    scroll = scroll.min((width + preedit_w - room).max(0.0)).max(0.0);
    let origin = area.x - scroll;
    field.view = FieldView {
        area,
        origin,
        scroll,
        stops,
        measured: field.view.measured.take(),
        wheeled,
        ..FieldView::default()
    };

    let text = field.text();
    let caret_rect = |x: f32| Rect::new(x, area.y + (area.h - 16.0) / 2.0, 2.0, 16.0);

    if text.is_empty() && preedit.is_empty() {
        ui_text(frame, app, area.x, area, placeholder, p.text_dim, area);

        // 没有文本时，光标绘制在占位文本左侧，不覆盖占位文本的第一个字符。
        if focused && app.caret_visible {
            draw_field_caret(frame, p, area.x - 3.0, area, area.inset(-3.0, 0.0));
        }

        return focused.then(|| caret_rect(area.x));
    }

    let (from, to) = field.selection();

    if from != to {
        let (x0, x1) = (origin + field.view.x_of(from), origin + field.view.x_of(to));
        let rect = Rect::new(x0, area.y + (area.h - 20.0) / 2.0, x1 - x0, 20.0);
        let color = if focused {
            p.selection
        } else {
            p.selected_inactive
        };
        frame.rect_clipped(rect, color, 3.0, area);
    }

    // 输入法组字中的文本插在光标处，绘制在光标前后两段文本之间。
    let color = if focused { p.text } else { p.text_muted };
    let (before, after) = text.split_at(field.caret());
    let mut x = if masked {
        draw_mask(frame, origin, area, count(before), color)
    } else {
        ui_text(frame, app, origin, area, before, color, area)
    };

    if !preedit.is_empty() {
        let start = x;
        x = if masked {
            draw_mask(frame, x, area, count(preedit), p.text)
        } else {
            ui_text(frame, app, x, area, preedit, p.text, area)
        };
        let underline = Rect::new(start, area.y + area.h / 2.0 + 8.0, x - start, 1.0);
        frame.rect_clipped(underline, p.accent, 0.0, area);
    }

    if masked {
        draw_mask(frame, x, area, count(after), color);
    } else {
        ui_text(frame, app, x, area, after, color, area);
    }

    // 有选区时不显示光标，选区本身已指示位置。
    if focused && app.caret_visible && from == to {
        draw_field_caret(frame, p, x, area, area.inset(-3.0, 0.0));
    }

    focused.then(|| caret_rect(x))
}
