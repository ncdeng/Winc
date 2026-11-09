//! 鼠标下面是什么：点击和悬停共用的目标判断、悬停的高亮、光标形状。

use super::*;

impl App {
    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处的悬停目标。
    pub(in crate::app) fn hover_at(&self, x: f32, y: f32) -> Hover {
        let l = &self.layout;
        // 窗口内的菜单盖在上面：下面的部件都不算悬停。
        if self.popup().is_some() {
            return Hover::None;
        }

        // 拖动滚动条时鼠标会经过文件树的行和编辑区，悬停保持在这根滚动条上，下方的内容不高亮。
        // 拖动文件树中的项或标签时没有悬停目标；文件树中高亮的是放置目标目录，而不是鼠标所在的行。
        match self.drag {
            Drag::Scrollbar { bar, .. } => return Hover::Scrollbar(bar),
            _ if self.tree_dragging().is_some() || self.dragged_tab().is_some() => {
                return Hover::None;
            }
            _ => {}
        }

        match self.target_at(x, y) {
            Target::WindowControl(control) => Hover::WindowControl(control),
            Target::ResizeEdge(direction) => Hover::ResizeEdge(direction),
            Target::NoticeButton => Hover::NoticeButton,
            Target::Change(target) => Hover::Change(target),
            Target::Merge(target) => Hover::Merge(target),
            Target::Conflict(action) => Hover::ConflictButton(action),
            Target::Picker
                if let Some(&(_, part)) =
                    (self.picker_part_hits.iter()).find(|(rect, _)| rect.contains(x, y)) =>
            {
                Hover::PickerPart(part)
            }
            Target::Picker => view::picker_parts(self, l.picker)
                .row_at(y, self.picker_scroll)
                .filter(|&i| i < self.picker_hits.len())
                .map_or(Hover::None, Hover::PickerRow),
            Target::TreeMore => Hover::TreeMore,
            Target::Scrollbar(bar, _) => Hover::Scrollbar(bar),
            Target::Minimap => Hover::Minimap,
            Target::Panel(hover) => hover,
            Target::Search => SearchParts::new(l.search)
                .button_at(x, y)
                .map_or(Hover::None, Hover::Search),
            Target::Splitter => Hover::Splitter,
            Target::TreeTool(tool) => Hover::TreeTool(tool),
            Target::ProjectName => Hover::ProjectName,
            Target::SidebarButton => Hover::SidebarButton,
            Target::NewTab => Hover::NewTab,
            Target::TabList => Hover::TabList,
            Target::Preview(mode) => Hover::Preview(mode),
            Target::Browser => Hover::Browser,
            Target::MarkdownLink(link) => Hover::MarkdownLink(link),
            Target::MarkdownToc(target) => Hover::MarkdownToc(target),
            Target::MarkdownText => Hover::None,
            Target::PdfText => self
                .pdf
                .note_at(x, y)
                .filter(|&(page, i)| {
                    let note = self.pdf.texts.get(&page).and_then(|t| t.notes.get(i));
                    note.is_some_and(|n| !n.comment.trim().is_empty())
                })
                .map_or(Hover::None, |(page, i)| Hover::PdfNote(page, i)),
            Target::Tab(index, on_close) => Hover::Tab(index, on_close),
            Target::RulerChange(hunk) => Hover::RulerChange(hunk),
            Target::Crumb(index) => Hover::Crumb(index),
            Target::Status(item) => Hover::Status(item),
            Target::Tree(row) => row.map_or(Hover::None, Hover::TreeRow),
            Target::Placeholder(action) => Hover::Placeholder(action),
            Target::Diff(target) => Hover::Diff(target),
            Target::Log(target) => Hover::Log(target),
            Target::Start(action) => Hover::Start(action),
            Target::Sticky(row) => Hover::Sticky(row.depth),
            Target::Folded(line) => Hover::Folded(line),
            Target::Blame(line) => Hover::Blame(line),
            Target::ChangeMark(hunk) => Hover::ChangeMark(hunk),
            Target::Gutter => Hover::Gutter,
            Target::TerminalDivider => Hover::TerminalDivider,
            Target::OutlineDivider => Hover::OutlineDivider,
            Target::SplitDivider => Hover::SplitDivider,
            Target::TerminalHeader(hit) => Hover::TerminalHeader(hit),
            Target::TerminalBlock(hit) => Hover::TerminalBlock(hit),
            // 挡在上面、没有按钮的地方，和没有悬停效果的部件：输入条、顶栏空白、编辑区文字。
            Target::Blocked
            | Target::Prompt
            | Target::TitleBar
            | Target::LogText
            | Target::Editor
            | Target::Terminal
            | Target::None => Hover::None,
        }
    }

    /// 窗口坐标 (`x`, `y`)（逻辑像素）处最上面的可交互目标。
    ///
    /// 悬停（[`App::hover_at`]）和左键按下（[`App::on_mouse_down`]）都按它判断，判断顺序
    /// （与绘制层次相反，画在上面的先认）只写在这一处，新加一个部件只需在这里放对位置。
    ///
    /// 只看位置，不看按下了什么键；设置浮层、右键、中键这些在调用方先处理。
    pub(super) fn target_at(&self, x: f32, y: f32) -> Target {
        let l = &self.layout;
        // 窗口最外圈的细边改变窗口大小，压在一切之上；窗口按钮浮在顶栏右端。
        if let Some(direction) = self.resize_edge_at(x, y) {
            return Target::ResizeEdge(direction);
        }
        if let Some(control) = self.window_control_at(x, y) {
            return Target::WindowControl(control);
        }
        // 浮在编辑区之上的卡片和提示条：只认上面的按钮，卡片其余地方挡住下面。
        if let Some(card) = self.changes.card
            && card.contains(x, y)
        {
            return hit(&self.changes.hits, x, y).map_or(Target::Blocked, Target::Change);
        }
        if self.notice_button.is_some_and(|b| b.contains(x, y)) {
            return Target::NoticeButton;
        }
        if self.notice_card.is_some_and(|card| card.contains(x, y)) {
            return Target::Blocked;
        }
        // 合并冲突的按钮在卡片上，也在标记行上。
        if let Some(target) = hit(&self.merge.hits, x, y) {
            return Target::Merge(target);
        }
        if self.merge.card.is_some_and(|card| card.contains(x, y)) {
            return Target::Blocked;
        }
        if let Some((card, buttons)) = &self.conflict_card
            && card.contains(x, y)
        {
            return buttons
                .iter()
                .find(|(rect, _)| rect.contains(x, y))
                .map_or(Target::Blocked, |&(_, action)| Target::Conflict(action));
        }
        // 快速打开面板浮在编辑区和侧栏之上。
        if self.picker_open && l.picker.contains(x, y) {
            return Target::Picker;
        }
        if self.tree_more.is_some_and(|more| more.contains(x, y)) {
            return Target::TreeMore;
        }
        // 搜索结果、改动列表的滚动条位于行的右端，先于行判断。显示全名的那一改动行覆盖在滚动条
        // 之上：鼠标移到这一行的右端时全名保持显示，按下作用于这一行。
        let over_commit_row = self.over_commit_overflow_row(y);
        if let Some((bar @ (Scrollbar::Panel | Scrollbar::Commit | Scrollbar::Outline), grab)) =
            self.scrollbar_at(x, y)
            && !(over_commit_row && bar == Scrollbar::Commit)
        {
            return Target::Scrollbar(bar, grab);
        }
        // 大纲标题行的上沿跨在文件树和大纲之间，先于两者判定。
        if self.outline_divider().contains(x, y) {
            return Target::OutlineDivider;
        }
        if let Some(hover) = self.panel_hover_at(x, y) {
            return Target::Panel(hover);
        }
        // 跳转条画在查找条上面（窄窗口里两者会重叠）；正在输入名字的那一行在文件树里。
        if (self.goto_input.is_some() && goto_bar(l.editor).contains(x, y))
            || self.name_input_rect().is_some_and(|r| r.contains(x, y))
        {
            return Target::Prompt;
        }
        if l.search.contains(x, y) {
            return Target::Search;
        }
        // 名称放不下的一行（文件树、改动列表）覆盖在分隔条上：这一行优先，其余各行仍然可以拖动
        // 分隔条。
        let over_row = self.over_tree_overflow_row(y);
        if self.panel_visible && !over_row && !over_commit_row && l.splitter.contains(x, y) {
            return Target::Splitter;
        }
        if let Some(tool) = hit(&self.tree_tools, x, y) {
            return Target::TreeTool(tool);
        }
        if self.project_hit.contains(x, y) {
            return Target::ProjectName;
        }
        if l.sidebar_button.contains(x, y) {
            return Target::SidebarButton;
        }
        if self.tabs.new_tab.contains(x, y) {
            return Target::NewTab;
        }
        if self.tabs.list.is_some_and(|r| r.contains(x, y)) {
            return Target::TabList;
        }
        // 切换到不可预览的文件后，三段切换收起的动画过程中不响应点击。
        if l.preview.contains(x, y) && self.editor.active().is_some_and(|d| d.can_preview()) {
            use winc_editor::PreviewMode::*;
            let segments = crate::layout::preview_segments(l.preview);
            // 按钮之间的空隙归离得近的一段，否则点在空隙里不会响应。
            let gap = |s: &winc_render::Rect| (s.x - x).max(x - s.right()).max(0.0);
            let at = (0..3)
                .min_by(|&a, &b| gap(&segments[a]).total_cmp(&gap(&segments[b])))
                .unwrap_or(0);
            return Target::Preview([Source, Split, Preview][at]);
        }
        // 换到不是网页的文件后，按钮收起的动画过程中不响应点击。
        if l.browser.contains(x, y) && self.browser_page().is_some() {
            return Target::Browser;
        }
        if let Some((index, on_close)) = self.tab_at(x, y) {
            return Target::Tab(index, on_close);
        }
        // 概览标尺上 Git 改动那一道在滑块左边，先于滚动条认。滚动条叠在文件树最下面一行和
        // 编辑区右侧文字之上：这些区域内按下拖的是滚动条。
        if let Some((hunk, _)) = self.ruler_change_at(x, y) {
            return Target::RulerChange(hunk);
        }
        // 名称放不下的一行同样盖在文件树的滚动条上，不会一进滚动条就收起。
        if let Some((bar, grab)) = self.scrollbar_at(x, y).filter(|(bar, _)| {
            !(over_row && *bar == Scrollbar::Tree || over_commit_row && *bar == Scrollbar::Commit)
        }) {
            return Target::Scrollbar(bar, grab);
        }
        if l.minimap.contains(x, y) {
            return Target::Minimap;
        }
        if let Some(index) = hit(&self.crumb_hits, x, y) {
            return Target::Crumb(index);
        }
        if let Some(item) = hit(&self.status_hits, x, y) {
            return Target::Status(item);
        }
        // 终端面板在右边时，标签栏和分隔间距落在顶栏那一行里，要先于「按下拖窗口」判定。
        if l.terminal_divider.contains(x, y) {
            return Target::TerminalDivider;
        }
        if l.split_divider.contains(x, y) {
            return Target::SplitDivider;
        }
        if let Some(hit) = hit(&self.terminal.hits, x, y) {
            return Target::TerminalHeader(hit);
        }
        if y < TOP_BAR_END {
            return Target::TitleBar;
        }
        if l.terminal.contains(x, y) {
            return match self.terminal_block_hit(x, y) {
                Some(hit) => Target::TerminalBlock(hit),
                None => Target::Terminal,
            };
        }
        if l.tree.contains(x, y) || self.in_tree_overflow(x, y) {
            return Target::Tree(self.tree_row_at(y));
        }
        if let Some(action) = hit(&self.placeholder_hits, x, y) {
            return Target::Placeholder(action);
        }
        if let Some(target) = hit(&self.diff_hits, x, y) {
            return Target::Diff(target);
        }
        if let Some(target) = hit(&self.log.hits, x, y) {
            return Target::Log(target);
        }
        if let Some(action) = hit(&self.start_hits, x, y) {
            return Target::Start(action);
        }
        if let Some(row) = self.sticky_at(x, y) {
            return Target::Sticky(row);
        }
        if let Some(line) = self.folded_at(x, y) {
            return Target::Folded(line);
        }
        if let Some(line) = self.blame_at(x, y) {
            return Target::Blame(line);
        }
        if let Some(hunk) = self.change_mark_at(x, y) {
            return Target::ChangeMark(hunk);
        }
        if self.editor.active.is_some()
            && self.active_shows_text()
            && l.gutter.contains(x, y)
            && y >= l.text.y
        {
            return Target::Gutter;
        }
        if self.log_active() && self.log_parts().details.contains(x, y) {
            return Target::LogText;
        }
        // 标题指示条和展开的标题列表盖在预览的文字上面，先于文字判定。
        if self.markdown.area.contains(x, y)
            && let Some(target) = hit(&self.markdown.toc, x, y)
        {
            return Target::MarkdownToc(target);
        }
        // 可编辑的预览中单击文字用于放置光标，便于编辑链接文本；按住 `⌘`（其他平台为 `Ctrl`）
        // 点击才打开链接。复制按钮、折叠块的标题仍然单击生效。
        if let Some(link) = self.markdown_link_at(x, y)
            && (!self.markdown_live()
                || self.markdown_link_is_control(link)
                || if cfg!(target_os = "macos") {
                    self.modifiers.super_key()
                } else {
                    self.modifiers.control_key()
                })
        {
            return Target::MarkdownLink(link);
        }
        // 预览上按下：选字，并拿到焦点（便于用键盘滚动和复制）。
        if self.markdown.area.contains(x, y) {
            return Target::MarkdownText;
        }
        // PDF 上按下：选字，并拿到焦点（便于用键盘滚动和复制）。
        if self.pdf.area.contains(x, y) {
            return Target::PdfText;
        }
        if l.editor.contains(x, y) && self.editor.active.is_some() {
            return Target::Editor;
        }
        Target::None
    }

    /// 返回鼠标所在的可点击滚动条，以及此时按下后抓住滑块的位置。
    ///
    /// 抓取位置是到滑块起点的距离（逻辑像素）；点在滑块之外时抓住滑块中点，滑块会跳到鼠标处。
    ///
    /// 滚动条贴着区域的右边和下边，可点击的范围是从该边向内 `SCROLLBAR_HIT`，比绘制出来的滑块更宽。
    /// 只在区域内判断，区域外的标签、状态栏不受影响。内容放得下、没有滑块时不返回。
    fn scrollbar_at(&self, x: f32, y: f32) -> Option<(Scrollbar, f32)> {
        let grab = |pos: f32, start: f32, len: f32| {
            if (start..start + len).contains(&pos) {
                pos - start
            } else {
                len / 2.0
            }
        };

        // 搜索视图的结果列表（和文件树在侧栏里的同一块地方，先认它）。
        if self.panel_shown() {
            let results = self.panel_parts().results;
            if results.contains(x, y) && x >= results.right() - SCROLLBAR_HIT {
                let content = (self.global.rows().len() as f32 + 1.0) * TREE_ROW;
                let thumb = scrollbar_thumb(results, self.global.scroll, content)?;
                return Some((Scrollbar::Panel, grab(y, thumb.y, thumb.h)));
            }
        }
        // 大纲的符号列表。
        if self.outline_shown() {
            let list = self.outline_list();
            if list.contains(x, y) && x >= list.right() - SCROLLBAR_HIT {
                let content = self.outline_max_scroll() + list.h;
                let thumb = scrollbar_thumb(list, self.outline.scroll, content)?;
                return Some((Scrollbar::Outline, grab(y, thumb.y, thumb.h)));
            }
        }
        // 提交视图的改动列表。
        if self.commit_shown() {
            let list = self.commit_parts().list;
            if list.contains(x, y) && x >= list.right() - SCROLLBAR_HIT {
                let content = self.commit_max_scroll() + list.h;
                let thumb = scrollbar_thumb(list, self.commit.scroll, content)?;
                return Some((Scrollbar::Commit, grab(y, thumb.y, thumb.h)));
            }
        }

        let tree = self.layout.tree;
        if self.panel_visible && tree.contains(x, y) {
            let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
            let content = (rows as f32 + 1.0) * TREE_ROW;

            if x >= tree.right() - SCROLLBAR_HIT
                && let Some(thumb) = scrollbar_thumb(tree, self.tree_scroll, content)
            {
                return Some((Scrollbar::Tree, grab(y, thumb.y, thumb.h)));
            }
            if y >= tree.bottom() - SCROLLBAR_HIT
                && let Some(thumb) = hscrollbar_thumb(tree, self.tree_scroll_x, self.tree_content_w)
            {
                return Some((Scrollbar::TreeHorizontal, grab(x, thumb.x, thumb.w)));
            }
            return None;
        }

        if let Some((track, scroll, content)) = self.terminal_scrollbar()
            && track.contains(x, y)
            && x >= track.right() - SCROLLBAR_HIT
        {
            let thumb = scrollbar_thumb(track, scroll, content)?;
            return Some((Scrollbar::Terminal, grab(y, thumb.y, thumb.h)));
        }

        // 预览（分栏时在右边，只看预览时占满编辑区）：先于编辑区认，只看预览时两者重叠。
        let preview = self.markdown.area;
        if preview.contains(x, y) {
            if x < preview.right() - SCROLLBAR_HIT {
                return None;
            }
            let scroll = self.markdown.scroll();
            let thumb = scrollbar_thumb(preview, scroll, self.markdown.height)?;
            return Some((Scrollbar::Markdown, grab(y, thumb.y, thumb.h)));
        }

        let editor = self.scroll_track();
        let doc = self.editor.active()?;
        if editor.contains(x, y) && x >= editor.right() - SCROLLBAR_HIT {
            let scroll = doc.scroll_line * self.fonts.line_height;
            let thumb = scrollbar_thumb(editor, scroll, self.content_height())?;
            return Some((Scrollbar::Editor, grab(y, thumb.y, thumb.h)));
        }

        let track = self.text_hscroll_track();
        if track.contains(x, y)
            && y >= track.bottom() - SCROLLBAR_HIT
            && let Some(thumb) = hscrollbar_thumb(track, doc.scroll_x, self.text_content_w)
        {
            return Some((Scrollbar::EditorHorizontal, grab(x, thumb.x, thumb.w)));
        }

        None
    }

    /// 滚动条滑块是否应绘制为加深的样子：鼠标在滑块上，或正在拖动滑块。
    pub(crate) fn scrollbar_active(&self, bar: Scrollbar) -> bool {
        self.hover == Hover::Scrollbar(bar)
            || matches!(self.drag, Drag::Scrollbar { bar: dragged, .. } if dragged == bar)
    }

    /// 窗口坐标 (`x`, `y`)（逻辑像素）是否位于文件树中放不下的文件名向右展开、覆盖到编辑区上方
    /// 的那一段区域内。
    /// 纵坐标 `y` 是否在正在显示完整名称的那一行上：这一行在最上层，盖住分隔条和文件树的滚动条。
    ///
    /// 只有已经展开的一行优先。沿着边缘上下移动到别的行时，那一行还没有展开，滚动条照常响应。
    pub(in crate::app) fn over_tree_overflow_row(&self, y: f32) -> bool {
        self.tree_overflow
            .is_some_and(|(rect, _)| (rect.y..rect.bottom()).contains(&y))
    }

    pub(in crate::app) fn in_tree_overflow(&self, x: f32, y: f32) -> bool {
        self.tree_overflow
            .is_some_and(|(rect, _)| rect.contains(x, y))
    }

    /// 按鼠标现在所在的位置重新判断悬停目标和指针样式；每画完一帧调用一次。
    ///
    /// 悬停平时只在鼠标移动时判断。按键改变了界面（`Esc` 关掉面板、提示条到时消失、打字使文字
    /// 滚动）而鼠标没动时，原来的悬停目标可能已经不在了，或者鼠标下面换成了另一个部件：不重新
    /// 判断时，下次打开面板时还会高亮上次那一行。判断依据的是刚画完这一帧记下的各个矩形；
    /// 悬停变了会再画一帧，之后就稳定了。
    pub(in crate::app) fn refresh_hover(&mut self) {
        if !self.cursor_inside || self.settings.open || self.form_dialog.is_some() {
            return;
        }
        let (x, y) = self.mouse;
        let hover = self.hover_at(x, y);
        if hover != self.hover {
            self.set_hover(hover);
            self.update_cursor_icon();
        }
        self.track_tree_segment();
    }

    /// `(x, y)` 是否在编辑区右上角的卡片（冲突卡片、提示条）上：卡片浮在文字上面，点击、
    /// 右键、指针样式都不该落到下面的文字上。
    pub(in crate::app) fn over_corner_card(&self, x: f32, y: f32) -> bool {
        self.notice_card.is_some_and(|card| card.contains(x, y))
            || self.merge.card.is_some_and(|card| card.contains(x, y))
            || self
                .conflict_card
                .as_ref()
                .is_some_and(|(card, _)| card.contains(x, y))
    }

    /// 根据鼠标位置和拖动状态更新鼠标指针的样式。
    pub(in crate::app) fn update_cursor_icon(&mut self) {
        let (x, y) = self.mouse;
        // 新建工作树对话框打开时只看对话框自身：名称输入框上显示文本光标（I 形），
        // 其余位置显示箭头。
        if self.form_dialog.is_some() {
            let icon = if self.dialog_part_at(x, y) == Some(crate::layout::DialogPart::Branch) {
                CursorIcon::Text
            } else {
                CursorIcon::Default
            };
            self.set_cursor_icon(icon);
            return;
        }
        // 全屏查看的图上：工具栏按钮处显示手形指针，其余位置显示表示可拖动的抓手指针。
        if let Some(view) = &self.markdown.diagram
            && (view.area.contains(x, y) || view.grab.is_some())
            && !self.settings.open
            && self.form_dialog.is_none()
        {
            let icon = if view.grab.is_some() {
                CursorIcon::Grabbing
            } else if view.hover.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Grab
            };
            self.set_cursor_icon(icon);
            return;
        }
        if self.settings.open {
            // 设置浮层打开时只看浮层自身：字体名输入框、搜索框和正在编辑的输入框上显示文本光标，
            // 链接上显示手形，其余位置显示箭头。
            let editing = self.settings.editing.as_ref().map(|(s, _)| *s);
            let icon = match self.settings_part_at(x, y) {
                Some(SettingsPart::Field(s))
                    if s.control() == SettingControl::Font || editing == Some(s) =>
                {
                    CursorIcon::Text
                }
                Some(SettingsPart::KeySearch | SettingsPart::SearchField) => CursorIcon::Text,
                Some(SettingsPart::Link(_)) => CursorIcon::Pointer,
                _ => CursorIcon::Default,
            };
            self.set_cursor_icon(icon);
            return;
        }

        let in_text = self.editor.active.is_some()
            && self.active_shows_text()
            && !matches!(
                self.hover,
                Hover::TreeRow(_) | Hover::Sticky(_) | Hover::Folded(_)
            )
            && self.layout.text.contains(x, y)
            && !self.layout.search.contains(x, y)
            && !(self.picker_open && self.layout.picker.contains(x, y))
            && !self.over_corner_card(x, y)
            && x < self.layout.editor.right() - SCROLLBAR_HIT
            || self.over_diff_text(x, y) && x < self.layout.editor.right() - SCROLLBAR_HIT;
        // 日志详情里能选的字上（不是能点的提交号、文件）：文字光标。
        let in_field = self.field_at(x, y).is_some()
            || matches!(
                self.drag,
                Drag::LogText | Drag::MarkdownText | Drag::PdfText
            )
            || (self.markdown.area.contains(x, y) && self.markdown_link_at(x, y).is_none())
            || self.pdf.text_at(x, y)
            || (self.log_active()
                && crate::layout::hit(&self.log.hits, x, y).is_none()
                && self.log_text_at(x, y, false).is_some());

        let icon = if let Hover::ResizeEdge(direction) = self.hover {
            resize_cursor(direction)
        } else if let Some(drag) = self.tree_dragging() {
            // 鼠标在文件树内但放置没有效果（目标在被拖项自身之内，或被拖项已在该目录中）时显示
            // 禁止图标；在文件树之外松开则不移动，显示普通箭头。
            if drag.target.is_none() && self.layout.tree.contains(x, y) {
                CursorIcon::NotAllowed
            } else {
                CursorIcon::Default
            }
        } else if matches!(self.drag, Drag::OutlineDivider { .. })
            || self.hover == Hover::OutlineDivider
        {
            CursorIcon::RowResize
        } else if matches!(self.drag, Drag::TerminalDivider { .. })
            || self.hover == Hover::TerminalDivider
        {
            // 面板在下面时上下拖，在右边时左右拖。
            if self.terminal_right() {
                CursorIcon::ColResize
            } else {
                CursorIcon::RowResize
            }
        } else if matches!(
            self.drag,
            Drag::Splitter { .. } | Drag::LogDivider { .. } | Drag::SplitDivider { .. }
        ) || matches!(
            self.hover,
            Hover::Splitter
                | Hover::SplitDivider
                | Hover::Log(crate::app::git::log::LogTarget::Divider)
        ) {
            CursorIcon::ColResize
        } else if matches!(
            self.hover,
            Hover::Log(crate::app::git::log::LogTarget::Line(..))
                | Hover::TerminalBlock(_)
                | Hover::MarkdownLink(_)
                | Hover::MarkdownToc(crate::app::preview::markdown::TocTarget::Heading(_))
        ) || self.terminal.link.is_some()
        {
            // 提交图里的线可以点（沿着它跳过去）；终端里命令块的色带、吸顶的命令行也是。
            CursorIcon::Pointer
        } else if in_text || in_field {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };

        self.set_cursor_icon(icon);
    }

    fn set_cursor_icon(&mut self, icon: CursorIcon) {
        if icon != self.cursor_icon {
            self.cursor_icon = icon;

            if let Some(window) = &self.window {
                window.set_cursor(icon);
            }
        }
    }
}

/// 窗口里某一点上最上面的可交互目标，见 [`App::target_at`]。
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    /// 挡在上面、但这里没有按钮：提示条、卡片的空白处。
    ///
    /// 点了什么都不做，也不落到下面。
    Blocked,
    NoticeButton,
    Change(crate::app::ChangeTarget),
    Merge(crate::app::git::merge::MergeTarget),
    Conflict(crate::app::editor::conflict::ConflictAction),
    /// 快速打开面板里。
    Picker,
    TreeMore,
    /// 滚动条，以及按下时抓住滑块的位置。
    Scrollbar(Scrollbar, f32),
    /// 侧栏里视图切换、项目名、分支和搜索、提交面板管的地方，带上悬停目标。
    Panel(Hover),
    /// 跳转条、正在输入名字的那一行。
    Prompt,
    Search,
    Splitter,
    TreeTool(crate::layout::TreeTool),
    ProjectName,
    SidebarButton,
    NewTab,
    /// 标签栏无法容纳全部标签时显示在最右端的「⌄」。
    TabList,
    /// 标签栏右端「源代码 / 分栏 / 预览」三段切换里的一个。
    Preview(winc_editor::PreviewMode),
    /// 标签栏右端「在浏览器中打开」。
    Browser,
    /// Markdown 预览里的链接，参数为它在本帧 `links` 里的下标。
    MarkdownLink(usize),
    MarkdownToc(crate::app::preview::markdown::TocTarget),
    /// Markdown 预览上不是链接的地方：选字。
    MarkdownText,
    /// PDF 预览：选字；悬停在带注释的批注上时显示注释。
    PdfText,
    /// 标签，参数为标签下标和在关闭按钮上。
    Tab(usize, bool),
    /// 概览标尺上的改动，参数为改动块的序号。
    RulerChange(usize),
    /// 状态栏路径中的一段，参数为段号（从 0 起）。
    Crumb(usize),
    /// 状态栏右侧的一项。
    Status(crate::app::StatusItem),
    /// 顶栏空白处：按下拖窗口。
    TitleBar,
    /// 应用绘制的标题栏上的窗口按钮。
    WindowControl(WindowControl),
    /// 可以拖动改变窗口大小的边缘，参数是方向。
    ResizeEdge(winit::window::ResizeDirection),
    /// 文件树（含伸出去写全名的那一段），以及鼠标所在的行。
    Tree(Option<usize>),
    Placeholder(crate::app::PlaceholderAction),
    Diff(crate::app::DiffTarget),
    Log(crate::app::git::log::LogTarget),
    Start(crate::app::StartAction),
    Sticky(crate::app::editor::sticky::StickyRow),
    /// 折起的地方，参数为行号（从 0 起）。
    Folded(usize),
    /// 逐行追溯的标注，参数为行号（从 0 起）。
    Blame(usize),
    /// 行号栏上标出的改动，参数为改动块的序号。
    ChangeMark(usize),
    /// 行号栏（折叠箭头在它里面）。
    Gutter,
    /// 日志详情里的文字。
    LogText,
    Editor,
    /// 终端面板。
    Terminal,
    /// 编辑区和终端面板之间的间距。
    TerminalDivider,
    /// 文件视图里大纲标题行的上沿。
    OutlineDivider,
    /// 分栏预览中间的分隔线。
    SplitDivider,
    /// 编辑区右侧的缩略图。
    Minimap,
    /// 终端面板标签栏上的按钮。
    TerminalHeader(crate::app::terminal::HeaderHit),
    /// 终端里命令块的色带、吸顶的命令行。
    TerminalBlock(crate::app::terminal::BlockHit),
    None,
}
