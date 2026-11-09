//! 移动鼠标：悬停、各种拖动、拖选文字时贴边滚动。

use super::*;

impl App {
    /// 处理鼠标移动到窗口坐标 (`x`, `y`)（逻辑像素）：更新悬停、指针样式和进行中的拖动。
    pub(in crate::app) fn on_mouse_move(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        self.update_scrollbar_hover(x, y);

        // 鼠标离开顶栏：解除关闭标签时锁定的宽度，标签重新排开。
        if y >= TOP_BAR_END && self.tabs.width_lock.take().is_some() {
            self.request_redraw();
        }

        // 窗口内的提示框、菜单在最上面：打开期间只有它们响应悬停。
        if self.alert_mouse_move(x, y) || self.popup_mouse_move(x, y) {
            return;
        }
        // 对话框和设置浮层覆盖整个窗口：下方的按钮、文件树都不响应悬停，只判断浮层内的各部分。
        if self.form_dialog.is_some() {
            self.hover_form_dialog(x, y);
            return;
        }
        if self.settings.open {
            // 拖动滚动条时下方的设置项不高亮；滑块在拖动和悬停时加深颜色。
            let dragging = match self.drag {
                Drag::Scrollbar {
                    bar: Scrollbar::Settings,
                    grab,
                } => {
                    self.drag_settings_scrollbar(y, grab);
                    true
                }
                _ => false,
            };
            let part = (!dragging).then(|| self.settings_part_at(x, y)).flatten();
            if part != self.settings.hover {
                self.settings.hover = part;
                self.request_redraw();
            }
            let on_bar = dragging || part.is_none() && self.settings_scrollbar_at(x, y).is_some();
            self.set_hover(if on_bar {
                Hover::Scrollbar(Scrollbar::Settings)
            } else {
                Hover::None
            });

            self.update_cursor_icon();

            if self.drag == Drag::Field
                && let Some(field) = self.focused_field()
            {
                let pos = field.view.position_at_point(x, y);
                field.set_caret(pos, true);
                self.request_redraw();
            }
            return;
        }

        // 鼠标位于全屏查看的图上或正在拖动图时，被遮住的部件不响应悬停。
        if self.diagram_view_mouse_move(x, y) {
            return;
        }

        self.track_tree_segment();
        if self.tree_drag.is_some() {
            self.drag_tree_item(x, y);
        }
        if self.tabs.drag.is_some() {
            self.drag_tab(x);
        }
        self.drag_terminal_tab(x);

        self.set_hover(self.hover_at(x, y));
        let code = self.markdown.codes.iter().position(|r| r.contains(x, y));
        if code != self.markdown.code_hover {
            self.markdown.code_hover = code;
            self.request_redraw();
        }
        if self.terminal.link.is_some() || self.layout.terminal.contains(x, y) {
            self.update_terminal_link();
        }
        self.update_cursor_icon();

        match self.drag {
            Drag::TerminalSelect | Drag::TerminalMouse(_) => self.terminal_mouse_drag(x, y),
            Drag::None if self.layout.terminal.contains(x, y) => self.terminal_mouse_hover(x, y),
            Drag::Select { by_word, origin } => self.drag_select(x, y, by_word, origin),
            Drag::SelectLine { line } => self.drag_select_line(x, y, line),
            Drag::Columns { anchor } => self.drag_columns(x, y, anchor),
            Drag::AddSelect { origin } => self.drag_add_selection(x, y, origin),
            Drag::DiffSelect { unit, origin, add } => {
                self.drag_diff_select(x, y, unit, origin, add);
            }
            Drag::DiffColumns { anchor } => self.drag_diff_columns(x, y, anchor),
            Drag::DiffSelectRead { side, unit, origin } => {
                self.drag_diff_select_read(x, y, side, unit, origin);
            }
            Drag::Splitter {
                start_x,
                start_width,
            } => {
                // 宽度拖到最小宽度的一半以下时收起侧栏，拖回该阈值以上时重新打开。
                // 收起时宽度恢复为拖动前的值，之后用 `⌘B` 打开仍是原来的宽度。
                let width = start_width + x - start_x;
                if width < MIN_PANEL_WIDTH / 2.0 {
                    if self.panel_visible {
                        self.panel_width = start_width;
                        self.set_sidebar(false);
                        if self.focus_in_sidebar() {
                            self.focus = Focus::Editor;
                        }
                    }
                    return;
                }
                let max = (self.layout.window.w - 320.0).max(MIN_PANEL_WIDTH);
                self.panel_width = width.clamp(MIN_PANEL_WIDTH, max);
                if self.panel_visible {
                    self.relayout();
                    self.request_redraw();
                } else {
                    self.set_sidebar(true);
                }
            }
            Drag::OutlineDivider { start, size } => self.drag_outline_divider(start, size, y),
            Drag::TerminalDivider { start, size } => {
                // 往上（在右边时往左）拖面板变大；限制在排版允许的范围里再记下，
                // 记的就是画出来的大小。
                if self.terminal_right() {
                    self.terminal.width = size - (x - start);
                    self.relayout();
                    self.terminal.width = self.layout.terminal.w;
                } else {
                    self.terminal.height = size - (y - start);
                    self.relayout();
                    self.terminal.height = self.layout.terminal.h;
                }
                self.request_redraw();
            }
            Drag::SplitDivider {
                start_x,
                start_left,
            } => {
                // 比例按分开之前编辑区的总宽度计算，排版时再夹在两边的最小宽度之间。
                let total = self.layout.editor.w + self.layout.split.w + 1.0;
                if total > 0.0 {
                    let left = start_left + (x - start_x);
                    self.split_share = Some((left / total).clamp(0.0, 1.0));
                    self.relayout();
                    self.request_redraw();
                }
            }
            Drag::LogDivider {
                start_x,
                start_width,
            } => {
                // 往左拖详情变宽。夹在排版允许的范围里再存，存的就是画出来的宽度。
                self.log.details_w = Some(start_width - (x - start_x));
                self.log.details_w = Some(self.log_parts().details.w);
                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: bar @ (Scrollbar::Tree | Scrollbar::TreeHorizontal),
                grab,
            } => {
                let tree = self.layout.tree;

                if bar == Scrollbar::TreeHorizontal {
                    let max = self.tree_max_scroll_x();

                    if let Some(thumb) =
                        hscrollbar_thumb(tree, self.tree_scroll_x, self.tree_content_w)
                    {
                        self.tree_scroll_x =
                            scroll_for_thumb(tree.x, tree.w, thumb.w, x, grab, max);
                    }
                } else {
                    let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
                    let content = (rows as f32 + 1.0) * TREE_ROW;

                    if let Some(thumb) = scrollbar_thumb(tree, self.tree_scroll, content) {
                        self.tree_scroll = scroll_for_thumb(
                            tree.y,
                            tree.h,
                            thumb.h,
                            y,
                            grab,
                            tree_max_scroll(tree, rows),
                        );
                    }
                }

                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: Scrollbar::Panel,
                grab,
            } => {
                let results = self.panel_parts().results;
                let content = (self.global.rows().len() as f32 + 1.0) * TREE_ROW;
                if let Some(thumb) = scrollbar_thumb(results, 0.0, content) {
                    self.global.scroll = scroll_for_thumb(
                        results.y,
                        results.h,
                        thumb.h,
                        y,
                        grab,
                        self.results_max_scroll(),
                    );
                }
                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: Scrollbar::Commit,
                grab,
            } => {
                let list = self.commit_parts().list;
                let max = self.commit_max_scroll();
                if let Some(thumb) = scrollbar_thumb(list, 0.0, max + list.h) {
                    self.commit.scroll = scroll_for_thumb(list.y, list.h, thumb.h, y, grab, max);
                }
                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: Scrollbar::Outline,
                grab,
            } => {
                let list = self.outline_list();
                let max = self.outline_max_scroll();
                if let Some(thumb) = scrollbar_thumb(list, 0.0, max + list.h) {
                    self.outline.scroll = scroll_for_thumb(list.y, list.h, thumb.h, y, grab, max);
                }
                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: Scrollbar::Terminal,
                grab,
            } => self.drag_terminal_scrollbar(y, grab),
            Drag::Scrollbar {
                bar: Scrollbar::Markdown,
                grab,
            } => {
                let area = self.markdown.area;
                let content = self.markdown.height;
                if let Some(thumb) = scrollbar_thumb(area, 0.0, content) {
                    let max = content - area.h;
                    let to = scroll_for_thumb(area.y, area.h, thumb.h, y, grab, max);
                    self.scroll_markdown_to(to);
                }
            }
            Drag::Scrollbar {
                bar: Scrollbar::Editor,
                grab,
            } => {
                let track = self.scroll_track();
                let max = self.max_scroll_line();

                if let Some(thumb) = scrollbar_thumb(track, 0.0, self.content_height())
                    && let Some(doc) = self.editor.active_mut()
                {
                    doc.scroll_line = scroll_for_thumb(track.y, track.h, thumb.h, y, grab, max);
                }

                self.request_redraw();
            }
            Drag::Scrollbar {
                bar: Scrollbar::EditorHorizontal,
                grab,
            } => {
                let track = self.text_hscroll_track();
                let content = self.text_content_w;
                if let Some(doc) = self.editor.active_mut()
                    && let Some(thumb) = hscrollbar_thumb(track, doc.scroll_x, content)
                {
                    let max = content - track.w;
                    doc.scroll_x = scroll_for_thumb(track.x, track.w, thumb.w, x, grab, max);
                }
                self.request_redraw();
            }
            // 设置浮层打开时在上面处理，不会走到这里。
            Drag::Scrollbar {
                bar: Scrollbar::Settings,
                ..
            } => {}
            Drag::Minimap { from, scroll } => self.drag_minimap(y, from, scroll),
            Drag::LogText => self.drag_log_text(x, y),
            Drag::MarkdownText => self.drag_markdown(x, y),
            Drag::PdfText => self.drag_pdf(x, y),
            Drag::TableCells => self.drag_table(x, y),
            Drag::Field => {
                if let Some(field) = self.focused_field() {
                    let pos = field.view.position_at_point(x, y);
                    field.set_caret(pos, true);
                }

                self.request_redraw();
            }
            Drag::None => {}
        }
    }

    /// 处理拖选：鼠标拖到编辑区上下边缘之外时自动滚动。
    ///
    /// `by_word` 为 `true` 时按词扩展选区；`origin` 为拖选起始范围的起点和终点（字符偏移）。
    fn drag_select(&mut self, x: f32, y: f32, by_word: bool, origin: (usize, usize)) {
        let t = self.layout.text;
        let scrolled = self.scroll_while_dragging(Some(x), y);

        if let Some(pos) = self.pos_at(x.clamp(t.x, t.right()), y.clamp(t.y, t.bottom() - 1.0))
            && let Some(doc) = self.editor.active_mut()
        {
            doc.selection = if !by_word {
                Selection::new(origin.0, pos)
            } else if pos < origin.0 {
                Selection::new(origin.1, movement::word_at(doc.text(), pos).0)
            } else {
                Selection::new(origin.0, movement::word_at(doc.text(), pos).1)
            };
        }

        // 只有拖出边缘并随之滚动时，才需要重新计算横向可滚动的范围
        // （需要测量所有可见行的宽度）。
        if scrolled {
            self.clamp_scroll();
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 拖选时鼠标位于编辑区边缘之外，按超出的距离滚动；返回鼠标是否在边缘之外。
    ///
    /// `x` 为 `None` 时只按上下边缘滚动（拖选整行）。
    ///
    /// 滚动量按经过的时间计算（速度见 [`drag_scroll_speed`]），而不按鼠标事件的次数，因此鼠标
    /// 停在边缘外不动时仍逐帧滚动（[`App::drag_autoscrolling`]），滚动速度也不受鼠标晃动快慢影响。
    pub(in crate::app) fn scroll_while_dragging(&mut self, x: Option<f32>, y: f32) -> bool {
        let t = self.layout.text;
        let outside = if y < t.y {
            y - t.y
        } else if y > t.bottom() {
            y - t.bottom()
        } else {
            0.0
        };
        let right = self.text_hscroll_track().right();
        let outside_x = x.map_or(0.0, |x| {
            if x < t.x {
                x - t.x
            } else if x > right {
                x - right
            } else {
                0.0
            }
        });
        if outside == 0.0 && outside_x == 0.0 {
            self.drag_scrolled_at = None;
            return false;
        }
        let now = Instant::now();
        // 刚拖出边缘时立即滚动一帧的量；距上次滚动的间隔（如主线程阻塞后）最多按 100 ms 计算。
        let elapsed = self
            .drag_scrolled_at
            .map_or(self.frame_interval, |at| now - at)
            .min(std::time::Duration::from_millis(100));
        self.drag_scrolled_at = Some(now);
        let lh = self.fonts.line_height;
        let cell = self.cell;
        let seconds = elapsed.as_secs_f32();
        let max = self.max_scroll_line();
        if let Some(doc) = self.editor.active_mut() {
            if outside != 0.0 {
                let step = drag_scroll_speed(outside.abs() / lh) * seconds;
                doc.scroll_line = (doc.scroll_line + step.copysign(outside)).clamp(0.0, max);
            }
            if outside_x != 0.0 {
                let step = drag_scroll_speed(outside_x.abs() / cell) * cell * seconds;
                doc.scroll_x = (doc.scroll_x + step.copysign(outside_x)).max(0.0);
            }
        }
        true
    }

    /// 每帧绘制前调用：拖选停在编辑区边缘之外时，即使鼠标不动也按经过的时间滚动并扩展选区。
    pub(in crate::app) fn continue_drag_scroll(&mut self) {
        if self.drag_autoscrolling() {
            let (x, y) = self.mouse;
            self.on_mouse_move(x, y);
        }
    }

    /// 是否需要逐帧自动滚动：正在拖选或列选择，鼠标位于编辑区边缘之外，且该方向仍可滚动。
    pub(in crate::app) fn drag_autoscrolling(&self) -> bool {
        if self.drag == Drag::TerminalSelect {
            let view = self.terminal_view();
            let y = self.mouse.1;
            let Some((position, limit)) = self.terminal.tab().map(|t| t.scroll_position()) else {
                return false;
            };
            return (y < view.y && position < limit) || (y >= view.bottom() && position > 0.0);
        }
        let selecting = matches!(
            self.drag,
            Drag::Select { .. }
                | Drag::SelectLine { .. }
                | Drag::Columns { .. }
                | Drag::AddSelect { .. }
        );
        let (t, (x, y)) = (self.layout.text, self.mouse);
        let Some(doc) = self.editor.active() else {
            return false;
        };
        // 拖选整行与横向位置无关。
        let sideways = !matches!(self.drag, Drag::SelectLine { .. });
        let track = self.text_hscroll_track();
        selecting
            && ((y < t.y && doc.scroll_line > 0.0)
                || (y > t.bottom() && doc.scroll_line < self.max_scroll_line())
                || (sideways && x < t.x && doc.scroll_x > 0.0)
                || (sideways && x > track.right() && doc.scroll_x < self.text_content_w - track.w))
    }

    /// 拖选整行：三击或点击行号栏之后，按光标所在行与按下的那一行重选整段行。
    fn drag_select_line(&mut self, x: f32, y: f32, anchor: usize) {
        let t = self.layout.text;
        let scrolled = self.scroll_while_dragging(None, y);

        if let Some(pos) = self.pos_at(x, y.clamp(t.y, t.bottom() - 1.0))
            && let Some(doc) = self.editor.active_mut()
        {
            let line = doc.text().pos_to_line_col(pos).0;
            let (first, last) = if line < anchor {
                (line, anchor)
            } else {
                (anchor, line)
            };
            let start = doc.text().line_start(first);
            let end = doc.text().line_span(last).end;
            doc.selection = Selection::new(start, end);
        }

        if scrolled {
            self.clamp_scroll();
        }
        self.reset_blink();
        self.request_redraw();
    }
}

/// 返回鼠标超出编辑区边缘 `lines` 行时的滚动速度（行/秒）。
///
/// 紧贴边缘时为 6 行/秒，每远一行加 6 行/秒，上限 120 行/秒（经验值）：刚超出边缘时
/// 能逐行看清内容，拖远后可以快速到达文档两端。
fn drag_scroll_speed(lines: f32) -> f32 {
    (6.0 + 6.0 * lines).min(120.0)
}

impl App {
    /// 返回拖选超出边缘 `lines` 行时这一帧滚动的行数；距上次滚动的间隔最多按 100 ms 计算。
    pub(in crate::app) fn drag_scroll_lines(&mut self, lines: f32) -> f32 {
        let now = Instant::now();
        let elapsed = self
            .drag_scrolled_at
            .map_or(self.frame_interval, |at| now - at)
            .min(std::time::Duration::from_millis(100));
        self.drag_scrolled_at = Some(now);
        drag_scroll_speed(lines) * elapsed.as_secs_f32()
    }
}
