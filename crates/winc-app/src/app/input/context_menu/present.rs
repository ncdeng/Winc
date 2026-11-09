//! 弹出菜单：右键点中的是什么、各项的文字和勾选、交给系统菜单显示。

use winc_i18n::t;

use super::*;

/// 菜单项上显示的文字：标题、标题下方的一行小字，以及悬停提示。
#[derive(Debug, Clone, PartialEq)]
pub struct MenuLabel {
    /// 标题。
    pub title: String,
    /// 标题下方的小字。
    pub subtitle: Option<String>,
    /// 悬停提示。
    pub tooltip: Option<String>,
}

impl MenuLabel {
    /// 只有标题的文字。
    pub(crate) fn plain(title: &str) -> MenuLabel {
        MenuLabel {
            title: title.to_owned(),
            subtitle: None,
            tooltip: None,
        }
    }
}

/// 菜单中一项需要的数据，由 `Prepared` 持有，弹出菜单时借给系统菜单。
#[cfg(target_os = "macos")]
struct Prepared {
    label: MenuLabel,
    /// 快捷键的按键字符和修饰键位。
    key: (String, usize),
    icon: Option<Image>,
    /// 图标的逻辑尺寸（宽，高）。
    icon_size: (f32, f32),
    enabled: bool,
    checked: bool,
    /// 按住 `⌥` 时代替前一项（[`MenuEntry::Alternate`]）。
    alternate: bool,
    tag: isize,
    children: Vec<Option<Prepared>>,
}

#[cfg(target_os = "macos")]
impl Prepared {
    fn item(&self) -> crate::macos::MenuItem<'_> {
        crate::macos::MenuItem {
            title: &self.label.title,
            subtitle: self.label.subtitle.as_deref(),
            tooltip: self.label.tooltip.as_deref(),
            icon: self.icon.as_ref().map(|image| crate::macos::MenuIcon {
                image,
                size: self.icon_size,
            }),
            key: &self.key.0,
            modifiers: self.key.1,
            enabled: self.enabled,
            checked: self.checked,
            alternate: self.alternate,
            tag: self.tag,
            submenu: self
                .children
                .iter()
                .map(|c| c.as_ref().map(Prepared::item))
                .collect(),
        }
    }
}

/// 标签列表菜单的图标边长（逻辑像素）。
///
/// 文件图标为 12 × 15，无法放入其他菜单 15 × 12 的图标位，因此改用 16 × 16 的图标位，
/// 状态栏路径的目录菜单也使用该尺寸。
const TAB_LIST_ICON: f32 = 16.0;

/// 菜单项左边的图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuIcon {
    /// 最近打开的目录：与文件树中的目录相同的图标和颜色。
    Folder,
    /// 标签列表的第几项：标签上的图标（文件类型图标，日志标签为 Git 图标）。
    Tab(usize),
    /// 目录菜单中的文件：文件类型的记号和它在深色、浅色配色下的颜色（同文件树）。
    File(icons::FileGlyph, [u32; 2]),
}

impl MenuIcon {
    /// 图标位的逻辑尺寸（宽，高）。
    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            MenuIcon::Tab(_) | MenuIcon::File(..) => (TAB_LIST_ICON, TAB_LIST_ICON),
            MenuIcon::Folder => MENU_ICON,
        }
    }
}

/// 命令 `command` 的菜单项图标的逻辑尺寸（宽，高）。
#[cfg(target_os = "macos")]
fn menu_icon_size(command: MenuCommand) -> (f32, f32) {
    match command {
        MenuCommand::ShowTab(_) => (TAB_LIST_ICON, TAB_LIST_ICON),
        _ => MENU_ICON,
    }
}

impl App {
    /// 处理右键点击（macOS 上也包括 `⌃` 单击）：点在标签上弹出标签菜单，点在文件树里弹出
    /// 文件树菜单，点在编辑区里弹出编辑区菜单。
    ///
    /// 文件树中点中的那一行先被选中，点在空白处表示根目录。其他位置没有菜单。设置浮层或
    /// 快速打开面板打开时不处理。
    pub(in crate::app) fn context_click(&mut self, x: f32, y: f32) {
        if self.settings.open || self.form_dialog.is_some() || self.picker_open {
            return;
        }

        // 正在输入名字时点击了右键：先提交名字，与点击其他位置时相同。
        if self.name_input.is_some() && !self.name_input_rect().is_some_and(|r| r.contains(x, y)) {
            self.submit_name_input();
        }

        let tab = self
            .tab_at(x, y)
            .and_then(|(index, _)| self.editor.documents.get(index));
        let target = if let Some(doc) = tab {
            MenuTarget::Tab(doc.id)
        } else if self.panel_visible
            && self.sidebar_view == SidebarView::Files
            && (self.layout.tree.contains(x, y) || self.in_tree_overflow(x, y))
            && self.editor.workspace.is_some()
        {
            let row = self.tree_row_at(y);
            let segment = row.and_then(|i| self.tree_segment_at(i, x));
            let Some(ws) = &mut self.editor.workspace else {
                return;
            };
            // 点在选区里的一行上时菜单作用于选中的各项，否则只选这一行。
            if !row.is_some_and(|i| ws.is_selected(i)) {
                ws.select(row);
            }
            // 点在合并显示的一行上时，菜单作用于点中的那一级目录。
            if let Some(i) = row.filter(|_| !ws.multi_selected()) {
                ws.select_segment(i, segment.as_deref());
            }
            self.focus = Focus::Tree;
            MenuTarget::Tree(row.map(|i| segment.unwrap_or_else(|| ws.rows()[i].path.clone())))
        } else if let Some((path, m)) = self.search_context_target(x, y) {
            MenuTarget::SearchResult(path, m)
        } else if let Some(i) = self.stash_context_target(x, y) {
            MenuTarget::StashRow(i)
        } else if let Some(path) = self.commit_context_target(x, y) {
            MenuTarget::CommitFile(path)
        } else if let Some((i, path)) = self.log_file_context_target(x, y) {
            MenuTarget::LogFile(i, path)
        } else if let Some(oid) = self.log_context_target(x, y) {
            MenuTarget::LogCommit(oid)
        } else if let Some((id, right)) = self.diff_context_target(x, y) {
            MenuTarget::Diff(id, right)
        } else if let Some(id) = self.terminal_tab_context_target(x, y) {
            MenuTarget::TerminalTab(id)
        } else if let Some(block) = self.terminal_context_target(x, y) {
            MenuTarget::Terminal(block)
        } else if self.markdown_context_target(x, y) {
            MenuTarget::Markdown
        } else if self.pdf_context_target(x, y) {
            MenuTarget::Pdf
        } else if let Some(id) = self.editor_context_target(x, y) {
            MenuTarget::Editor(id)
        } else {
            return;
        };

        self.open_context_menu(target, (x, y));
    }

    /// 右键点在搜索面板的一个结果行上：选中这一行，焦点给结果列表，返回它（文件路径，第几处
    /// 匹配）。
    fn search_context_target(&mut self, x: f32, y: f32) -> Option<(PathBuf, Option<usize>)> {
        if !self.panel_shown() {
            return None;
        }
        if !self.panel_parts().results.contains(x, y) || !self.panel_rect().contains(x, y) {
            return None;
        }
        let index = self.result_row_at(y)?;
        self.global.select_row(index);
        self.focus = Focus::Results;
        self.global.selected.clone()
    }

    /// 右键点在提交视图里的一条储藏上：返回它是第几条储藏。
    fn stash_context_target(&self, x: f32, y: f32) -> Option<usize> {
        if !self.commit_shown() || self.commit.hits_generation != self.git.generation {
            return None;
        }
        match hit(&self.commit_hits, x, y)? {
            CommitTarget::Stash(i) | CommitTarget::StashApply(i) | CommitTarget::StashMenu(i) => {
                Some(i)
            }
            _ => None,
        }
    }

    /// 右键点在提交视图改动列表里的一个文件上：选中它（不打开预览），返回它的路径。
    ///
    /// 点击目录或组标题时选中其下的各个文件，返回第一个文件的路径。
    fn commit_context_target(&mut self, x: f32, y: f32) -> Option<PathBuf> {
        if !self.panel_visible
            || self.sidebar_view != SidebarView::Commit
            || self.commit.hits_generation != self.git.generation
        {
            return None;
        }
        let path = match hit(&self.commit_hits, x, y)? {
            CommitTarget::File(i) | CommitTarget::Check(i) => {
                self.git.status.as_ref()?.entries.get(i)?.path.clone()
            }
            // 目录：菜单作用于它下面的各个文件（都已在选区里时作用于整个选区）。
            CommitTarget::Dir(row) | CommitTarget::DirCheck(row) => {
                let files = self.commit_dir_files(row);
                let first = files.first()?.clone();
                if !files.iter().all(|f| self.commit_is_selected(f)) {
                    self.select_commit_files(files);
                }
                self.focus = Focus::CommitList;
                return Some(first);
            }
            // 组标题的处理方式同目录：菜单作用于组内的各个文件，组收起时同样适用。
            CommitTarget::Group(group) | CommitTarget::GroupCheck(group) => {
                let files: Vec<PathBuf> = self
                    .git
                    .status
                    .as_ref()?
                    .entries
                    .iter()
                    .filter(|e| group.holds(e))
                    .map(|e| e.path.clone())
                    .collect();
                let first = files.first()?.clone();
                if !files.iter().all(|f| self.commit_is_selected(f)) {
                    self.select_commit_files(files);
                }
                self.focus = Focus::CommitList;
                return Some(first);
            }
            _ => return None,
        };
        // 点在选区里的文件上时菜单作用于选中的各个文件，否则只选这一个。
        if !self.commit_is_selected(&path) {
            self.commit.clear_selection();
            self.commit.selected = Some(path.clone());
        }
        self.focus = Focus::CommitList;
        Some(path)
    }

    /// 返回右键点击处所在的编辑区文档；该位置不应弹出编辑区菜单时返回 `None`。
    ///
    /// 点在选区外时光标先移到点击处；点在选区内时保留选区，菜单作用于该选区。查找条、
    /// 跳转条、右上角的冲突卡片和提示条覆盖的区域，以及不显示文本的文档没有菜单。
    fn editor_context_target(&mut self, x: f32, y: f32) -> Option<DocumentId> {
        let l = self.layout;
        let covered = l.search.contains(x, y)
            || (self.goto_input.is_some() && goto_bar(l.editor).contains(x, y))
            || self.over_corner_card(x, y);
        if covered || !l.editor.contains(x, y) {
            return None;
        }

        let pos = self.pos_at(x.max(l.text.x), y)?;
        let doc = self.editor.active_mut()?;
        if !doc.shows_text() {
            return None;
        }
        if !(doc.selection.from()..=doc.selection.to()).contains(&pos) {
            doc.selection = Selection::point(pos);
            doc.goal_cell = None;
        }
        let id = doc.id;

        self.focus = Focus::Editor;
        self.preedit.clear();
        Some(id)
    }

    /// 右键点在终端的标签上：它成为正在看的标签，返回它的编号。
    fn terminal_tab_context_target(&mut self, x: f32, y: f32) -> Option<u16> {
        let index = match crate::layout::hit(&self.terminal.hits, x, y)? {
            crate::app::terminal::HeaderHit::Tab(i) | crate::app::terminal::HeaderHit::Close(i) => {
                i
            }
            _ => return None,
        };
        self.activate_terminal_tab(index);
        self.focus = Focus::Terminal;
        self.terminal.tabs.get(index).map(|t| t.id)
    }

    /// 右键点在终端面板的网格上（标签栏以下）：焦点给终端，弹终端的菜单；返回点着的命令块。
    ///
    /// 程序要了鼠标时右键已经上报给程序了，到不了这里（按着 `⇧` 时才到）。
    fn terminal_context_target(&mut self, x: f32, y: f32) -> Option<Option<u32>> {
        let card = self.layout.terminal;
        let hit = card.contains(x, y)
            && y >= card.y + crate::app::terminal::header_height(card)
            && self.terminal.tab().is_some();
        if !hit {
            return None;
        }
        self.focus = Focus::Terminal;
        Some(self.terminal_block_under(x, y))
    }

    /// 为 `target` 生成菜单项，并在窗口坐标 `at`（逻辑像素）处弹出系统菜单。
    ///
    /// 目标文档已不存在时不弹出。
    pub(in crate::app) fn open_context_menu(&mut self, target: MenuTarget, at: (f32, f32)) {
        let entries = match &target {
            &MenuTarget::AddLanguage(task) => {
                let listed = match task {
                    Task::Format => &self.settings.languages.format,
                    Task::Fix => &self.settings.languages.fix,
                };
                language_menu(task, listed)
            }
            MenuTarget::SkipOnSave => skip_menu(&self.settings.languages.format),
            MenuTarget::Locale => locale_menu(),
            MenuTarget::Whitespace => whitespace_menu(),
            MenuTarget::DefaultShell => default_shell_menu(self.terminal.shells.len()),
            MenuTarget::Zoom => zoom_menu(),
            MenuTarget::TerminalPosition => terminal_position_menu(),
            MenuTarget::OptionAsMeta => option_as_meta_menu(),
            &MenuTarget::Font(setting) => font_menu(setting),
            &MenuTarget::Tool(task, id) => {
                let row = self.settings.tool_row(tool_setting(task, id));
                let install = row.is_some_and(|r| !r.install.is_empty());
                let removable = row.is_some_and(|r| r.removable);
                tool_menu(task, id, install, removable)
            }
            MenuTarget::Project => {
                let root = self.editor.root();
                let current = root.and_then(|r| self.state.recent.iter().position(|p| p == r));
                let mut menu: Vec<MenuEntry> = Vec::new();
                menu.extend(project_menu(
                    self.state.recent.len(),
                    current,
                    root.is_some(),
                    crate::system::wsl_distributions().len(),
                ));
                menu
            }
            MenuTarget::Tree(Some(_))
                if self
                    .editor
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.multi_selected()) =>
            {
                tree_selection_menu(self.tree_selected_files().len())
            }
            MenuTarget::Tree(path) => {
                let kind = path.as_ref().and_then(|p| {
                    let ws = self.editor.workspace.as_ref()?;
                    ws.row_of(p).map(|i| ws.rows()[i].kind)
                });
                let can_paste = self.can_paste_into_tree();
                let mut menu = tree_menu(kind, can_paste);
                // Git 模块开着、文件在仓库里：最后加「显示文件历史」。
                if kind == Some(EntryKind::File) && path.as_deref().is_some_and(|p| self.in_repo(p))
                {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::FileHistory, true),
                    ]);
                }
                menu
            }
            MenuTarget::Editor(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let bookmarked = doc
                    .bookmarks()
                    .lines()
                    .contains(&doc.text().line_of(doc.selection.head));
                let mut menu = editor_menu(
                    doc.readonly_reason().is_none(),
                    !doc.selection.is_empty(),
                    bookmarked,
                );
                // Git 模块开着、文件在仓库里：最后是「逐行追溯」开关。
                if doc.path.as_deref().is_some_and(|p| self.in_repo(p)) {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::ToggleBlame, true),
                    ]);
                }
                menu
            }
            // 可编辑的预览：选区即文档的选区，剪切、复制、粘贴按编辑区的方式处理。
            MenuTarget::Markdown if self.markdown_live() => {
                let selected = self
                    .editor
                    .active()
                    .is_some_and(|d| !d.selection.is_empty());
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::Cut, selected),
                    MenuEntry::Item(MenuCommand::Copy, selected),
                    MenuEntry::Item(MenuCommand::Paste, true),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::SelectAll, true),
                ];
                // 光标位于表格中时提供「表格」子菜单，集中列出行、列的编辑操作。
                if let Some(table) = self.live_table_menu() {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Submenu(t!("menu-table").into(), table),
                    ]);
                }
                menu.extend([
                    MenuEntry::Separator,
                    MenuEntry::Item(
                        MenuCommand::RevealSource,
                        self.markdown.menu_source.is_some(),
                    ),
                ]);
                menu
            }
            MenuTarget::Markdown => {
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::Copy, self.markdown_selected_text().is_some()),
                    MenuEntry::Item(MenuCommand::SelectAll, true),
                ];
                // docx 的阅读视图由转换得到，没有可以显示的源代码。
                let has_source = self
                    .editor
                    .active()
                    .is_some_and(|d| d.unsupported.is_none());
                if has_source {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(
                            MenuCommand::RevealSource,
                            self.markdown.menu_source.is_some(),
                        ),
                    ]);
                }
                menu
            }
            MenuTarget::Pdf => {
                let marked = self.pdf_can_mark();
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::Copy, self.pdf_selected_text().is_some()),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::PdfHighlight, marked),
                    MenuEntry::Item(MenuCommand::PdfUnderline, marked),
                    MenuEntry::Item(MenuCommand::PdfStrikeOut, marked),
                    MenuEntry::Item(MenuCommand::PdfComment, marked),
                ];
                // 右键点在已有的批注上：可以改写注释、删除批注。
                if self.pdf.menu_note.is_some() {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::PdfEditNote, true),
                        MenuEntry::Item(MenuCommand::PdfRemoveNote, true),
                    ]);
                }
                menu
            }
            MenuTarget::TerminalTab(_) => vec![
                MenuEntry::Item(MenuCommand::RenameTerminal, true),
                MenuEntry::Separator,
                MenuEntry::Item(MenuCommand::CloseTerminal, true),
            ],
            &MenuTarget::Terminal(block) => {
                let Some(tab) = self.terminal.tab() else {
                    return;
                };
                let running = tab.ended.is_none() && !self.terminal.crashed;
                let block = block.and_then(|id| {
                    let b = tab.blocks.iter().find(|b| b.id == id)?;
                    Some((!b.text.is_empty(), self.terminal_can_rerun(id)))
                });
                terminal_menu(tab.selection_range().is_some(), running, block)
            }
            &MenuTarget::Diff(id, right) => {
                if self.editor.active().map(|d| d.id) != Some(id) {
                    return;
                }
                diff_menu(right, self.diff_caret_hunk().is_some())
            }
            MenuTarget::Tab(id) => {
                let Some(index) = self.editor.index_of(*id) else {
                    return;
                };
                let docs = &self.editor.documents;
                let path = docs[index].path.as_deref();
                let span = self.tab_span(*id, index);
                let selection = self.tab_selection();
                let selected = if selection.contains(id) {
                    selection.len()
                } else {
                    1
                };
                let mut menu = tab_menu(
                    span,
                    selected,
                    docs.len(),
                    path.is_some(),
                    path.is_some_and(|p| self.relative_path(p).is_some()),
                    docs.iter().any(|d| !d.is_dirty()),
                );
                if path.is_some_and(|p| self.in_repo(p)) {
                    menu.extend([
                        MenuEntry::Separator,
                        MenuEntry::Item(MenuCommand::FileHistory, true),
                    ]);
                }
                menu
            }
            MenuTarget::Indent(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let editable = doc.readonly_reason().is_none() && !doc.is_large();
                indent_menu(indent_size_title(doc.indent()), editable)
            }
            MenuTarget::LineEnding(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let editable = doc.readonly_reason().is_none() && !doc.is_large();
                line_ending_menu(doc.line_ending, editable)
            }
            MenuTarget::Encoding(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                let reopen = doc.path.is_some()
                    && !doc.is_large()
                    && doc.encoding != winc_editor::Encoding::Binary;
                encoding_menu(reopen, |encoding| {
                    doc.encoding.writable() && doc.can_save_with(encoding)
                })
            }
            MenuTarget::Language(id) => {
                let Some(doc) = self.editor.document(*id) else {
                    return;
                };
                document_language_menu(doc.path.is_some())
            }
            MenuTarget::CommitFile(_) if self.commit_multi_selected() => {
                let any_exists = self.commit_selection().iter().any(|p| p.exists());
                commit_selection_menu(any_exists)
            }
            MenuTarget::CommitFile(path) => {
                commit_file_menu(path.exists(), self.relative_path(path).is_some())
            }
            MenuTarget::SearchResult(_, m) => {
                search_result_menu(m.is_none(), self.global.replace_open)
            }
            MenuTarget::CommitMore => self.commit_more_menu(),
            MenuTarget::StashRow(i) => {
                crate::app::git::branch::stash_ops(*i, self.branch.running.is_some())
            }
            MenuTarget::StashList => {
                let busy = self.branch.running.is_some();
                crate::app::git::branch::stash_menu(&self.git.stashes, busy)
            }
            MenuTarget::CommitButton => self.commit_button_menu(),
            MenuTarget::LastCommit => self.last_commit_menu(),
            MenuTarget::TreeTools => self.tree_tools_menu(),
            MenuTarget::TabList => (0..self.editor.documents.len())
                .map(|i| MenuEntry::Item(MenuCommand::ShowTab(i), true))
                .collect(),
            MenuTarget::RunConfigs => (0..self.run.list.len())
                .map(|i| MenuEntry::Item(MenuCommand::RunSelect(i), true))
                .chain([
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuCommand::RunEdit, true),
                ])
                .collect(),
            MenuTarget::TerminalShells => (0..self.terminal_shell_list().len())
                .map(|i| MenuEntry::Item(MenuCommand::NewTerminalShell(i), true))
                .collect(),
            MenuTarget::LogBranch => log_branch_menu(&self.git.branches),
            MenuTarget::StatusPlaces(_) => self.location_menu(),
            MenuTarget::RebaseAction(_) => (crate::app::git::rebase::REBASE_ACTIONS.iter())
                .map(|&action| MenuEntry::Item(MenuCommand::RebaseAction(action), true))
                .collect(),
            MenuTarget::FormBase => form_base_menu(&self.git.branches),
            MenuTarget::LogFile(_, path) => {
                log_file_menu(path.exists(), self.relative_path(path).is_some())
            }
            MenuTarget::LogCommit(oid) => {
                let commit = self.log_commit(oid);
                let tags: Vec<&str> = (commit.iter().flat_map(|c| &c.refs))
                    .filter_map(crate::app::git::log::menus::tag_name)
                    .collect();
                let busy = self.branch.running.is_some()
                    || self
                        .git
                        .status
                        .as_ref()
                        .is_some_and(|s| s.operation.is_some());
                log_commit_menu(!busy, &self.current_branch_label(), &tags)
            }
            MenuTarget::LogAuthor => {
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::LogAuthor(0), true),
                    MenuEntry::Separator,
                ];
                menu.extend(
                    (0..self.log.authors.len())
                        .map(|i| MenuEntry::Item(MenuCommand::LogAuthor(i + 1), true)),
                );
                menu
            }
            // 目录菜单不经过这里（`App::open_crumb_menu`）。
            MenuTarget::Folder => return,
            MenuTarget::Location(_, worktree) => {
                let missing = worktree
                    .and_then(|i| self.git.worktrees.get(i))
                    .is_some_and(|w| w.prunable);
                let removable = worktree.is_some_and(|i| {
                    i > 0
                        && self.git.current_worktree() != Some(i)
                        && self.git.worktrees.get(i).is_some_and(|w| !w.locked)
                        && self.branch.running.is_none()
                });
                let idle = self.branch.running.is_none();
                let mut menu = vec![
                    MenuEntry::Item(MenuCommand::LocationNewWindow, true),
                    MenuEntry::Item(MenuCommand::LocationReveal, true),
                ];
                if worktree.is_some_and(|i| i > 0) {
                    menu.push(MenuEntry::Separator);
                    menu.push(MenuEntry::Item(MenuCommand::LocationRemove, removable));
                }
                // 目录已不在：打不开，也没有东西可以移除，只能清理。
                if missing {
                    menu = vec![MenuEntry::Item(MenuCommand::LocationPrune, idle)];
                }
                menu
            }
            MenuTarget::BranchOps(index) => crate::app::git::branch::branch_ops(
                *index,
                &self.git.branches,
                &self.git.held_branches(),
                &self.git.stale_branches(),
                self.branch.running.is_some(),
            ),
        };

        // 菜单上显示的快捷键按菜单弹出的位置取：文件树中取文件树的键位，编辑区中取编辑区的键位。
        let context = match &target {
            MenuTarget::Tree(_) => Context::Tree,
            MenuTarget::Editor(_)
            | MenuTarget::Diff(..)
            | MenuTarget::Markdown
            | MenuTarget::Pdf => Context::Editor,
            MenuTarget::Terminal(_) | MenuTarget::TerminalTab(_) => Context::Terminal,
            MenuTarget::SearchResult(..) => Context::Panel,
            _ => Context::Other,
        };
        // 状态栏的菜单行为类似弹出式按钮：当前选中的项显示在被点击的位置上。
        let at_checked = matches!(target, MenuTarget::Indent(_) | MenuTarget::LineEnding(_));
        // 拆分按钮的菜单：`at` 是菜单的右上角，菜单往左展开。
        let right_aligned = matches!(
            target,
            MenuTarget::CommitButton
                | MenuTarget::TabList
                | MenuTarget::Locale
                | MenuTarget::Whitespace
                | MenuTarget::Zoom
                | MenuTarget::TerminalPosition
                | MenuTarget::OptionAsMeta
                | MenuTarget::Font(_)
        );
        // 设置中语言、字体等选项的菜单不窄于弹出它的控件。
        let control = match target {
            MenuTarget::Locale => Some(Setting::Language),
            MenuTarget::Whitespace => Some(Setting::RenderWhitespace),
            MenuTarget::Zoom => Some(Setting::Zoom),
            MenuTarget::TerminalPosition => Some(Setting::TerminalPosition),
            MenuTarget::OptionAsMeta => Some(Setting::TerminalOptionAsMeta),
            MenuTarget::Font(setting) => Some(setting),
            _ => None,
        };
        let min_width = control
            .and_then(|setting| self.settings_layout().row(setting).map(|row| row.control.w))
            .unwrap_or(0.0);

        // 菜单项是否打勾、显示什么取决于目标，因此先记录目标，再准备菜单项。
        let (branches, stashes) = match target {
            MenuTarget::BranchOps(_) | MenuTarget::CommitMore => {
                (self.git.branches.clone(), self.git.stashes.clone())
            }
            MenuTarget::StashRow(_) | MenuTarget::StashList => {
                (Vec::new(), self.git.stashes.clone())
            }
            MenuTarget::LogBranch | MenuTarget::FormBase => (self.git.branches.clone(), Vec::new()),
            _ => Default::default(),
        };
        self.context_menu = Some(ContextMenu {
            target,
            entries: entries.clone(),
            branches,
            stashes,
            popup: None,
        });
        self.set_hover(Hover::None);

        // 没有系统菜单（macOS 以外的平台，以及没有窗口的测试中）：在窗口里画出来，由应用自己
        // 处理悬停、点击和按键，打开期间菜单按钮保持按下的样子。
        if !cfg!(target_os = "macos") || self.window.is_none() {
            let rows = self.popup_rows(&entries, context, &mut 0);
            if let Some(menu) = &mut self.context_menu {
                let mut popup = PopupMenu::new(rows, at, (at_checked, right_aligned));
                popup.min_width = min_width;
                menu.popup = Some(popup);
            }
            self.request_redraw();
            return;
        }

        #[cfg(target_os = "macos")]
        let prepared = self.prepare_menu(&entries, context, &mut 0);

        // 菜单打开期间系统在跟踪菜单事件，窗口不会重绘，因此先把选中的行同步绘制一次，
        // 再弹出菜单。
        self.paint_now();

        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.clone() {
            let items: Vec<_> = prepared
                .iter()
                .map(|p| p.as_ref().map(Prepared::item))
                .collect();
            // 界面的逻辑像素乘以界面缩放倍数得到 `AppKit` 的点。
            let zoom = f64::from(self.zoom());
            let at = (f64::from(at.0) * zoom, f64::from(at.1) * zoom);
            let place = crate::macos::MenuPlace {
                at,
                at_checked,
                right_aligned,
                min_width: f64::from(min_width) * zoom,
            };
            crate::macos::popup_menu(&window, &items, place);
        }
    }

    /// 先算好各菜单项需要的数据（标题、快捷键、图标、是否打勾），再借给系统菜单。
    ///
    /// 菜单项按显示顺序从 1 起编号，和 [`MenuEntry::command_at`] 数的一样。
    /// `numbered` 是已经编到的号。
    #[cfg(target_os = "macos")]
    fn prepare_menu(
        &mut self,
        entries: &[MenuEntry],
        context: Context,
        numbered: &mut isize,
    ) -> Vec<Option<Prepared>> {
        use crate::macos::key_equivalent;

        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(command, enabled) | MenuEntry::Alternate(command, enabled) => {
                    Some(Prepared {
                        tag: {
                            *numbered += 1;
                            *numbered
                        },
                        label: self.menu_label(*command),
                        key: command
                            .shortcut(context)
                            .and_then(key_equivalent)
                            .unwrap_or_default(),
                        icon: self.menu_icon(*command),
                        icon_size: menu_icon_size(*command),
                        enabled: *enabled,
                        checked: self.menu_checked(*command),
                        alternate: matches!(entry, MenuEntry::Alternate(..)),
                        children: Vec::new(),
                    })
                }
                MenuEntry::Submenu(title, items) => Some(Prepared {
                    label: MenuLabel::plain(title),
                    key: Default::default(),
                    icon: None,
                    icon_size: MENU_ICON,
                    enabled: true,
                    checked: false,
                    alternate: false,
                    tag: 0,
                    children: self.prepare_menu(items, context, numbered),
                }),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    /// 返回命令 `command` 的菜单项文字。
    pub(crate) fn menu_label(&self, command: MenuCommand) -> MenuLabel {
        match command {
            MenuCommand::OpenRecent(i) => self.recent_label(i),
            MenuCommand::ShowTab(i) => self.tab_list_label(i),
            MenuCommand::RunSelect(i) => {
                let config = self.run.list.get(i);
                // 名字就是命令时不重复显示。
                let subtitle = config.and_then(|c| {
                    if self.config_running(c) {
                        Some(t!(
                            "terminal-run-menu-running",
                            command = c.command.as_str()
                        ))
                    } else {
                        (c.command != c.name).then(|| c.command.clone())
                    }
                });
                MenuLabel {
                    title: config.map(|c| c.name.clone()).unwrap_or_default(),
                    subtitle,
                    tooltip: None,
                }
            }
            MenuCommand::TerminalDock => {
                let title = if self.terminal_right() {
                    t!("terminal-dock-bottom")
                } else {
                    t!("terminal-dock-right")
                };
                MenuLabel {
                    title: title.to_owned(),
                    subtitle: None,
                    tooltip: None,
                }
            }
            MenuCommand::NewTerminalShell(i) => {
                let shell = self.terminal_shell_list().into_iter().nth(i);
                MenuLabel {
                    title: shell.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
                    subtitle: shell.map(|s| s.command),
                    tooltip: None,
                }
            }
            MenuCommand::Zoom(i) => MenuLabel {
                title: (crate::app::settings::ZOOM_STEPS.get(i))
                    .map(|zoom| format!("{zoom}%"))
                    .unwrap_or_default(),
                subtitle: None,
                tooltip: None,
            },
            MenuCommand::DefaultShell(Some(i)) => {
                let shell = self.terminal.shells.get(i);
                MenuLabel {
                    title: shell.map(|s| s.name.clone()).unwrap_or_default(),
                    subtitle: shell.map(|s| s.command.clone()),
                    tooltip: None,
                }
            }
            MenuCommand::FontDefault
                if matches!(
                    &self.context_menu,
                    Some(ContextMenu {
                        target: MenuTarget::Font(Setting::TerminalFont),
                        ..
                    })
                ) =>
            {
                MenuLabel {
                    title: t!("settings-terminal-font-default").to_owned(),
                    subtitle: None,
                    tooltip: None,
                }
            }
            MenuCommand::OpenRecentInNewWindow(i) => {
                let label = self.recent_label(i);
                MenuLabel {
                    title: t!("menu-open-recent-in-new-window", name = &label.title),
                    ..label
                }
            }
            MenuCommand::ToolAuto | MenuCommand::ToolPreset(_) => self.tool_label(command),
            MenuCommand::LogCheckout | MenuCommand::LogCherryPick | MenuCommand::LogNewWorktree => {
                self.log_commit_label(command)
            }
            // 三种重置：名字是标题，各自对改动做什么写在下面一行小字里。
            MenuCommand::LogResetSoft | MenuCommand::LogResetMixed | MenuCommand::LogResetHard => {
                let detail = match command {
                    MenuCommand::LogResetSoft => t!("menu-log-reset-soft-desc"),
                    MenuCommand::LogResetMixed => t!("menu-log-reset-mixed-desc"),
                    _ => t!("menu-log-reset-hard-desc"),
                };
                MenuLabel {
                    subtitle: Some(detail.to_owned()),
                    ..MenuLabel::plain(command.label())
                }
            }
            MenuCommand::StatusPlace(i) => self.location_menu_label(i),
            MenuCommand::LogBranch(i @ 1..) => {
                MenuLabel::plain(self.git.branches.get(i - 1).map_or("", |b| b.name.as_str()))
            }
            MenuCommand::FormBase(0) => match self.git.branches.iter().find(|b| b.current) {
                Some(b) => MenuLabel::plain(&t!("menu-form-base-current-branch", branch = &b.name)),
                None => MenuLabel::plain(t!("menu-form-base-current-commit")),
            },
            MenuCommand::FormBase(i) => {
                MenuLabel::plain(self.git.branches.get(i - 1).map_or("", |b| b.name.as_str()))
            }
            MenuCommand::LogAuthor(i @ 1..) => {
                MenuLabel::plain(self.log.authors.get(i - 1).map_or("", String::as_str))
            }
            MenuCommand::GitBranch(..)
            | MenuCommand::GitFetch
            | MenuCommand::GitPush
            | MenuCommand::GitPull
            | MenuCommand::GitPullRebase
            | MenuCommand::GitContinue
            | MenuCommand::GitAbort => self.git_label(command),
            _ => MenuLabel::plain(command.label()),
        }
    }

    /// 日志里提交菜单的文字：有本地分支指着这个提交时「签出「dev」」；拣选写明拣到哪个分支。
    fn log_commit_label(&self, command: MenuCommand) -> MenuLabel {
        let Some(ContextMenu {
            target: MenuTarget::LogCommit(oid),
            ..
        }) = &self.context_menu
        else {
            return MenuLabel::plain(command.label());
        };
        match command {
            MenuCommand::LogCheckout => match self.log_checkout_branch(oid) {
                Some(branch) => MenuLabel::plain(&t!("menu-log-checkout-branch", branch = branch)),
                None => MenuLabel::plain(command.label()),
            },
            // 有能签出的本地分支指着它时签出那个分支，否则游离地签出提交；建在哪儿写在下面。
            MenuCommand::LogNewWorktree => {
                let branch = self.log_checkout_branch(oid);
                let branch = branch.and_then(|b| self.log_worktree_branch(&b));
                let (title, dir) = match branch {
                    Some(b) => (t!("menu-log-new-worktree-branch", branch = &b.name), b.name),
                    None => (
                        command.label().to_owned(),
                        crate::app::git::log::short(oid).into(),
                    ),
                };
                MenuLabel {
                    title,
                    subtitle: Some(self.worktree_dir_hint(&dir)),
                    tooltip: None,
                }
            }
            _ => {
                let branch = self.current_branch_label();
                MenuLabel::plain(&t!("menu-log-cherry-pick-onto", branch = branch))
            }
        }
    }

    /// 分支操作菜单和「⋯」里的文字：推送、拉取下方写明和哪个上游、差几个提交。
    fn git_label(&self, command: MenuCommand) -> MenuLabel {
        use crate::app::git::branch::BranchOp;
        let current = self.git.branches.iter().find(|b| b.current);
        let here = current.map_or(t!("menu-git-current-branch"), |b| b.name.as_str());
        let upstream = current.and_then(|b| b.upstream.as_deref());
        let (ahead, behind) = self
            .git
            .status
            .as_ref()
            .map_or((0, 0), |s| (s.ahead, s.behind));
        let title = match command {
            // 已经在别的工作树里签出（远程分支看同名的本地分支）：签出就是切过去。
            MenuCommand::GitBranch(BranchOp::Checkout, i) => {
                let holding = self.git.branches.get(i).and_then(|b| {
                    let name = crate::app::git::branch::local_name(b);
                    self.git.worktree_holding(name)
                });
                match holding {
                    Some(w) => {
                        let worktree = &self.git.worktrees[w];
                        let name = crate::app::git::worktree::worktree_name(worktree);
                        t!("menu-git-switch-to-worktree", name = name)
                    }
                    None => command.label().to_owned(),
                }
            }
            MenuCommand::GitContinue | MenuCommand::GitAbort => {
                use winc_editor::git::Operation;
                let operation = self.git.status.as_ref().and_then(|s| s.operation);
                let label = operation.map_or("", Operation::label);
                match (command, operation) {
                    (MenuCommand::GitContinue, Some(Operation::Merge)) => {
                        t!("menu-git-finish-operation", operation = label)
                    }
                    (MenuCommand::GitContinue, _) => {
                        t!("menu-git-continue-operation", operation = label)
                    }
                    _ => t!("menu-git-abort-operation", operation = label),
                }
            }
            _ => command.label().to_owned(),
        };
        let subtitle = match (command, upstream) {
            // 被目录已经不在的工作树占着：git 不让签出，写明要先清理。
            (MenuCommand::GitBranch(BranchOp::Checkout, i), _)
                if self.git.branches.get(i).is_some_and(|b| {
                    let name = crate::app::git::branch::local_name(b);
                    self.git.stale_holding(name).is_some()
                }) =>
            {
                Some(t!("menu-git-checkout-stale-worktree").to_owned())
            }
            (MenuCommand::GitFetch, _) => Some(t!("menu-git-fetch-desc").to_owned()),
            (MenuCommand::GitContinue, _) => Some(t!("menu-git-continue-desc").to_owned()),
            (MenuCommand::GitAbort, _) => Some(t!("menu-git-abort-desc").to_owned()),
            (MenuCommand::GitPush, Some(up)) if ahead > 0 => {
                Some(t!("menu-git-push-ahead", count = ahead, upstream = up))
            }
            (MenuCommand::GitPush, Some(up)) => Some(t!("menu-git-push-up-to-date", upstream = up)),
            (MenuCommand::GitPush, None) => Some(t!("menu-git-push-set-upstream", branch = here)),
            (MenuCommand::GitPull, Some(up)) if behind > 0 => {
                Some(t!("menu-git-pull-behind", count = behind, upstream = up))
            }
            (MenuCommand::GitPull, Some(up)) => Some(t!("menu-git-pull-from", upstream = up)),
            (MenuCommand::GitPull | MenuCommand::GitPullRebase, None) => {
                Some(t!("menu-git-pull-no-upstream").to_owned())
            }
            _ => None,
        };
        MenuLabel {
            title,
            subtitle,
            tooltip: None,
        }
    }

    /// 返回最近目录第 `i` 项的菜单文字：标题只写目录名，完整路径放在悬停提示中（主目录显示为
    /// `~`）。
    ///
    /// 只有最近列表中另有同名目录时，才在标题下方用一行小字写明所在的上级目录；否则每一项
    /// 都会多出一行相同的上级目录，既占空间又没有信息量。
    fn recent_label(&self, i: usize) -> MenuLabel {
        let Some(folder) = self.state.recent.get(i) else {
            return MenuLabel::plain("");
        };
        let name = winc_editor::display_name(folder);
        let same_name = |other: &PathBuf| winc_editor::display_name(other) == name;
        let ambiguous = self.state.recent.iter().filter(|p| same_name(p)).count() > 1;
        MenuLabel {
            subtitle: folder
                .parent()
                .filter(|_| ambiguous)
                .map(|parent| winc_editor::display_path(parent, None)),
            tooltip: Some(winc_editor::display_path(folder, None)),
            title: name.into_owned(),
        }
    }

    /// 返回格式化（修复）工具菜单中一项的文字：「自动」下方写着当前自动选中的工具；
    /// 预设下方写着该预设的命令，没装的在名字后面注明「（未安装）」。
    fn tool_label(&self, command: MenuCommand) -> MenuLabel {
        let preset = match command {
            MenuCommand::ToolPreset(i) => PRESETS.get(i),
            _ => None,
        };
        let Some(preset) = preset else {
            let (task, auto) = match &self.context_menu {
                Some(ContextMenu {
                    target: MenuTarget::Tool(task, id),
                    ..
                }) => {
                    let row = self.settings.tool_row(tool_setting(*task, id));
                    (*task, row.and_then(|r| r.auto.clone()))
                }
                _ => (Task::Format, None),
            };
            let subtitle = auto.unwrap_or_else(|| {
                match task {
                    Task::Format => t!("menu-tool-auto-format-desc"),
                    Task::Fix => t!("menu-tool-auto-fix-desc"),
                }
                .to_owned()
            });
            return MenuLabel {
                title: command.label().to_owned(),
                subtitle: Some(subtitle),
                tooltip: None,
            };
        };
        let title = match self.format.formatter.has(preset.program()) {
            Some(false) => t!("menu-tool-not-installed", name = preset.name),
            _ => preset.name.to_owned(),
        };
        MenuLabel {
            title,
            subtitle: Some(preset.command.to_owned()),
            tooltip: None,
        }
    }

    /// 是否给菜单项 `command` 打勾。
    ///
    /// 状态栏缩进、行尾、编码菜单中当前使用的那一项打勾；「排除的语言」菜单中已被排除的语言打勾；
    /// 格式化工具菜单中当前使用的那一项打勾。
    pub(in crate::app) fn menu_checked(&self, command: MenuCommand) -> bool {
        if let MenuCommand::ShowTab(i) = command {
            return self.editor.active == Some(i);
        }
        if let MenuCommand::RunSelect(i) = command {
            let current = self.run.current();
            return current.is_some_and(|c| self.run.list.get(i) == Some(c));
        }
        if command == MenuCommand::ToggleBlame {
            return self.editor.active().is_some_and(|d| self.blame.is_on(d.id));
        }
        if command == MenuCommand::CommitAmend {
            return self.commit.amend;
        }
        if command == MenuCommand::CommitTree {
            return !self.commit.flat();
        }
        if command == MenuCommand::TerminalMaximize {
            return self.terminal.maximized;
        }
        // 日志的分支、作者筛选：现在用着的那一项。
        let query = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .map(|v| &v.query);
        if let MenuCommand::FormBase(i) = command {
            return self.form_base_chosen(i);
        }
        if let MenuCommand::StatusPlace(i) = command {
            return self.location_places().get(i).is_some_and(|p| p.current);
        }
        match (command, query) {
            (MenuCommand::LogBranch(i), Some(q)) => {
                return match i.checked_sub(1) {
                    None => q.branch.is_none(),
                    Some(i) => self
                        .git
                        .branches
                        .get(i)
                        .is_some_and(|b| q.branch.as_deref() == Some(b.name.as_str())),
                };
            }
            (MenuCommand::LogAuthor(i), Some(q)) => {
                return match i.checked_sub(1) {
                    None => q.author.is_none(),
                    Some(i) => self.log.authors.get(i) == q.author.as_ref(),
                };
            }
            _ => {}
        }

        // 状态栏的缩进、行尾、编码、语言菜单：当前使用的那一种。
        let status_doc = match &self.context_menu {
            Some(ContextMenu {
                target:
                    MenuTarget::Indent(id)
                    | MenuTarget::LineEnding(id)
                    | MenuTarget::Encoding(id)
                    | MenuTarget::Language(id),
                ..
            }) => self.editor.document(*id),
            _ => None,
        };
        if let Some(doc) = status_doc {
            let tab_width = self.editor.tab_width;
            return match command {
                MenuCommand::IndentUsingSpaces => matches!(doc.indent(), Indent::Spaces(_)),
                MenuCommand::IndentUsingTabs => doc.indent() == Indent::Tabs,
                MenuCommand::IndentSize(n) => doc.indent_size(tab_width) == n,
                MenuCommand::LineEnding(eol) => doc.line_ending == eol,
                MenuCommand::ReopenEncoding(i) | MenuCommand::SaveEncoding(i) => {
                    CHOICES.get(i).is_some_and(|(e, _)| *e == doc.encoding)
                }
                MenuCommand::UseLanguage(i) => (winc_syntax::languages().get(i))
                    .is_some_and(|l| std::ptr::eq(*l, doc.highlighter.language())),
                MenuCommand::UsePlainText => doc.highlighter.language().id == "text",
                _ => false,
            };
        }

        let tool = match &self.context_menu {
            Some(ContextMenu {
                target: MenuTarget::Tool(task, id),
                ..
            }) => Some((*task, *id)),
            _ => None,
        };
        let language = tool.map(|(_, id)| id);
        let configured = tool.and_then(|(task, id)| {
            crate::app::editor::format::task_commands(&self.config, task).get(id)
        });
        match command {
            MenuCommand::SkipOnSave(i) => winc_syntax::languages().get(i).is_some_and(|l| {
                self.config
                    .format_on_save_exclude
                    .iter()
                    .any(|id| id == l.id)
            }),
            MenuCommand::Locale(i) => {
                winc_i18n::LocaleChoice::ALL.get(i)
                    == Some(&self.config.language.unwrap_or_default())
            }
            MenuCommand::Whitespace(i) => {
                winc_config::RenderWhitespace::ALL.get(i)
                    == Some(&self.config.render_whitespace.unwrap_or_default())
            }
            MenuCommand::DefaultShell(i) => {
                let configured = self.configured_shell().map(|shell| shell.command);
                let listed = i.and_then(|i| self.terminal.shells.get(i));
                configured == listed.map(|shell| shell.command.clone())
            }
            MenuCommand::DefaultShellCustom => self.configured_shell().is_some_and(|configured| {
                (self.terminal.shells.iter()).all(|shell| shell.command != configured.command)
            }),
            MenuCommand::TerminalPosition(right) => self.terminal_right() == right,
            MenuCommand::Zoom(i) => {
                crate::app::settings::ZOOM_STEPS.get(i) == Some(&self.config.ui_zoom.unwrap_or(100))
            }
            MenuCommand::OptionAsMeta(i) => {
                let current =
                    (self.config.option_as_meta).unwrap_or(winc_config::OptionAsMeta::Left);
                winc_config::OptionAsMeta::ALL.get(i) == Some(&current)
            }
            MenuCommand::FontDefault | MenuCommand::Font(_) => {
                let Some(ContextMenu {
                    target: MenuTarget::Font(setting),
                    ..
                }) = &self.context_menu
                else {
                    return false;
                };
                let current = match setting {
                    Setting::UiFont => &self.config.ui_font,
                    Setting::TerminalFont => &self.config.terminal_font,
                    _ => &self.config.editor_font,
                };
                match command {
                    MenuCommand::Font(i) => winc_render::installed_fonts()
                        .get(i)
                        .is_some_and(|family| current.as_deref() == Some(family.name.as_str())),
                    _ => current.is_none(),
                }
            }
            MenuCommand::ToolAuto => language.is_some() && configured.is_none(),
            MenuCommand::ToolPreset(i) => {
                configured.is_some_and(|c| PRESETS.get(i).is_some_and(|p| p.command == c))
            }
            MenuCommand::ToolCustom => {
                configured.is_some_and(|c| PRESETS.iter().all(|p| p.command != c))
            }
            _ => false,
        }
    }

    /// 返回菜单项左边的图标：最近的目录用目录图标，工作树用分支图标，标签列表用标签上的图标；
    /// 其他命令没有图标。
    pub(in crate::app) fn menu_icon_kind(&self, command: MenuCommand) -> Option<MenuIcon> {
        match command {
            MenuCommand::OpenRecent(_) | MenuCommand::OpenRecentInNewWindow(_) => {
                Some(MenuIcon::Folder)
            }
            MenuCommand::ShowTab(i) => self.editor.documents.get(i).map(|_| MenuIcon::Tab(i)),
            _ => None,
        }
    }

    /// 在 `(x, y)`（图标位的左上角）绘制菜单项图标 `icon`，图标位的尺寸见
    /// [`MenuIcon::size`]。
    ///
    /// 窗口的深浅外观跟随主题，菜单跟随窗口，因此图标颜色与菜单的深浅外观一致。
    pub(crate) fn draw_menu_icon(
        &self,
        frame: &mut winc_render::Frame<'_>,
        icon: MenuIcon,
        (x, y): (f32, f32),
        clip: Rect,
    ) {
        let (w, h) = icon.size();
        match icon {
            MenuIcon::Folder => icons::folder(frame, (x, y), false, self.palette.folder, clip),
            MenuIcon::File(glyph, [dark, light]) => {
                let color = winc_render::Color::hex(if self.palette.dark { dark } else { light });
                // 文件图标 12 × 15。
                icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
            }
            MenuIcon::Tab(i) => {
                let Some(doc) = self.editor.documents.get(i) else {
                    return;
                };
                if doc.log.is_some() {
                    icons::git(
                        frame,
                        (x + w / 2.0, y + h / 2.0),
                        1.0,
                        self.palette.accent,
                        clip,
                    );
                } else {
                    let (glyph, color) = self.palette.file_icon(&crate::view::tab_icon_name(doc));
                    // 文件图标 12 × 15。
                    icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
                }
            }
        }
    }

    /// 把菜单项 `command` 的图标画成图片，交给系统菜单；没有图标或没有渲染器时为 `None`。
    #[cfg(any(target_os = "macos", test))]
    pub(in crate::app) fn menu_icon(&mut self, command: MenuCommand) -> Option<Image> {
        let icon = self.menu_icon_kind(command)?;
        let (w, h) = icon.size();
        let clip = Rect::new(0.0, 0.0, w, h);
        let mut renderer = self.renderer.take()?;
        let image = renderer.draw_transparent(w, h, |frame| {
            self.draw_menu_icon(frame, icon, (0.0, 0.0), clip);
        });
        self.renderer = Some(renderer);
        Some(image)
    }

    /// 返回标签列表第 `i` 项的菜单文字。
    ///
    /// 标题为标签上的名字；存在同名标签时，标题下方以小字显示标签栏所标的上级目录，
    /// 用于区分同名文件。完整路径显示在悬停提示中（主目录显示为 `~`）。
    fn tab_list_label(&self, i: usize) -> MenuLabel {
        let docs = &self.editor.documents;
        let Some(doc) = docs.get(i) else {
            return MenuLabel::plain("");
        };
        MenuLabel {
            title: doc.name().into_owned(),
            subtitle: crate::view::tab_folder(docs, i),
            tooltip: doc
                .path
                .as_deref()
                .map(|p| winc_editor::display_path(p, None)),
        }
    }

    /// 文件树工具栏「⋯」的菜单：列出收起的各个工具，按工具栏中的顺序排列。
    pub(super) fn tree_tools_menu(&self) -> Vec<MenuEntry> {
        crate::layout::folded_tools(self.tree_tools.len())
            .iter()
            .map(|&tool| {
                let command = match tool {
                    TreeTool::NewFile => MenuCommand::NewFile,
                    TreeTool::NewFolder => MenuCommand::NewFolder,
                    TreeTool::Refresh => MenuCommand::TreeRefresh,
                    TreeTool::CollapseAll => MenuCommand::TreeCollapseAll,
                    TreeTool::Locate => MenuCommand::TreeLocate,
                };
                MenuEntry::Item(command, true)
            })
            .collect()
    }

    /// 处理对项目名的点击：在项目名下方弹出最近项目菜单。
    pub(in crate::app) fn open_project_menu(&mut self) {
        self.git_discover();
        self.git_scan_repositories();
        let hit = self.project_hit;
        self.open_button_menu(MenuButton::Project, MenuTarget::Project, menu_below(hit));
    }

    /// 从按钮 `button` 上弹出 `target` 的菜单（在 `at` 处，见 [`App::open_context_menu`]）：系统
    /// 菜单开着期间按钮画成按下的样子，菜单关掉以后还原。
    pub(in crate::app) fn open_button_menu(
        &mut self,
        button: MenuButton,
        target: MenuTarget,
        at: (f32, f32),
    ) {
        self.menu_button = Some(button);
        self.open_context_menu(target, at);
        // 系统菜单关掉以后才返回；窗口内的菜单还开着，关掉时再放开按钮。
        if self.popup().is_none() {
            self.menu_button = None;
        }
        self.request_redraw();
    }

    /// 立即同步绘制一帧，不等待下一轮事件循环。
    pub(in crate::app) fn paint_now(&mut self) {
        self.relayout();
        if let Some(mut renderer) = self.renderer.take() {
            self.cover_frame(&mut renderer);
            renderer.draw(|frame| view::draw(self, frame));
            self.renderer = Some(renderer);
        }
    }
}
