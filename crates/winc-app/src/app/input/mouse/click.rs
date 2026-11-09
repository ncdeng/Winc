//! 按下鼠标：先按焦点和浮层分，再落到各区域；连击的判断。

use super::*;

impl App {
    /// 处理鼠标按键 `button` 在当前鼠标位置按下。
    pub(in crate::app) fn on_mouse_down(&mut self, button: MouseButton) {
        // 选择面板的动画期间高度还在变，而点击按最终的布局：先取消动画，使显示直接到位。
        if self.picker_fold.take().is_some() {
            self.relayout();
        }
        let (x, y) = self.mouse;
        let l = self.layout;
        // 折叠动画期间绘制的是展开状态，而点击按的是折叠后的布局：先取消动画，使显示直接到位。
        self.fold_motion = None;
        self.dismiss_hint();

        // 窗口内的提示框、菜单打开时，这次按下只交给它们：点在菜单外面即关掉菜单。
        if (button == MouseButton::Left && self.alert_mouse_down())
            || self.alert.is_some()
            || self.popup_mouse_down(button)
        {
            return;
        }

        // 鼠标侧面的两个键：后退、前进。
        if matches!(button, MouseButton::Back | MouseButton::Forward) {
            if !self.settings.open && self.form_dialog.is_none() {
                self.navigate(button == MouseButton::Forward);
            }
            return;
        }

        // 点在标签上改名字的输入框以外：名字照输入的算。
        if self.terminal.rename.is_some() && self.field_at(x, y) != Some(Focus::TerminalRename) {
            self.finish_terminal_rename(true);
        }

        // 终端网格上的中键、右键：程序要了鼠标时上报给它（左键在下面按部件分发时处理）。
        if matches!(button, MouseButton::Middle | MouseButton::Right)
            && !self.settings.open
            && self.form_dialog.is_none()
            && self.terminal_view().contains(x, y)
            && !self
                .terminal_find_bar()
                .is_some_and(|bar| bar.contains(x, y))
            && self
                .terminal
                .tab()
                .is_some_and(|t| winc_pty::input::wants_mouse(t.modes))
            && !self.modifiers.shift_key()
        {
            self.terminal_mouse_down(x, y, button);
            return;
        }

        // 中键在编辑区的文字上拖动时进行列选择，效果等同于 `⇧⌥` 拖动。
        if button == MouseButton::Middle
            && !self.settings.open
            && self.form_dialog.is_none()
            && !self.picker_open
            && l.text.contains(x, y)
            && self.active_shows_text()
            && self.editor.active().is_some_and(|d| d.diff.is_none())
        {
            self.start_column_drag(x, y);
            return;
        }
        // 差异里可编辑一侧的文字上同样框出一列。
        if button == MouseButton::Middle
            && !self.settings.open
            && self.form_dialog.is_none()
            && !self.picker_open
            && self.over_diff_editable(x, y)
        {
            self.start_diff_columns(x, y);
            return;
        }

        if button == MouseButton::Middle {
            // 标签栏的位置取自上一帧的绘制：其间可能用快捷键关闭了标签，序号可能已不存在，
            // 因此用 `get` 取文档。
            self.close_pressed = self
                .tab_at(x, y)
                .and_then(|(index, _)| self.editor.documents.get(index))
                .map(|d| d.id);
            return;
        }

        // 右键菜单。macOS 上 `⌃` 加单击也弹出这个菜单，这是系统约定。
        let control_click = cfg!(target_os = "macos")
            && button == MouseButton::Left
            && self.modifiers.control_key();
        if button == MouseButton::Right || control_click {
            // 全屏查看的图上不提供右键菜单，被遮住的预览也不响应。
            if self
                .markdown
                .diagram
                .as_ref()
                .is_some_and(|v| v.area.contains(x, y))
            {
                return;
            }
            self.context_click(x, y);
            return;
        }

        if button != MouseButton::Left {
            return;
        }

        self.reset_blink();
        self.request_redraw();

        // 改动卡片：点按钮执行；点卡片里的其他位置什么都不做；点卡片外面关掉卡片，
        // 该事件仍然向下传递。
        if let Some(card) = self.changes.card {
            if let Some(target) = hit(&self.changes.hits, x, y) {
                self.press_change(target);
                return;
            }
            if card.contains(x, y) {
                return;
            }
            self.changes.popup = None;
        }

        // 设置浮层位于最上层，打开时所有点击都先交给它。
        if self.settings.open {
            self.click_settings(x, y);
            return;
        }
        if self.form_dialog.is_some() {
            self.click_form_dialog(x, y);
            return;
        }

        // 全屏查看的图位于编辑区上层，落在其范围内的点击只由查看器处理。
        if self.diagram_view_mouse_down(x, y) {
            return;
        }

        // 点在预览以外：预览的选区和焦点不要了（`click_markdown` 再拿回来）。拖预览的滚动条不算。
        let target = self.target_at(x, y);
        let keeps_preview = matches!(
            target,
            Target::MarkdownText | Target::Scrollbar(Scrollbar::Markdown, _)
        );
        // PDF 同理：点在 PDF 以外时选区和焦点不要了（`click_pdf` 再拿回来）。
        if !matches!(target, Target::PdfText) && self.pdf.focused {
            self.pdf.focused = false;
            self.pdf.selection = None;
        }
        if !keeps_preview && self.markdown.focused {
            self.markdown.focused = false;
            self.markdown.selection = None;
        }

        // 点击的目标由 `target_at` 决定（与悬停同一个顺序）；只有点击才有的规则写成前面的
        // 分支：快速打开面板开着时点其他位置关掉快速打开面板，正在输入名字时点其他位置
        // 先提交名字。
        match target {
            Target::Blocked | Target::None => {}
            Target::NoticeButton => self.run_notice_action(),
            Target::Change(target) => self.press_change(target),
            Target::Merge(target) => self.press_merge(target),
            Target::Conflict(action) => self.resolve_conflict(action),
            Target::Picker => {
                self.click_picker(x, y);
            }
            _ if self.picker_open => self.close_picker(),
            Target::Scrollbar(
                bar @ (Scrollbar::Panel | Scrollbar::Commit | Scrollbar::Outline),
                grab,
            ) => {
                self.grab_scrollbar(bar, grab);
            }
            Target::Panel(_) => {
                self.click_panel(x, y);
            }
            Target::Prompt => {
                self.focus = Focus::Prompt;
                self.click_field(x, y);
            }
            _ if self.name_input.is_some() => self.submit_name_input(),
            Target::Search => self.click_search(x, y),
            // 双击分隔条恢复默认宽度。
            Target::Splitter if self.click_count(x, y) == 2 => {
                self.panel_width = DEFAULT_PANEL_WIDTH;
                self.remember_panel_width();
                self.relayout();
                self.request_redraw();
            }
            // 从看到的宽度开始拖：记下的宽度可能比这个窗口放得下的大，否则开头一段拖了没有反应。
            Target::Splitter => {
                let most = (self.layout.window.w - 320.0).max(crate::theme::MIN_PANEL_WIDTH);
                self.drag = Drag::Splitter {
                    start_x: x,
                    start_width: self.panel_width.min(most),
                };
            }
            Target::TreeTool(tool) => self.run_tree_tool(tool),
            Target::TreeMore => self.open_tree_tools_menu(),
            Target::ProjectName => self.open_project_menu(),
            Target::SidebarButton => self.toggle_sidebar(),
            Target::NewTab => self.new_file(),
            Target::TabList => self.open_tab_list_menu(),
            Target::Preview(mode) => self.choose_preview_mode(mode),
            Target::Browser => self.open_in_browser(),
            Target::MarkdownLink(link) => self.open_markdown_link(link),
            Target::MarkdownToc(target) => self.press_toc(target),
            Target::MarkdownText => self.click_markdown(x, y),
            Target::PdfText => self.click_pdf(x, y),
            Target::Tab(index, true) => {
                self.close_pressed = self.editor.documents.get(index).map(|d| d.id);
            }
            Target::Tab(index, false) => {
                // 双击预览标签使其成为普通标签。
                let double = self.click_count(x, y) == 2;
                self.press_tab(index, x);
                if double {
                    self.keep_tab(index);
                }
            }
            Target::RulerChange(hunk) => self.go_to_change(hunk),
            Target::Scrollbar(bar, grab) => self.grab_scrollbar(bar, grab),
            Target::Minimap => self.press_minimap(y),
            Target::Crumb(index) => self.open_crumb_menu(index),
            Target::Status(item) => self.open_status_menu(item),
            Target::TitleBar => self.click_title_bar(x, y),
            Target::WindowControl(control) => self.press_window_control(control),
            Target::ResizeEdge(direction) => self.start_resize(direction),
            Target::Tree(_) => self.click_tree(x, y),
            Target::Placeholder(action) => self.run_placeholder_action(action),
            Target::Diff(target) => self.press_diff(target),
            Target::Log(crate::app::git::log::LogTarget::Divider) => {
                // 双击回到默认宽度。
                if self.click_count(x, y) == 2 {
                    self.log.details_w = None;
                    self.request_redraw();
                } else {
                    self.drag = Drag::LogDivider {
                        start_x: x,
                        start_width: self.log_parts().details.w,
                    };
                }
            }
            Target::Log(target) => {
                let count = self.click_count(x, y);
                self.press_log(target);
                // 双击一个提交时打开其改动的第一个文件，效果等同于按回车。
                if let (
                    2,
                    crate::app::git::log::LogTarget::Row(row)
                    | crate::app::git::log::LogTarget::Avatar(row),
                ) = (count, target)
                {
                    self.open_log_row_first_file(row);
                }
            }
            // 按住 `⌥` 点击最近的目录时在新窗口中打开，与项目菜单中 `⌥` 的含义保持统一。
            Target::Start(crate::app::StartAction::Recent(i)) if self.modifiers.alt_key() => {
                self.open_recent_in_new_window(i);
            }
            Target::Start(action) => self.run_start_action(action),
            Target::Sticky(row) => self.click_sticky(row, x),
            Target::Folded(line) => self.click_folded(line),
            Target::Blame(line) => self.click_blame(line),
            Target::ChangeMark(_) => {
                self.click_change_mark(x, y);
            }
            // 行号栏：点折叠箭头折叠或展开，点行号选中整行（由编辑区处理）。
            Target::Gutter => match self.fold_at(x, y) {
                Some(line) => self.click_fold(line),
                None => self.click_editor(x, y),
            },
            Target::LogText => {
                // 详情里没按在可点击的条目上：选字（双击选一整行），按在空白处清掉选中。
                let count = self.click_count(x, y);
                if self.press_log_text(x, y, count) {
                    self.drag = Drag::LogText;
                }
            }
            Target::Editor => self.click_editor(x, y),
            Target::Terminal => {
                if self.terminal_view().contains(x, y) {
                    self.terminal_mouse_down(x, y, MouseButton::Left);
                } else {
                    self.focus = Focus::Terminal;
                    self.reset_blink();
                    self.request_redraw();
                }
            }
            Target::TerminalBlock(_) => self.terminal_mouse_down(x, y, MouseButton::Left),
            Target::TerminalHeader(_) if self.field_at(x, y) == Some(Focus::TerminalRename) => {
                self.focus = Focus::TerminalRename;
                self.click_field(x, y);
                self.request_redraw();
            }
            Target::TerminalHeader(_) if self.field_at(x, y) == Some(Focus::TerminalFind) => {
                self.focus = Focus::TerminalFind;
                self.click_field(x, y);
                self.request_redraw();
            }
            Target::TerminalHeader(crate::app::terminal::HeaderHit::Find(part)) => {
                self.press_terminal_find(part);
            }
            // 运行配置的按钮不移动焦点，不打断正在进行的编辑。
            Target::TerminalHeader(crate::app::terminal::HeaderHit::Run(button)) => {
                self.press_run_button(button);
            }
            Target::TerminalHeader(hit) => {
                use crate::app::terminal::HeaderHit;
                self.focus = Focus::Terminal;
                match hit {
                    // 双击标签重命名。
                    HeaderHit::Tab(i) if self.click_count(x, y) == 2 => {
                        self.activate_terminal_tab(i);
                        self.open_terminal_rename(i);
                    }
                    HeaderHit::Tab(i) => self.press_terminal_tab(i, x),
                    HeaderHit::Close(i) => self.close_terminal_tab(i),
                    HeaderHit::New => self.new_terminal_tab(),
                    HeaderHit::ShellMenu => self.open_terminal_shell_menu(),
                    HeaderHit::Restart => self.request_terminal_restart(),
                    HeaderHit::Position => self.toggle_terminal_position(),
                    HeaderHit::Hide => self.click_terminal_button(),
                    HeaderHit::Run(_) | HeaderHit::Find(_) => {}
                }
            }
            // 双击分隔线恢复为左右各占一半，单击按住开始拖动。
            Target::SplitDivider if self.click_count(x, y) >= 2 => {
                self.split_share = None;
                self.relayout();
                self.request_redraw();
            }
            Target::SplitDivider => {
                self.drag = Drag::SplitDivider {
                    start_x: x,
                    start_left: self.layout.editor.w,
                };
            }
            Target::OutlineDivider => {
                let size = self.outline_list().h;
                self.drag = Drag::OutlineDivider { start: y, size };
            }
            Target::TerminalDivider => {
                // 拖动分隔条时取消最大化，从当前大小开始拖。
                self.terminal.maximized = false;
                self.terminal.grow = None;
                let l = self.layout.terminal;
                let (start, size) = if self.terminal_right() {
                    (x, l.w)
                } else {
                    (y, l.h)
                };
                self.drag = Drag::TerminalDivider { start, size };
                self.terminal_dragging(true);
            }
        }
    }

    /// 按下滚动条 `bar`，抓住滑块 `grab` 处开始拖；点在轨道上时滑块立即跳到鼠标处，
    /// 无需等待鼠标移动。
    pub(super) fn grab_scrollbar(&mut self, bar: Scrollbar, grab: f32) {
        self.scroll_glide = None;
        self.drag = Drag::Scrollbar { bar, grab };
        let (x, y) = self.mouse;
        self.on_mouse_move(x, y);
    }

    /// 点在查找条上：点按钮时焦点留在原来的输入框里；点在第二行时编辑替换文字。
    fn click_search(&mut self, x: f32, y: f32) {
        let search = self.layout.search;
        match SearchParts::new(search).button_at(x, y) {
            Some(button) => {
                if !matches!(self.focus, Focus::Search | Focus::Replace) {
                    self.focus = Focus::Search;
                }
                self.press_search_button(button);
            }
            None => {
                self.focus = if y >= search.y + BAR_HEIGHT {
                    Focus::Replace
                } else {
                    Focus::Search
                };
                self.click_field(x, y);
            }
        }
    }

    /// 处理顶栏空白处的点击：拖动窗口；双击时按系统设置缩放或最小化窗口。
    fn click_title_bar(&mut self, x: f32, y: f32) {
        let double = self.title_click.take().is_some_and(|c| {
            c.at.elapsed() < DOUBLE_CLICK && (c.x - x).abs() < 4.0 && (c.y - y).abs() < 4.0
        });

        if double {
            self.title_bar_double_click();
            return;
        }

        self.title_click = Some(Click {
            at: Instant::now(),
            x,
            y,
            count: 1,
        });

        if let Some(window) = &self.window {
            let _ = window.drag_window();
        }
    }

    /// 记录这次点击并返回连击次数：1 为单击，2 为双击，3 为三击，再点击则重新从 1 开始。
    ///
    /// 相邻两次点击的间隔小于 `DOUBLE_CLICK`、位置相差小于 4 逻辑像素时算连击；位置容差为
    /// 经验值，可容纳按下与松开之间鼠标的轻微移动。
    pub(in crate::app) fn click_count(&mut self, x: f32, y: f32) -> u8 {
        let count = match self.last_click {
            Some(c)
                if c.at.elapsed() < DOUBLE_CLICK
                    && (c.x - x).abs() < 4.0
                    && (c.y - y).abs() < 4.0 =>
            {
                c.count % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some(Click {
            at: Instant::now(),
            x,
            y,
            count,
        });
        count
    }
}
