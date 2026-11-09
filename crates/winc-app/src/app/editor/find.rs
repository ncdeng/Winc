//! 文件内查找与替换（查找条），以及跳转到行（顶部输入条）。
//!
//! 查找条的状态由 [`Search`] 保存；本模块负责打开、更新和关闭它，并把当前匹配滚动到可见
//! 区域。跳转条只解析行号和列号。

use winc_i18n::t;

use super::*;

/// 返回输入分支名、标签名的输入框：名字里不能有空白和 `~ ^ : ? * [ \`（git check-ref-format
/// 的规则），这些字符输入不进去。
pub(in crate::app) fn ref_name_field() -> TextField {
    TextField::with_filter(|c| !c.is_whitespace() && !"~^:?*[\\".contains(c))
}

impl App {
    /// 打开查找条。
    ///
    /// `replace` 为 `true` 时同时展开替换；此时如果已有查找内容，焦点直接放到替换输入框。
    /// 没有可显示文本的文档时不打开。
    pub(in crate::app) fn open_search(&mut self, replace: bool) {
        // 差异标签查右侧的文件。
        if self.diff_active() {
            self.in_diff_search(|app| app.open_search(replace));
            return;
        }
        let preview = self.markdown_focused();
        if !self.editor.active().is_some_and(|d| d.edits_text()) && !preview {
            return;
        }
        self.finish_name_input();

        if let Some(seed) = self.search_seed() {
            // 被替换掉的旧查找内容记入历史，按 `↑` 可以找回。
            self.find_input.remember();
            self.find_input.set_text(&seed);
            self.search.query = seed;
        } else {
            // 全选已有的查找内容，后续键入会直接替换该内容。
            self.find_input.select_all();
        }

        self.search_open = true;
        // 预览只读，不能替换。
        self.replace_open = replace && !preview;
        self.focus = if replace && !self.search.query.is_empty() {
            Focus::Replace
        } else {
            Focus::Search
        };
        self.relayout();
        self.search_changed();
        self.reset_blink();
    }

    /// 返回打开查找时预填的内容：选区的文字，没有选区时取光标所在的词；不适合预填时返回 `None`。
    ///
    /// 选区只在单行、少于 200 个字符时采用；没有选区时，词必须只由字母、数字和下划线组成。
    /// 范围达到 1024 个字符及以上时直接放弃。上述上限都是经验值：更长的文字不适合显示在
    /// 单行输入框里，也不像要查找的内容。
    pub(in crate::app) fn search_seed(&self) -> Option<String> {
        if self.markdown_focused() {
            let text = self.markdown_selected_text()?;
            return (text.chars().count() < 200 && !text.contains('\n')).then_some(text);
        }
        let doc = self.editor.active()?;
        let selection = doc.selection;
        let (from, to) = if selection.is_empty() {
            winc_core::movement::word_at(doc.text(), selection.head)
        } else {
            (selection.from(), selection.to())
        };

        if from >= to || to - from >= 1024 {
            return None;
        }

        let text = doc.text_range(from, to)?;
        let usable = if selection.is_empty() {
            text.chars().all(|c| c.is_alphanumeric() || c == '_')
        } else {
            text.chars().count() < 200 && !text.contains(['\n', '\r'])
        };

        usable.then_some(text)
    }

    /// 编辑区中的「查找下一个」（`F3`、`⌘G`），`backwards` 为 `true` 时向前查找。
    ///
    /// 还没有查找内容时，先用光标所在的词（`search_seed`）作为查找内容。
    pub(in crate::app) fn find_again(&mut self, backwards: bool) {
        if self.diff_active() {
            self.in_diff_search(|app| app.find_again(backwards));
            return;
        }
        if self.search.query.is_empty()
            && let Some(seed) = self.search_seed()
        {
            self.find_input.set_text(&seed);
            self.search.query = seed;
        }

        self.find_next(backwards);
    }

    fn remember_search(&mut self) {
        self.find_input.remember();
        self.replace_input.remember();
    }

    /// 关闭查找条，并把焦点还给编辑区。
    pub(in crate::app) fn close_search(&mut self) {
        self.remember_search();
        self.stop_line_filter();
        self.search_open = false;
        self.pending_jump = None;
        self.field_preedit.clear();
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 切换「只显示匹配的行」。
    ///
    /// 开启后，有匹配的行保留，其余行隐藏，便于查看日志或长文件。隐藏的行与折叠的块使用同一套
    /// 显示行，上下移动、滚动、行号都只经过保留的行。查询或选项改变时筛选结果随之更新；
    /// 关闭查找条或再次切换时恢复显示全部行。
    fn toggle_line_filter(&mut self) {
        // 差异两侧按改动对齐，只显示匹配的行没有意义；查的是 Markdown 预览、或者在可编辑的预览
        // 中查原文时也没有「行」。
        if self.diff_active() || self.markdown_focused() || self.markdown_live() {
            return;
        }
        if self.filter_lines {
            self.stop_line_filter();
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        if !doc.can_fold() {
            // 图片和无法打开的文件没有可筛选的行（查找条也无法打开），不提示。
            if doc.shows_text() {
                self.flash(t!("editor-find-large-filter"));
            }
            return;
        }
        self.filter_lines = true;
        self.apply_line_filter();
        self.request_redraw();
    }

    /// 关闭「只显示匹配的行」，恢复显示全部行。
    ///
    /// 光标所在行已经不可见时，滚动到使它位于可见区域顶部起约 1/3 处。
    fn stop_line_filter(&mut self) {
        if !std::mem::take(&mut self.filter_lines) {
            return;
        }
        let top = self
            .editor
            .active()
            .map(|doc| doc.text().line_of(doc.selection.head));
        if let Some(doc) = self.editor.active_mut() {
            doc.filter_lines(None, 0);
        }
        self.filter_key = None;
        self.sticky_cache = None;
        if let Some(line) = top {
            self.scroll_to_line(line, 1.0 / 3.0, true);
        }
        self.request_redraw();
    }

    /// 每帧绘制之前调用：开启「只显示匹配的行」时，按当前搜索结果更新筛选。
    ///
    /// 搜索结果没有变化时不处理。
    pub(in crate::app) fn apply_line_filter(&mut self) {
        if !self.filter_lines {
            return;
        }
        // 切换标签后，其他文档取消筛选，只筛选当前文档。
        let active = self.editor.active;
        for (i, doc) in self.editor.documents.iter_mut().enumerate() {
            if Some(i) != active && doc.folds().is_filtered() {
                doc.filter_lines(None, 0);
            }
        }
        // 无法筛选的文档（大文件）直接返回，避免每搜完一段就把光标滚回可见范围。
        let Some(doc) = self.editor.active().filter(|d| d.can_fold()) else {
            return;
        };
        if !self.search.is_for(doc) {
            return;
        }
        // 查询为空（尚未输入）时显示全部行。
        let shown = (!self.search.query.is_empty()).then(|| self.search.lines());
        let key = (
            doc.id,
            doc.revision,
            self.search.lines().len(),
            self.search.searched_lines(),
            shown.is_some(),
        );
        // 搜索结果没变，且文档仍保持上次筛选后的状态（没有被替换或重新读取）时不处理。
        // 状态要与 `is_filtered` 的实际结果比较，不能只看是否有查询：每一行都匹配时筛选不会
        // 隐藏任何行，若只看是否有查询，每一帧都会重新筛选并把光标滚回可见范围，导致无法滚动。
        if self
            .filter_key
            .is_some_and(|(k, filtered)| k == key && filtered == doc.folds().is_filtered())
        {
            return;
        }
        let searched = self.search.searched_lines();
        let shown = shown.map(<[usize]>::to_vec);
        let mut filtered = false;
        if let Some(doc) = self.editor.active_mut() {
            doc.filter_lines(shown.as_deref(), searched);
            filtered = doc.folds().is_filtered();
        }
        self.filter_key = Some((key, filtered));
        self.sticky_cache = None;
        self.clamp_scroll();
        self.ensure_caret_visible();
    }

    /// 执行查找条上按钮 `button` 的操作；鼠标点击和对应的快捷键都经过这里。
    pub(in crate::app) fn press_search_button(&mut self, button: SearchButton) {
        match button {
            SearchButton::Expand => {
                self.replace_open = !self.replace_open;

                if !self.replace_open && self.focus == Focus::Replace {
                    self.focus = Focus::Search;
                }

                self.relayout();
            }
            SearchButton::Case => self.search.case_sensitive = !self.search.case_sensitive,
            SearchButton::Word => self.search.whole_word = !self.search.whole_word,
            SearchButton::Regex => self.search.regex = !self.search.regex,
            SearchButton::Prev => self.find_next(true),
            SearchButton::Next => self.find_next(false),
            SearchButton::Filter => self.toggle_line_filter(),
            SearchButton::Close => self.close_search(),
            SearchButton::ReplaceOne => self.replace_current(),
            SearchButton::ReplaceAll => self.replace_all(),
        }

        if matches!(
            button,
            SearchButton::Case | SearchButton::Word | SearchButton::Regex
        ) {
            self.search_changed();
        }

        self.request_redraw();
    }

    /// 替换当前选中的匹配，并选中下一个匹配。
    ///
    /// 选区不是匹配时，只选中下一个匹配，再次执行才替换。
    pub(in crate::app) fn replace_current(&mut self) {
        if self.diff_active() {
            self.in_diff_search(App::replace_current);
            return;
        }
        if self.markdown_focused() {
            return;
        }
        self.remember_search();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };

        match self.search.replace_current(doc) {
            Ok(true) => self.search_changed(),
            Ok(false) => self.find_next(false),
            Err(reason) => self.flash(reason),
        }

        self.after_replace();
    }

    /// 替换全部匹配，并在状态栏提示替换的处数。
    pub(in crate::app) fn replace_all(&mut self) {
        if self.diff_active() {
            self.in_diff_search(App::replace_all);
            return;
        }
        if self.markdown_focused() {
            return;
        }
        self.remember_search();

        let Some(doc) = self.editor.active_mut() else {
            return;
        };

        match self.search.replace_all(doc) {
            Ok(0) => self.flash(t!("editor-replace-none")),
            Ok(count) => self.flash(t!("editor-replaced", count = count)),
            Err(reason) => self.flash(reason),
        }

        self.after_replace();
    }

    /// 替换之后重新布局并修正滚动位置：行数可能改变，行号栏宽度随之变化；光标需保持在可见范围内。
    fn after_replace(&mut self) {
        self.relayout();
        self.clamp_scroll();
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 处理查找内容的变化：重新搜索，并选中光标处或其后的第一个结果。
    pub(in crate::app) fn search_changed(&mut self) {
        if self.diff_active() {
            self.in_diff_search(App::search_changed);
            return;
        }
        if self.markdown_focused() {
            if self.markdown_find_doc().is_some()
                && let Some(doc) = &self.markdown.find_doc
            {
                self.search.update(doc);
            }
            let from = self.markdown.selection.map_or(0, |(a, b)| a.min(b));
            self.jump_to_match(from, false);
            self.request_redraw();
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        self.search.update(doc);
        self.jump_to_match(doc.selection.from(), false);
        self.request_redraw();
    }

    /// 选中从 `pos`（字符偏移）起的下一个匹配，`backwards` 为 `true` 时向前找。
    ///
    /// 大文件尚未搜到匹配时，把请求记入 `pending_jump`，搜到之后再跳转。
    pub(in crate::app) fn jump_to_match(&mut self, pos: usize, backwards: bool) {
        self.pending_jump = None;

        let preview = self.markdown_focused();
        match self.search.next_from(pos, backwards) {
            Some((start, end)) if preview => self.select_in_markdown(start, end),
            Some((start, end)) => self.reveal_match(start, end),
            None if !self.search.is_complete() => {
                self.pending_jump = self.search.document().map(|id| (id, pos, backwards));
            }
            None => {}
        }
    }

    /// 选中查找到的匹配 `start..end`，并滚动编辑区使其可见。
    ///
    /// 横向只滚动到匹配刚好露出。纵向在匹配已可见时不滚动；不可见或被右上角的查找条遮挡时，
    /// 将匹配滚到编辑区正中而不是边缘：位于边缘时上下文只显示一两行，向上查找时匹配还可能
    /// 恰好落在查找条下方。
    pub(in crate::app) fn reveal_match(&mut self, start: usize, end: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let before = doc.scroll_line;
        doc.selection = Selection::new(start, end);
        doc.goal_cell = None;
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();

        let visible = self.visible_lines() as f32;
        let (bar, text, lh) = (self.layout.search, self.layout.text, self.fonts.line_height);
        let (tab_width, cell) = (self.editor.active_tab_width(), self.cell);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let (line, _) = doc.text().pos_to_line_col(start);
        let (end_line, end_col) = doc.text().pos_to_line_col(end);
        let right = text.x + TEXT_PAD + doc.col_to_cell(end_line, end_col, tab_width) as f32 * cell
            - doc.scroll_x;
        let row = doc.row_at(start) as f32;
        // 匹配横向位于查找条下方时，查找条遮挡的行同样视为不可见。
        let covered = if bar.h > 0.0 && right > bar.x {
            ((bar.bottom() - text.y) / lh).ceil()
        } else {
            0.0
        };
        doc.scroll_line = before;
        let top = (self.sticky_depth(line) as f32).max(covered).max(1.0);
        let doc = self.editor.active().unwrap();
        let shown = row >= doc.scroll_line + top && row <= doc.scroll_line + visible - 2.0;
        if !shown {
            self.scroll_to_line(line, 0.5, false);
        }
    }

    /// 查找下一个匹配（`backwards` 为 `true` 时查找上一个）；查找内容为空时不处理。
    pub(in crate::app) fn find_next(&mut self, backwards: bool) {
        if self.diff_active() {
            self.in_diff_search(|app| app.find_next(backwards));
            return;
        }
        self.find_input.remember();

        let Some(doc) = self.editor.active() else {
            return;
        };

        if self.search.query.is_empty() {
            return;
        }

        if self.markdown_focused() {
            if self.markdown_find_doc().is_some()
                && let Some(doc) = &self.markdown.find_doc
            {
                self.search.update(doc);
            }
            let (from, to) = self
                .markdown
                .selection
                .map_or((0, 0), |(a, b)| (a.min(b), a.max(b)));
            self.jump_to_match(if backwards { from } else { to }, backwards);
            return;
        }

        self.search.update(doc);
        let pos = if backwards {
            doc.selection.from()
        } else {
            doc.selection.to()
        };
        self.jump_to_match(pos, backwards);
    }

    /// 打开跳转到行的输入条（`⌃G`）；输入条已打开时关闭它，使同一快捷键可以切换开关。
    ///
    /// 没有可显示文本的文档时不处理。
    pub(in crate::app) fn open_goto(&mut self) {
        if self.goto_input.is_some() && self.prompt_kind == PromptKind::GoTo {
            self.close_prompt();
            self.focus = Focus::Editor;
            return;
        }
        if self.editor.active.is_none() || !self.active_shows_text() {
            return;
        }
        self.finish_name_input();
        let field = TextField::with_filter(|c| c.is_ascii_digit() || c == ':');
        self.open_prompt(PromptKind::GoTo, field);
    }

    /// 打开储藏改动的输入条：写说明（可以不写），回车储藏。
    pub(in crate::app) fn open_stash_prompt(&mut self) {
        self.open_prompt(PromptKind::Stash, TextField::default());
    }

    /// 弹出输入条，输入在提交 `at` 上打的标签名（日志里提交的右键菜单）。
    pub(in crate::app) fn open_tag_prompt(&mut self, at: String) {
        self.open_prompt(PromptKind::NewTag { at }, ref_name_field());
    }

    /// 弹出输入条，输入本地分支 `old` 的新名字；输入框里是现在的名字并已全选。
    pub(in crate::app) fn open_rename_prompt(&mut self, old: String) {
        let mut field = ref_name_field();
        field.set_text(&old);
        self.open_prompt(PromptKind::RenameBranch { old }, field);
    }

    /// 打开 `kind` 的输入条，输入框为 `field`，焦点移到输入条上；文件树中正在输入的名字先结束。
    pub(in crate::app) fn open_prompt(&mut self, kind: PromptKind, field: TextField) {
        self.finish_name_input();
        self.goto_input = Some(field);
        self.prompt_kind = kind;
        self.field_preedit.clear();
        self.focus = Focus::Prompt;
        self.relayout();
        self.request_redraw();
    }

    /// 输入条的位置：写 PDF 批注时是会折行的宽输入条（见 [`crate::layout::note_bar`]，行数取上一帧
    /// 折出的行数），其余是跳转条。
    pub(crate) fn prompt_bar(&self) -> Rect {
        let editor = self.layout.editor;
        match (&self.prompt_kind, &self.goto_input) {
            (PromptKind::PdfComment, Some(field)) => {
                let lines = field.view.rows.last().map_or(1, |row| row + 1);
                crate::layout::note_bar(editor, lines)
            }
            _ => crate::layout::goto_bar(editor),
        }
    }

    /// 关闭跳转条，以及文件树中输入名字的那一行。
    pub(in crate::app) fn close_prompt(&mut self) {
        self.goto_input = None;
        self.name_input = None;
        self.field_preedit.clear();
        self.relayout();
        self.request_redraw();
    }

    /// 处理跳转条或文件树输入名字行中的回车。
    pub(in crate::app) fn submit_prompt(&mut self) {
        if self.name_input.is_some() {
            self.submit_name_input();
            return;
        }

        let Some(field) = self.goto_input.take() else {
            return;
        };
        let input = field.text().to_owned();
        self.focus = Focus::Editor;
        match std::mem::replace(&mut self.prompt_kind, PromptKind::GoTo) {
            PromptKind::GoTo => self.goto_line(&input),
            PromptKind::Stash => self.git_stash(&input),
            PromptKind::NewTag { at } => self.git_new_tag(&input, at),
            PromptKind::RenameBranch { old } => self.git_rename_branch(old, &input),
            PromptKind::PdfComment => self.pdf_submit_comment(&input),
        }
        self.relayout();
        self.request_redraw();
    }

    /// 跳到 `input` 指定的「行:列」（从 1 起），并把目标行滚动到可见区域顶部起约 1/3 处。
    ///
    /// 无法解析的部分按 1 处理；省略列时跳到行首。
    pub(in crate::app) fn goto_line(&mut self, input: &str) {
        let mut parts = input
            .split(':')
            .map(|s| s.trim().parse::<usize>().unwrap_or(1).max(1) - 1);
        let line = parts.next().unwrap_or(0);
        let col = parts.next().unwrap_or(0);
        self.go_to(line, col, 0);
    }

    /// 选中第 `line` 行第 `col` 列起的 `len` 个字符（行号、列号都从 0 起），并把该行滚动到可见
    /// 区域顶部起约 1/3 处。
    ///
    /// 先滚动（展开所在的折叠）再选中：选中时只需横向滚动，使选区的末端可见。
    pub(in crate::app) fn go_to(&mut self, line: usize, col: usize, len: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let start = doc.text().line_col_to_pos(line, col);
        self.scroll_to_line(line, 1.0 / 3.0, false);
        self.select(start, start + len);
    }
}
