//! 执行编辑命令，处理剪贴板和光标定位，以及使光标保持在可见范围内的滚动。

use winc_i18n::t;

use super::*;

/// 括号配对缓存的键：文档、版本号和光标位置都不变时，配对结果不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BraceKey {
    /// 缓存对应的文档。
    pub doc: DocumentId,
    /// 缓存对应的文档版本号（`revision`）。
    pub revision: u64,
    /// 缓存对应的光标位置（字符偏移）。
    pub head: usize,
}

impl App {
    /// 返回行号栏的宽度（逻辑像素）：随行数的位数变化，至少按 2 位计算；不显示行号时只留两侧
    /// 给折叠箭头、书签和改动标记的空白。
    pub(in crate::app) fn gutter_width(&self) -> f32 {
        let digits = self.line_count().max(1).ilog10() as usize + 1;
        let numbers = if self.config.line_numbers == Some(false) {
            0.0
        } else {
            digits.max(2) as f32 * self.cell
        };
        numbers + 2.0 * GUTTER_PADDING + self.blame_width()
    }

    /// 返回自动换行时一个显示行可容纳的单元格数，至少 10 格（经验值，窗口极窄时仍能显示几个字）。
    ///
    /// 可用宽度为文本区宽度减去左侧留白 `TEXT_PAD` 和右侧滚动条 `SCROLLBAR_HIT`。
    fn wrap_cells(&self) -> usize {
        let room = self.layout.text.w - TEXT_PAD - SCROLLBAR_HIT;
        ((room / self.cell).floor() as usize).max(10)
    }

    /// 文本区宽度是否正在逐帧变化：侧栏或终端面板的动画进行中，或者正在拖动两者的分隔条。
    pub(in crate::app) fn text_width_changing(&self) -> bool {
        self.sidebar_motion.is_some()
            || self.terminal.motion.is_some()
            || self.terminal.grow.is_some()
            || matches!(
                self.drag,
                Drag::Splitter { .. } | Drag::TerminalDivider { .. } | Drag::SplitDivider { .. }
            )
    }

    /// 让当前文档按当前设置和宽度断行；仅在宽度、内容或开关变化时才重新计算。
    ///
    /// 重新断行后，屏幕最上面的那一行文字保持在原位，不会因为它上方各行的段数变化而移位。
    pub(in crate::app) fn sync_wrap(&mut self) {
        if self.sync_diff_wrap() {
            return;
        }
        let width = (self.config.word_wrap == Some(true)).then(|| self.wrap_cells());
        let tab_width = self.editor.active_tab_width();
        let resizing = self.text_width_changing();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if !doc.shows_text() {
            return;
        }
        let before = doc.wrap_width();
        // 整篇断行的耗时与行数成正比（20 万行约 115 ms），不能在宽度逐帧变化时每帧执行。此时沿用
        // 已有的断行宽度，动画结束或松开分隔条之后按最终宽度重新断行一次。
        let width = match (width, before) {
            (Some(_), Some((cells, _))) if resizing => Some(cells),
            _ => width,
        };
        let (top, part) = doc.line_and_part(doc.scroll_line.max(0.0) as usize);
        let fraction = doc.scroll_line.fract();
        let wrapping = doc.ensure_wrap(width, tab_width);
        if doc.wrap_width() != before {
            let part = part.min(doc.rows_of(top).saturating_sub(1));
            doc.scroll_line = (doc.row_of(top) + part) as f32 + fraction;
            if wrapping {
                doc.scroll_x = 0.0;
            }
            self.sticky_cache = None;
            self.fold_motion = None;
            self.clamp_scroll();
        }
    }

    /// 处理自动换行开关的变化：重新断行，视图顶部的文本行保持不动，并在状态栏提示结果。
    pub(in crate::app) fn word_wrap_changed(&mut self) {
        // 正在进行的平滑滚动按断行前的显示行计算，断行后已不再对应原来的内容。
        self.scroll_glide = None;
        self.relayout();
        let message = match (self.config.word_wrap == Some(true), self.editor.active()) {
            (true, Some(doc)) if doc.diff.as_deref().is_some_and(|v| !v.is_wrapping()) => {
                t!("editor-word-wrap-on-large")
            }
            (true, Some(doc)) if doc.diff.is_some() => t!("editor-word-wrap-on"),
            (true, Some(doc)) if !doc.is_wrapping() && doc.is_large() => {
                t!("editor-word-wrap-on-large")
            }
            (true, Some(doc)) if !doc.is_wrapping() => t!("editor-word-wrap-on-table"),
            (true, _) => t!("editor-word-wrap-on"),
            (false, _) => t!("editor-word-wrap-off"),
        };
        self.flash(message);
    }

    /// 返回缩略图最上面显示的行和滑块的位置。
    fn minimap_place(&self) -> (usize, Rect) {
        let area = self.layout.minimap;
        let scroll = self.editor.active().map_or(0.0, |doc| doc.scroll_line);
        let shown = (area.h / MINIMAP_ROW).ceil() as usize;
        let first = crate::layout::minimap_first_row(
            self.row_count(),
            shown,
            scroll,
            self.max_scroll_line(),
        );
        let slider = Rect::new(
            area.x,
            area.y + (scroll - first as f32) * MINIMAP_ROW,
            area.w,
            self.visible_lines() as f32 * MINIMAP_ROW,
        );
        (first, slider)
    }

    /// 返回绘制缩略图所需的状态：最上面显示的行、滑块的位置，以及滑块是否加深。
    pub(crate) fn minimap_view(&self) -> (usize, Rect, bool) {
        let active = self.hover == Hover::Minimap || matches!(self.drag, Drag::Minimap { .. });
        let (first, slider) = self.minimap_place();
        (first, slider, active)
    }

    /// 在缩略图上按下，开始拖动滑块。
    ///
    /// 点在滑块外时视为按在滑块中心：滑块移到指针下面，之后继续按指针的位移拖动。
    pub(in crate::app) fn press_minimap(&mut self, y: f32) {
        let (_, slider) = self.minimap_place();
        let scroll = self.editor.active().map_or(0.0, |doc| doc.scroll_line);
        let from = if y < slider.y || y >= slider.bottom() {
            slider.y + slider.h / 2.0
        } else {
            y
        };
        self.drag = Drag::Minimap { from, scroll };
        self.drag_minimap(y, from, scroll);
    }

    /// 拖动缩略图的滑块：滚动位置从按下时的 `scroll` 起，按指针相对按下点 `from` 的位移变化，
    /// 指针不动时滚动位置不变。
    ///
    /// 滑块滚到底时的位置就是它能移动的距离，滑块因此跟着指针移动。
    pub(in crate::app) fn drag_minimap(&mut self, y: f32, from: f32, scroll: f32) {
        let total = self.row_count();
        let shown = (self.layout.minimap.h / MINIMAP_ROW).ceil() as usize;
        let max = self.max_scroll_line();
        let first = crate::layout::minimap_first_row(total, shown, max, max);
        let room = (max - first as f32) * MINIMAP_ROW;
        if room > 0.0 {
            self.set_minimap_scroll(scroll + (y - from) / room * max);
        }
    }

    /// 把当前文档滚动到第 `line` 行（可以有小数），不播放平滑滚动。
    fn set_minimap_scroll(&mut self, line: f32) {
        let max = self.max_scroll_line();
        self.scroll_glide = None;
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_line = line.clamp(0.0, max);
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 返回当前文档的文件行数；没有打开文档时为 0。
    pub(super) fn line_count(&self) -> usize {
        self.editor.active().map_or(0, |d| d.text().line_count())
    }

    /// 返回当前文档的显示行数；已折叠的行不计入。
    pub(in crate::app) fn row_count(&self) -> usize {
        self.editor.active().map_or(0, |d| d.row_count())
    }

    /// 返回文本区可容纳的行数，至少为 1。
    pub(in crate::app) fn visible_lines(&self) -> usize {
        (self.layout.text.h / self.fonts.line_height)
            .floor()
            .max(1.0) as usize
    }

    /// 返回翻页时移动的行数：可见行数减 1，至少为 1。
    fn page_lines(&self) -> usize {
        self.visible_lines().saturating_sub(1).max(1)
    }

    /// 返回 `scroll_line` 的最大值（以显示行计）：内容放得下时不滚；放不下时滚到底，最后一行
    /// 下面留四成屏的空白（[`OVERSCROLL`](crate::layout::OVERSCROLL)）。
    pub(in crate::app) fn max_scroll_line(&self) -> f32 {
        crate::layout::editor_max_scroll(
            self.row_count(),
            self.scroll_track().h,
            self.fonts.line_height,
        )
    }

    /// 返回编辑区内容的总高度（逻辑像素），供滚动条使用：能滚的距离加一屏。
    pub(in crate::app) fn content_height(&self) -> f32 {
        editor_content_height(
            self.row_count(),
            self.scroll_track().h,
            self.fonts.line_height,
        )
    }

    /// 编辑区竖直滚动条的轨道：差异标签让出顶上的标题栏，滑块、概览标尺和点的范围都不伸到
    /// 标题栏里的按钮旁边。
    ///
    /// 绘制和点击共用。
    pub(crate) fn scroll_track(&self) -> Rect {
        let editor = self.layout.editor;
        // 日志标签：滚动条在左边提交列表的右边，详情一栏不滚这一根。
        if self.log_active() {
            return self.log_parts().list;
        }
        if self.editor.active().is_some_and(|d| d.diff.is_some()) {
            let header = crate::app::diff::DIFF_HEADER;
            Rect::new(
                editor.x,
                editor.y + header,
                editor.w,
                (editor.h - header).max(0.0),
            )
        } else {
            editor
        }
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）对应的文档位置（字符偏移）。
    ///
    /// 位置在最后一个显示行之下时返回文档末尾；没有打开文档时返回 `None`。
    pub(in crate::app) fn pos_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        let t = self.layout.text;
        let row = ((y - t.y) / self.fonts.line_height + doc.scroll_line)
            .floor()
            .max(0.0) as usize;

        if row >= doc.row_count() {
            return Some(doc.text().end_pos());
        }
        let cell = ((x - (t.x + TEXT_PAD - doc.scroll_x)) / self.cell)
            .round()
            .max(0.0) as usize;
        if doc.is_wrapping() {
            return Some(doc.pos_in_row(row, cell, self.editor.active_tab_width()));
        }
        let line = doc.line_of_row(row);
        let col = doc.cell_to_col(line, cell, self.editor.active_tab_width());
        let pos = doc.text().line_col_to_pos(line, col);
        Some(winc_core::movement::snap_to_grapheme(doc.text(), pos))
    }

    /// 在编辑区纵向滚动位置从 `from` 变为当前值后调用，距离较远时启动平滑滚动。
    ///
    /// 距离不小于 [`GLIDE_MIN_ROWS`] 行时按 [`ScrollGlide`] 平滑滚动，否则直接到达目标。
    /// 已在滚动时从屏幕上的当前位置继续滚动，不跳回起点。减弱动态效果或没有窗口时不做动画。
    pub(in crate::app) fn glide_scroll(&mut self, from: f32) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (id, to) = (doc.id, doc.scroll_line);
        let shown = match &self.scroll_glide {
            Some(g) if g.doc == id => g.clock.between(g.from, from),
            _ => from,
        };
        if (to - shown).abs() < GLIDE_MIN_ROWS {
            // 距离较近时直接到达；同一文档正在进行的平滑滚动保留，继续滚向新目标。
            if self.scroll_glide.as_ref().is_some_and(|g| g.doc != id) {
                self.scroll_glide = None;
            }
            return;
        }
        self.scroll_glide = self
            .clock(crate::app::animation::SCROLL_GLIDE)
            .map(|clock| ScrollGlide {
                doc: id,
                from: shown,
                clock,
            });
    }

    /// 绘制前调用：正在平滑滚动时，将当前文档的滚动位置替换为本帧的插值位置，并返回目标位置。
    ///
    /// 绘制完成后由 [`Self::end_glide_frame`] 恢复目标位置。其余时刻滚动位置始终等于目标，
    /// 翻页、跳转都按目标计算，连续触发时结果仍然正确。正在折叠或展开时取消平滑滚动，
    /// 因为折叠动画假定滚动位置不变。
    pub(in crate::app) fn begin_glide_frame(&mut self) -> Option<f32> {
        let glide = self.scroll_glide.as_ref()?;
        if self.fold_motion.is_some() {
            self.scroll_glide = None;
            return None;
        }
        let (doc_id, from, clock) = (glide.doc, glide.from, glide.clock);
        let doc = self.editor.active_mut().filter(|d| d.id == doc_id)?;
        let target = doc.scroll_line;
        doc.scroll_line = clock.between(from, target);
        Some(target)
    }

    /// 绘制完一帧后将滚动位置恢复为目标位置，见 [`Self::begin_glide_frame`]。
    pub(in crate::app) fn end_glide_frame(&mut self, target: Option<f32>) {
        if let Some(target) = target
            && let Some(doc) = self.editor.active_mut()
        {
            doc.scroll_line = target;
        }
    }

    /// 是否将键入送往编辑区：焦点位于编辑区，且窗口在前台。
    ///
    /// 返回 `false` 时选区按 `Palette::selection_inactive` 绘制。
    pub(crate) fn editor_focused(&self) -> bool {
        self.focus == Focus::Editor && self.window_focused
    }

    /// 滚动编辑区，使光标保持在可见范围内。
    ///
    /// 上下各留一行余量（顶部有固定行时，留出固定行占用的行数），左右各留 4 个单元格。
    /// 选区的任一端处在折叠的块内（查找、转到行、撤销都可能把光标移到折叠处）时，先展开。
    pub(in crate::app) fn ensure_caret_visible(&mut self) {
        let before = self.editor.active().map(|d| d.scroll_line);
        self.reveal_caret();
        if let Some(before) = before {
            self.glide_scroll(before);
        }
    }

    /// 滚动编辑区使光标可见，不做平滑滚动；供 [`Self::ensure_caret_visible`] 调用。
    fn reveal_caret(&mut self) {
        if let Some(doc) = self.editor.active_mut() {
            let selection = doc.selection;
            doc.reveal(selection.anchor);
            doc.reveal(selection.head);
        }
        let visible = self.visible_lines() as f32;
        let caret_line = self
            .editor
            .active()
            .map_or(0, |d| d.text().line_of(d.selection.head));
        let top = (self.sticky_depth(caret_line) as f32).max(1.0);
        let width = self.layout.text.w;
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();
        let max_scroll = self.max_scroll_line();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
        // 自动换行时按光标所在的显示行计算，且无需横向滚动。
        let row = doc.row_at(doc.selection.head) as f32;

        if row < doc.scroll_line + top {
            doc.scroll_line = (row - top).max(0.0);
        } else if row > doc.scroll_line + visible - 2.0 {
            doc.scroll_line = (row - visible + 2.0).max(0.0);
        }

        doc.scroll_line = doc.scroll_line.clamp(0.0, max_scroll.max(row));

        if doc.is_wrapping() {
            doc.scroll_x = 0.0;
            return;
        }
        // 按列对齐的 CSV 先计算并记录当前版本的列信息，后面的换算和绘制都使用该结果。
        doc.columns(tab_width);
        let caret_x = doc.col_to_cell(line, col, tab_width) as f32 * cell;
        let margin = cell * 4.0;
        let right = width - TEXT_PAD - margin - SCROLLBAR_HIT;

        if caret_x < doc.scroll_x + margin {
            doc.scroll_x = (caret_x - margin).max(0.0);
        } else if caret_x > doc.scroll_x + right {
            doc.scroll_x = caret_x - right;
        }
    }

    /// 把当前文档的纵向和横向滚动位置限制在有效范围内。
    pub(in crate::app) fn clamp_scroll(&mut self) {
        let max_scroll = self.max_scroll_line();
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.scroll_line = doc.scroll_line.clamp(0.0, max_scroll);
        if let Some(view) = doc.diff.as_deref() {
            if view.is_wrapping() {
                doc.scroll_x = 0.0;
                return;
            }
            // 差异的两侧共用一个横向滚动位置，按两侧最长的一行、半个编辑区宽的文字区限制。
            let text_w = crate::app::diff_parts(self.layout.editor, view, cell).panes[1]
                .text
                .w;
            let widest = view.widest(tab_width) as f32 * cell;
            let max_x = (widest - text_w + crate::app::DIFF_TEXT_PAD + cell * 8.0).max(0.0);
            doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
            return;
        }
        if doc.is_wrapping() {
            doc.scroll_x = 0.0;
            return;
        }

        let max_x = self.text_max_scroll_x();
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_x = doc.scroll_x.clamp(0.0, max_x);
        }
    }

    /// 返回编辑区水平滚动条的轨道：文字区，右端让出竖直滚动条。
    pub(crate) fn text_hscroll_track(&self) -> Rect {
        let t = self.layout.text;
        Rect::new(t.x, t.y, (t.w - SCROLLBAR_HIT).max(0.0), t.h)
    }

    /// 返回当前文档文字的最大横向滚动量（逻辑像素）；差异视图和自动换行时为 0。
    ///
    /// 范围只按可见行中最长的一行计算。
    pub(crate) fn text_max_scroll_x(&mut self) -> f32 {
        let width = self.layout.text.w;
        let visible = self.visible_lines() + 1;
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut() else {
            return 0.0;
        };
        if doc.diff.is_some() || doc.is_wrapping() {
            return 0.0;
        }

        let segments =
            doc.folds()
                .segments(doc.scroll_line as usize, visible, doc.text().line_count());
        let mut widest = 0;
        // CSV 按列对齐时，行宽按最后一列的起始位置加上该行自身的宽度估算，结果可能偏大，
        // 仅用于限制滚动范围。
        let columns = doc.columns(tab_width);
        let shift = columns
            .as_ref()
            .map_or(0, |c| c.starts.last().copied().unwrap_or(0));

        for range in segments {
            doc.text()
                .visit_lines(range.start, MAX_LINE_BYTES, |line, _, content| {
                    if line >= range.end {
                        return std::ops::ControlFlow::Break(());
                    }

                    widest = widest
                        .max(shift + text::col_to_cell(content.into(), MAX_LINE_CHARS, tab_width));
                    std::ops::ControlFlow::Continue(())
                });
        }
        (widest as f32 * cell - width + TEXT_PAD + cell * 8.0).max(0.0)
    }

    /// 写入剪贴板。
    ///
    /// `whole_line` 为 `true` 表示复制的是整行，粘贴到编辑区时插入当前行上方。
    pub(in crate::app) fn set_clipboard(&mut self, text: String, whole_line: bool) {
        self.line_clipboard = whole_line.then(|| text.clone());

        #[cfg(test)]
        {
            self.copied = Some(text);
            self.copied_files.clear();
            self.copied_image = None;
        }
        #[cfg(not(test))]
        if let Some(clipboard) = self.system_clipboard() {
            let _ = clipboard.set_text(text);
        }
    }

    /// 返回剪贴板中的文本；测试中返回 `copied` 的内容，没有时返回 `None`。
    pub(in crate::app) fn clipboard_text(&mut self) -> Option<String> {
        #[cfg(test)]
        {
            self.copied.clone()
        }
        #[cfg(not(test))]
        {
            self.system_clipboard()?.get_text().ok()
        }
    }

    /// 返回系统剪贴板，首次调用时创建；创建失败时返回 `None`。
    #[cfg(not(test))]
    pub(super) fn system_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// 复制选区（`cut` 为 `true` 时剪切）；没有选区时操作光标所在的整行。
    pub(in crate::app) fn copy(&mut self, cut: bool) {
        // 日志标签：详情里选中了字就复制选中的字；没选中时复制选中提交的完整提交号（和点详情里
        // 的提交号一样）。
        if self.log_active() {
            if cut {
                return;
            }
            match self.log_selected_text() {
                Some(text) => {
                    self.set_clipboard(text, false);
                    self.flash(t!("editor-selection-copied"));
                }
                None => self.press_log(crate::app::git::log::LogTarget::CopyOid),
            }
            return;
        }
        // 表格：复制选中的单元格（表格只读，剪切也只复制）。
        if self.table_focused() {
            self.copy_table();
            return;
        }
        // PDF 里只复制选中的字（PDF 的文字只读，剪切也只复制）。
        if self.pdf_focused() {
            if let Some(text) = self.pdf_selected_text() {
                self.set_clipboard(text, false);
                self.flash(t!("editor-selection-copied"));
            }
            return;
        }
        // 预览里只复制选中的字（预览只读，剪切也只复制）。
        if self.markdown_focused() {
            if let Some(text) = self.markdown_selected_text() {
                self.set_clipboard(text, false);
                self.flash(t!("editor-selection-copied"));
            }
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        if doc.has_cursors() {
            self.copy_each(cut);
            return;
        }
        let selection = doc.selection;
        let whole_line = selection.is_empty();
        let range = if whole_line {
            doc.text().line_span(doc.text().line_of(selection.head))
        } else {
            selection.range()
        };

        let Some(mut content) = doc.text_range(range.start, range.end) else {
            self.flash(t!(
                "editor-selection-too-large-to-copy",
                size = COPY_LIMIT >> 20
            ));
            return;
        };

        // 整行复制到文件最后一行（末尾没有换行符）时补上行尾换行符，使粘贴时仍是完整的一行。
        if whole_line && range.end == doc.text().end_pos() {
            content.push_str(doc.line_ending.as_str());
        }

        self.set_clipboard(content, whole_line);

        if cut {
            self.run(if whole_line {
                Command::DeleteLine
            } else {
                Command::Delete { word: false }
            });
        }
    }

    /// 多光标时复制（`cut` 为 `true` 时剪切）：各选区的文字按位置顺序用换行符连起来；全是光标时
    /// 复制各光标所在的整行（同一行只复制一次）。
    ///
    /// 粘贴时若剪贴板的行数等于光标个数，每个光标各插入一行，见 `Document::execute`。
    fn copy_each(&mut self, cut: bool) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (all, _) = doc.selections();
        let text = doc.text();
        let ending = doc.line_ending.as_str();
        let whole_line = all.iter().all(|s| s.is_empty());
        let mut parts = Vec::with_capacity(all.len());
        let mut copied = 0;
        let mut last_line = None;
        for selection in &all {
            let range = if whole_line {
                let line = text.line_of(selection.head);
                if last_line.replace(line) == Some(line) {
                    continue;
                }
                text.line_span(line)
            } else {
                selection.range()
            };
            copied += text.byte_len(range.start, range.end);
            let part = (copied <= COPY_LIMIT)
                .then(|| doc.text_range(range.start, range.end))
                .flatten();
            let Some(mut part) = part else {
                self.flash(t!(
                    "editor-selection-too-large-to-copy",
                    size = COPY_LIMIT >> 20
                ));
                return;
            };
            if whole_line && range.end == text.end_pos() {
                part.push_str(ending);
            }
            parts.push(part);
        }

        // 整行各自带着行尾的换行符，直接相连；选中的文字之间补上换行符。
        let content = parts.join(if whole_line { "" } else { ending });
        self.set_clipboard(content, whole_line);

        if cut {
            // 剪切只删掉选中的文字：空的选区（光标）替换成空串不算修改。
            self.run(if whole_line {
                Command::DeleteLine
            } else {
                Command::Insert(String::new())
            });
        }
    }

    /// 粘贴剪贴板内容。
    ///
    /// 焦点在输入框时插入文本；在编辑区时粘贴文件、文本或图片，在文件树中粘贴文件或图片
    /// （见 `paste` 模块）。
    pub(in crate::app) fn paste(&mut self) {
        match self.focus {
            focus if focus.is_field() => {
                if let Some(content) = self.clipboard_text() {
                    self.edit_field(|field| field.insert(&content));
                }
            }
            Focus::Editor => self.paste_into_editor(),
            Focus::Tree => self.paste_into_tree(),
            Focus::Terminal => self.terminal_paste(),
            _ => {}
        }

        self.request_redraw();
    }

    /// 在编辑区粘贴文本 `content`。
    ///
    /// 没有选区且 `content` 等于最近一次整行复制的内容时，插入到当前行上方；否则在光标处插入。
    /// Markdown 中选中文字后粘贴网址时生成链接，见 [`App::markdown_link_paste`]。
    pub(super) fn paste_text(&mut self, content: String) {
        if let Some(link) = self.markdown_link_paste(&content) {
            self.run(Command::Insert(link));
            return;
        }
        let empty_selection = self
            .editor
            .active()
            .is_some_and(|d| d.selections().0.iter().all(|s| s.is_empty()));
        let whole_line =
            empty_selection && self.line_clipboard.as_deref() == Some(content.as_str());
        self.run(if whole_line {
            Command::InsertLinesAbove(content)
        } else {
            Command::Paste(content)
        });
    }

    /// 对当前文档执行编辑命令 `command`，并更新布局、滚动位置和光标闪烁状态。
    ///
    /// 文档不可编辑时在状态栏提示原因，不执行命令。
    pub(in crate::app) fn run(&mut self, command: Command) {
        let page = self.page_lines();
        let tab_width = self.editor.active_tab_width();
        let lines_before = self.line_count();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };

        // 没有可编辑文本的文档（无法打开的文件、图片、SVG 的预览）不处理编辑命令；Markdown 的
        // 预览可以直接编辑。
        if !doc.edits_text() {
            return;
        }

        if let Some(reason) = doc.edit_blocked(&command) {
            self.flash(reason);
            return;
        }

        doc.execute(command, page, tab_width);

        // 行数变化可能改变行号的位数，因此需要重新布局行号栏。
        if self.line_count() != lines_before {
            self.relayout();
        }

        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 把当前文档的选区设为从 `from` 到 `to`（字符偏移），并滚动到光标。
    pub(in crate::app) fn select(&mut self, from: usize, to: usize) {
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = Selection::new(from, to);
            doc.goal_cell = None;
        }

        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }
}

/// 触发平滑滚动的最小滚动距离（显示行）。
///
/// 键入或上下移动光标通常只滚动一两行，此时直接到达；否则连续按方向键时画面会滞后于光标。
const GLIDE_MIN_ROWS: f32 = 2.0;

/// 编辑区平滑滚动的状态。
///
/// 文档的滚动位置已经是目标，绘制时按时钟从 `from` 插值到目标。
#[derive(Debug, Clone)]
pub struct ScrollGlide {
    /// 正在滚动的文档；切换标签后停止平滑滚动。
    pub doc: DocumentId,
    /// 开始时屏幕上的滚动位置（显示行）。
    pub from: f32,
    /// 本次平滑滚动的时钟。
    pub clock: Clock,
}
