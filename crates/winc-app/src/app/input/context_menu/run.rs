//! 执行菜单里选中的命令，以及几个命令共用的动作（复制路径、改名、删除、在目录中查找）。

use winc_i18n::t;

use super::*;

impl App {
    /// 处理菜单中选中的命令 `command`：作用于弹出菜单时记录的目标。
    pub(in crate::app) fn run_menu_command(&mut self, command: MenuCommand) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        // 弹出菜单的按钮在菜单关掉以后放开。
        self.menu_button = None;

        // 菜单中灰显的项本来点不到，这里再校验一次（例如菜单打开期间按了它的快捷键）。
        if !MenuEntry::offers(&menu.entries, command) {
            return;
        }

        // 选择器里的菜单选了一项：操作开始，选择器不再需要；取消时选择器留着。
        if matches!(
            menu.target,
            MenuTarget::BranchOps(_) | MenuTarget::Location(..)
        ) {
            self.close_picker();
        }

        match menu.target {
            MenuTarget::Tree(path) => self.run_tree_command(command, path),
            MenuTarget::Tab(id) => self.run_tab_command(command, id),
            MenuTarget::CommitFile(path) => self.run_commit_file_command(command, path),
            MenuTarget::LogCommit(oid) => self.run_log_commit_command(command, &oid),
            MenuTarget::LogFile(i, path) => self.run_log_file_command(command, i, path),
            MenuTarget::SearchResult(path, m) => self.run_search_result_command(command, path, m),
            // 分支、储藏的下标按弹出菜单时的列表解释：菜单打开期间状态刷新后，
            // 最新列表中相同下标的项可能是另一个分支。
            MenuTarget::RebaseAction(row) => self.run_rebase_action_command(row, command),
            MenuTarget::FormBase => {
                if let MenuCommand::FormBase(i) = command {
                    let base = i.checked_sub(1).and_then(|i| menu.branches.get(i));
                    self.choose_form_base(base.map(|b| b.name.clone()));
                }
            }
            MenuTarget::StatusPlaces(places) => match command {
                MenuCommand::StatusPlace(i) => {
                    if let Some(place) = places.get(i) {
                        self.go_to_place(place);
                    }
                }
                MenuCommand::NewWorktree => self.open_worktree_dialog(),
                _ => {}
            },
            MenuTarget::LogBranch => {
                if let MenuCommand::LogBranch(i) = command {
                    let branch = i.checked_sub(1).and_then(|i| menu.branches.get(i));
                    let branch = branch.map(|b| b.name.clone());
                    self.change_log_query(|q| q.branch = branch);
                }
            }
            MenuTarget::LogAuthor => {
                if let MenuCommand::LogAuthor(i) = command {
                    let author = i
                        .checked_sub(1)
                        .and_then(|i| self.log.authors.get(i))
                        .cloned();
                    self.change_log_query(|q| q.author = author);
                }
            }
            MenuTarget::Editor(id) => self.run_editor_command(command, id),
            MenuTarget::Markdown => match command {
                MenuCommand::Cut => self.copy(true),
                MenuCommand::Copy => self.copy(false),
                MenuCommand::Paste => self.paste(),
                MenuCommand::SelectAll if self.markdown_live() => {
                    self.run(winc_editor::Command::SelectAll);
                }
                MenuCommand::SelectAll => self.markdown_select_all(),
                MenuCommand::RevealSource => self.reveal_markdown_source(),
                MenuCommand::Table(op) => {
                    self.live_table_tool(op);
                }
                _ => {}
            },
            MenuTarget::Pdf => {
                use winc_preview::pdf::AnnotationKind;
                match command {
                    MenuCommand::Copy => self.copy(false),
                    MenuCommand::PdfHighlight => self.pdf_mark(AnnotationKind::Highlight),
                    MenuCommand::PdfUnderline => self.pdf_mark(AnnotationKind::Underline),
                    MenuCommand::PdfStrikeOut => self.pdf_mark(AnnotationKind::StrikeOut),
                    MenuCommand::PdfComment => self.pdf_comment_selection(),
                    MenuCommand::PdfEditNote => self.pdf_edit_note(),
                    MenuCommand::PdfRemoveNote => self.pdf_remove_note(),
                    MenuCommand::PdfInstallEngine => self.install_pdf_engine(),
                    _ => {}
                }
            }
            MenuTarget::Terminal(block) => self.run_terminal_command(command, block),
            MenuTarget::TerminalTab(id) => {
                let Some(index) = self.terminal.tabs.iter().position(|t| t.id == id) else {
                    return;
                };
                match command {
                    MenuCommand::RenameTerminal => self.open_terminal_rename(index),
                    MenuCommand::CloseTerminal => self.close_terminal_tab(index),
                    _ => {}
                }
            }
            // 和快捷键走同一条路：光标在右侧时由右侧的文档执行，只读的一侧有选区时复制这一侧。
            MenuTarget::Diff(id, _) => {
                if self.editor.active().map(|d| d.id) == Some(id)
                    && let Some(action) = command.action()
                {
                    self.run_action(action);
                }
            }
            MenuTarget::Project => match command {
                MenuCommand::OpenRecent(i) => self.run_start_action(StartAction::Recent(i)),
                MenuCommand::OpenRecentInNewWindow(i) => self.open_recent_in_new_window(i),
                MenuCommand::OpenFolder => self.pick(Pick::OpenFolder),
                MenuCommand::OpenWslFolder(i) => {
                    if let Some(distro) = crate::system::wsl_distributions().get(i) {
                        self.pick(Pick::WslFolder(distro.clone()));
                    }
                }
                MenuCommand::CloseFolder => self.close_folder(),
                _ => {}
            },
            MenuTarget::AddLanguage(task) => {
                if let MenuCommand::Language(i) = command
                    && let Some(&language) = winc_syntax::languages().get(i)
                {
                    self.settings_add_language(task, language);
                }
            }
            MenuTarget::SkipOnSave => {
                if let MenuCommand::SkipOnSave(i) = command
                    && let Some(&language) = winc_syntax::languages().get(i)
                {
                    self.settings_toggle_skip(language.id);
                }
            }
            MenuTarget::Locale => {
                if let MenuCommand::Locale(i) = command
                    && let Some(&choice) = winc_i18n::LocaleChoice::ALL.get(i)
                {
                    self.set_locale_choice(choice);
                }
            }
            MenuTarget::Whitespace => {
                if let MenuCommand::Whitespace(i) = command
                    && let Some(&choice) = winc_config::RenderWhitespace::ALL.get(i)
                {
                    self.set_render_whitespace(choice);
                }
            }
            MenuTarget::DefaultShell => match command {
                MenuCommand::DefaultShell(None) => self.set_terminal_shell(None),
                MenuCommand::DefaultShell(Some(i)) => {
                    if let Some(shell) = self.terminal.shells.get(i) {
                        self.set_terminal_shell(Some(shell.command.clone()));
                    }
                }
                MenuCommand::DefaultShellCustom => {
                    self.settings_start_editing(Setting::TerminalShell)
                }
                _ => {}
            },
            MenuTarget::Zoom => {
                if let MenuCommand::Zoom(i) = command
                    && let Some(&zoom) = crate::app::settings::ZOOM_STEPS.get(i)
                {
                    self.set_zoom(zoom);
                }
            }
            MenuTarget::TerminalPosition => {
                if let MenuCommand::TerminalPosition(right) = command {
                    self.set_terminal_position(right);
                }
            }
            MenuTarget::OptionAsMeta => {
                if let MenuCommand::OptionAsMeta(i) = command
                    && let Some(&choice) = winc_config::OptionAsMeta::ALL.get(i)
                {
                    self.set_option_as_meta(choice);
                }
            }
            MenuTarget::Font(setting) => match command {
                MenuCommand::FontDefault => self.settings_choose_font(setting, None),
                MenuCommand::Font(i) => {
                    if let Some(family) = winc_render::installed_fonts().get(i) {
                        self.settings_choose_font(setting, Some(family.name.clone()));
                    }
                }
                _ => {}
            },
            MenuTarget::Tool(task, id) => {
                self.settings_choose_tool(tool_setting(task, id), command)
            }
            MenuTarget::Indent(id)
            | MenuTarget::LineEnding(id)
            | MenuTarget::Encoding(id)
            | MenuTarget::Language(id) => self.run_status_command(command, id),
            MenuTarget::BranchOps(_) => {
                if let MenuCommand::GitBranch(op, i) = command
                    && let Some(branch) = menu.branches.get(i).cloned()
                {
                    self.run_branch_op(op, branch);
                }
            }
            MenuTarget::Location(path, worktree) => match command {
                MenuCommand::LocationNewWindow => self.open_in_new_window(&path),
                MenuCommand::LocationReveal => self.reveal_in_file_manager(&path),
                MenuCommand::LocationPrune => {
                    self.close_picker();
                    self.prune_worktrees();
                }
                MenuCommand::LocationRemove => {
                    if let Some(i) = worktree {
                        self.remove_worktree(i);
                    }
                }
                _ => {}
            },
            MenuTarget::CommitMore => self.run_commit_more_command(command),
            // 按弹出菜单时记下的那份储藏找：菜单开着的时候储藏的编号可能已经变了。
            MenuTarget::StashRow(_) | MenuTarget::StashList => {
                if let MenuCommand::GitStashOp(op, i) = command
                    && let Some(stash) = menu.stashes.get(i).cloned()
                {
                    self.run_stash_op(op, stash);
                }
            }
            MenuTarget::CommitButton => self.run_commit_button_command(command),
            MenuTarget::LastCommit => self.run_last_commit_command(command),
            MenuTarget::TabList => {
                if let MenuCommand::ShowTab(i) = command {
                    self.activate_tab(i);
                }
            }
            MenuTarget::RunConfigs => match command {
                MenuCommand::RunSelect(i) => self.select_run_config(i),
                MenuCommand::RunEdit => self.edit_run_configs(),
                _ => {}
            },
            MenuTarget::TerminalShells => {
                if let MenuCommand::NewTerminalShell(i) = command {
                    self.new_terminal_with_shell(i);
                }
            }
            MenuTarget::TreeTools => {
                if let Some(tool) = tree_tool_of(command) {
                    self.run_tree_tool(tool);
                }
            }
            // 目录菜单没有命令，选中的文件直接打开（`PopupRow::Entry`）。
            MenuTarget::Folder => {}
        }

        self.request_redraw();
    }

    /// 执行文件树菜单中的命令 `command`；`path` 为右键点中的项，`None` 表示根目录。
    pub(in crate::app) fn run_tree_command(&mut self, command: MenuCommand, path: Option<PathBuf>) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let target = path.clone().unwrap_or_else(|| root.clone());

        // 选中了多项时（菜单只有这几项）作用于选中的各项。
        if path.is_some()
            && self
                .editor
                .workspace
                .as_ref()
                .is_some_and(|w| w.multi_selected())
        {
            let selection = self.editor.workspace.as_ref().map(|w| w.selection());
            let selection = selection.unwrap_or_default();
            match command {
                MenuCommand::OpenFiles => self.open_tree_selection(),
                MenuCommand::CompareFiles => {
                    if let [left, right] = &self.tree_selected_files()[..] {
                        self.compare_files(left, right);
                    }
                }
                MenuCommand::Cut => self.cut_entries(&self.tree_targets()),
                MenuCommand::Copy => self.copy_entries(&self.tree_targets()),
                MenuCommand::Delete => self.ask_delete_all(self.tree_targets()),
                MenuCommand::CopyPath => self.copy_paths(&selection, false),
                MenuCommand::CopyRelativePath => self.copy_paths(&selection, true),
                _ => {}
            }
            return;
        }

        match command {
            MenuCommand::FileHistory => {
                if let Some(path) = &path {
                    self.git_file_history(path);
                }
            }
            MenuCommand::NewFile | MenuCommand::NewFolder => {
                // 右键时已选中了那一行（空白处则取消选中），新建的位置由选中的行决定。
                self.start_new_entry(command == MenuCommand::NewFolder);
            }
            MenuCommand::Reveal => self.reveal_in_file_manager(&target),
            MenuCommand::OpenInTerminal => self.open_in_terminal(Some(&target)),
            MenuCommand::FindInFolder => self.find_in_folder(&target),
            MenuCommand::CopyPath => self.copy_path(&target, false),
            MenuCommand::CopyRelativePath => self.copy_path(&target, true),
            MenuCommand::Rename => self.start_rename(&target),
            MenuCommand::Delete => self.ask_delete(&target),
            MenuCommand::Cut => self.cut_entries(std::slice::from_ref(&target)),
            MenuCommand::Copy => self.copy_entry(&target),
            // 右键时已选中了那一行（空白处则取消选中），粘贴到的目录由选中的行决定。
            MenuCommand::Paste => self.paste_into_tree(),
            _ => {}
        }
    }

    fn run_terminal_command(&mut self, command: MenuCommand, block: Option<u32>) {
        match (command, block) {
            (MenuCommand::CopyCommand, Some(id)) => return self.copy_terminal_command(id),
            (MenuCommand::CopyOutput, Some(id)) => return self.copy_terminal_output(id),
            (MenuCommand::Rerun, Some(id)) => return self.rerun_terminal_command(id),
            _ => {}
        }
        match command {
            MenuCommand::Copy => self.terminal_copy(),
            MenuCommand::Paste => self.terminal_paste(),
            MenuCommand::SelectAll => self.terminal_select_all(),
            MenuCommand::TerminalFind => self.open_terminal_find(),
            MenuCommand::TerminalClear => self.clear_terminal(),
            MenuCommand::TerminalMaximize => self.toggle_terminal_maximized(),
            MenuCommand::TerminalDock => self.toggle_terminal_position(),
            MenuCommand::NewTerminal => self.new_terminal_tab(),
            MenuCommand::CloseTerminal => self.close_terminal_tab(self.terminal.active),
            _ => {}
        }
    }

    fn run_editor_command(&mut self, command: MenuCommand, id: DocumentId) {
        if self.editor.active().map(|d| d.id) != Some(id) {
            return;
        }
        match command {
            MenuCommand::ToggleBlame => self.toggle_blame(),
            MenuCommand::Cut => self.copy(true),
            MenuCommand::Copy => self.copy(false),
            MenuCommand::Paste => self.paste(),
            MenuCommand::Format => self.format_active(),
            MenuCommand::FormatSelection => self.format_selection(),
            MenuCommand::Fix => self.fix_active(),
            MenuCommand::GoToDefinition => self.go_to_definition(),
            MenuCommand::FindReferences => self.find_references(),
            MenuCommand::AddBookmark => self.set_bookmark(true),
            MenuCommand::RemoveBookmark => self.set_bookmark(false),
            MenuCommand::ListBookmarks => self.list_bookmarks(),
            MenuCommand::CopyReference => self.copy_reference(false),
            _ => {}
        }
    }

    fn run_tab_command(&mut self, command: MenuCommand, id: DocumentId) {
        let Some(index) = self.editor.index_of(id) else {
            return;
        };
        let docs = &self.editor.documents;
        let path = docs[index].path.clone();

        // 右键点在多选的标签上：关闭、关闭其他作用于选中的各个标签，关闭左侧、右侧以选中的
        // 最左、最右一个为界。
        let selection = self.tab_selection();
        let (first, last) = self.tab_span(id, index);
        if selection.len() > 1 && selection.contains(&id) {
            match command {
                MenuCommand::Close => return self.close_tabs(selection),
                MenuCommand::CloseOthers => {
                    let ids = docs
                        .iter()
                        .map(|d| d.id)
                        .filter(|d| !selection.contains(d))
                        .collect();
                    return self.close_tabs(ids);
                }
                _ => {}
            }
        }

        match command {
            MenuCommand::FileHistory => {
                if let Some(path) = path {
                    self.git_file_history(&path);
                }
            }
            MenuCommand::Close => self.close_tab(index),
            MenuCommand::CloseOthers => {
                let ids = docs.iter().filter(|d| d.id != id).map(|d| d.id).collect();
                self.close_tabs(ids);
            }
            MenuCommand::CloseRight => {
                let ids = docs[last + 1..].iter().map(|d| d.id).collect();
                self.close_tabs(ids);
            }
            MenuCommand::CloseLeft => {
                let ids = docs[..first].iter().map(|d| d.id).collect();
                self.close_tabs(ids);
            }
            MenuCommand::CloseSaved => {
                let ids = docs
                    .iter()
                    .filter(|d| !d.is_dirty())
                    .map(|d| d.id)
                    .collect();
                self.close_tabs(ids);
            }
            MenuCommand::CloseAll => {
                let ids = docs.iter().map(|d| d.id).collect();
                self.close_tabs(ids);
            }
            MenuCommand::CopyPath | MenuCommand::CopyRelativePath => {
                if let Some(path) = path {
                    self.copy_path(&path, command == MenuCommand::CopyRelativePath);
                }
            }
            MenuCommand::Reveal => {
                if let Some(path) = path {
                    self.reveal_in_file_manager(&path);
                }
            }
            MenuCommand::RevealInTree => {
                if let Some(path) = path {
                    self.show_files();
                    self.reveal_path(&path);
                }
            }
            _ => {}
        }
    }

    /// 右键点在第 `index` 个标签（文档 `id`）上时，菜单作用的标签范围：它在多选的选区里时是
    /// 选中的最左、最右一个的下标，否则只是它自己。
    pub(super) fn tab_span(&self, id: DocumentId, index: usize) -> (usize, usize) {
        let selection = self.tab_selection();
        if selection.len() < 2 || !selection.contains(&id) {
            return (index, index);
        }
        let indices = selection.iter().filter_map(|&id| self.editor.index_of(id));
        let (first, last) = indices.fold((index, index), |(a, b), i| (a.min(i), b.max(i)));
        (first, last)
    }

    /// 文件树里选中的文件（不含目录），按显示顺序。
    pub(in crate::app) fn tree_selected_files(&self) -> Vec<PathBuf> {
        let Some(ws) = self.editor.workspace.as_ref() else {
            return Vec::new();
        };
        ws.rows()
            .iter()
            .enumerate()
            .filter(|&(i, e)| e.kind == EntryKind::File && ws.is_selected(i))
            .map(|(_, e)| e.path.clone())
            .collect()
    }

    /// 在编辑区打开文件树里选中的各个文件（目录跳过），最后一个成为当前标签；焦点留在文件树。
    pub(in crate::app) fn open_tree_selection(&mut self) {
        let selection = self.editor.workspace.as_ref().map(|w| w.selection());
        let files = self.tree_selected_files();
        for path in &files {
            self.open_path(path);
        }
        if files.is_empty() {
            return;
        }
        // 打开时文件树会跟着定位到当前文件，选区随之收成一行；打开完再选回去。
        if let (Some(ws), Some(selection)) = (&mut self.editor.workspace, selection) {
            ws.select_paths(&selection);
        }
        self.focus = Focus::Tree;
    }

    /// Git 模块开着、`path` 在仓库里。
    pub(super) fn in_repo(&self, path: &Path) -> bool {
        self.git.root.as_deref().is_some_and(|root| {
            path.starts_with(root)
                || winc_config::files::canonicalize(path).is_ok_and(|real| real.starts_with(root))
        })
    }

    /// 返回 `path` 相对于已打开目录的路径；不在目录内或就是目录本身时返回 `None`。
    pub(in crate::app) fn relative_path(&self, path: &Path) -> Option<String> {
        let root = self.editor.root()?;
        let relative = path.strip_prefix(root).ok()?;
        (!relative.as_os_str().is_empty()).then(|| relative.to_string_lossy().into_owned())
    }

    /// 把 `path` 复制到剪贴板；`relative` 为 `true` 时复制相对于已打开目录的路径。
    pub(in crate::app) fn copy_path(&mut self, path: &Path, relative: bool) {
        self.copy_paths(&[path.to_path_buf()], relative);
    }

    /// 复制 `paths` 里的各个路径，一行一个；`relative` 为 `true` 时用相对于已打开目录的
    /// 路径。
    ///
    /// 相对路径时不在已打开目录里的跳过，一个都不剩时不复制。
    pub(in crate::app) fn copy_paths(&mut self, paths: &[PathBuf], relative: bool) {
        let lines: Vec<String> = if relative {
            paths.iter().filter_map(|p| self.relative_path(p)).collect()
        } else {
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        };
        if lines.is_empty() {
            return;
        }

        self.set_clipboard(lines.join("\n"), false);
        self.flash(if relative {
            t!("menu-copied-relative-path")
        } else {
            t!("menu-copied-path")
        });
    }

    /// 复制代码引用：路径加行号，如 `src/app.rs:12`；选了几行时是 `src/app.rs:12-18`，选区停在
    /// 下一行行首时不算那一行。
    ///
    /// `absolute` 为 `true`（`⌘L`）时用完整路径，否则用相对于已打开目录的路径，文件不在目录里时
    /// 仍用完整路径。
    pub(in crate::app) fn copy_reference(&mut self, absolute: bool) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            self.flash(t!("menu-copy-reference-unsaved"));
            return;
        };
        let text = doc.text();
        let s = doc.selection;
        let (start, end) = (s.anchor.min(s.head), s.anchor.max(s.head));
        let first = text.line_of(start);
        let mut last = text.line_of(end);
        if last > first && text.line_start(last) == end {
            last -= 1;
        }
        let path = (!absolute)
            .then(|| self.relative_path(&path))
            .flatten()
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let reference = if last > first {
            format!("{path}:{}-{}", first + 1, last + 1)
        } else {
            format!("{path}:{}", first + 1)
        };
        self.flash(t!("menu-copied-reference", reference = reference.as_str()));
        self.set_clipboard(reference, false);
    }

    /// 让全局搜索只搜索 `path` 所在的目录（`path` 是文件时取该文件所在的目录）。
    ///
    /// 把「包含」填成 `./相对路径`，并展开过滤条件。
    pub(in crate::app) fn find_in_folder(&mut self, path: &Path) {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let include = self
            .relative_path(dir)
            .map(|relative| format!("./{relative}"))
            .unwrap_or_default();

        if include != self.global.include.text() {
            self.global.include.set_text(&include);
        }
        self.global.details_open |= !include.is_empty();
        self.open_global_search(false);

        if !self.global.query.text().is_empty() {
            self.global_changed(true);
        }
    }

    /// 在文件树中就地改名：那一行变成输入框，填入原来的名字，并选中扩展名之前的部分；`↩` 确认，
    /// `Esc` 放弃。
    pub(in crate::app) fn start_rename(&mut self, path: &Path) {
        self.goto_input = None;
        let Some(ws) = &mut self.editor.workspace else {
            return;
        };
        let Some(index) = ws.row_of(path) else {
            return;
        };
        let entry = &ws.rows()[index];
        let name = winc_editor::segment_name(path).into_owned();
        let (kind, depth, expanded) = (entry.kind, entry.depth, entry.expanded);
        ws.select_segment(index, Some(path));

        let mut field = TextField::default();
        field.set_text(&name);
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 && kind == EntryKind::File => dot,
            _ => name.len(),
        };
        field.set_caret(0, false);
        field.set_caret(stem, true);

        self.name_input = Some(NameInput {
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            kind,
            index,
            depth,
            field,
            error: None,
            rename: Some((path.to_path_buf(), expanded)),
            contents: None,
        });
        self.focus = Focus::Prompt;
        self.field_preedit.clear();
        self.scroll_tree_to(index, false);
        self.relayout();
        self.request_redraw();
    }

    /// 提交改名 `entry`：文件树同步更新，已打开文档换成新路径（内容和修改状态不变）。
    ///
    /// 失败时输入行保留，并显示原因。
    pub(in crate::app) fn submit_rename(&mut self, mut entry: NameInput) {
        let Some((path, _)) = entry.rename.clone() else {
            return;
        };
        let input = entry.field.text().to_owned();
        // 先等待后台保存完成：保存的最后一步把临时文件改名为原路径，
        // 在此之前改名会使旧路径上重新出现该文件。
        self.wait_for_saves();

        let renamed = self
            .editor
            .workspace
            .as_mut()
            .map(|ws| ws.rename(&path, &input));

        match renamed {
            Some(Ok(target)) => {
                self.paths_moved(&path, &target);
                self.focus = Focus::Tree;
                if let Some(index) = self.editor.workspace.as_ref().and_then(|w| w.selected) {
                    self.scroll_tree_to(index, false);
                }
            }
            Some(Err(e)) => {
                entry.error = Some(e.to_string());
                self.name_input = Some(entry);
                self.focus = Focus::Prompt;
            }
            None => {}
        }

        self.relayout();
        self.request_redraw();
    }

    /// 删除前先确认（[`App::show_confirm`]），确认后移到废纸篓。
    pub(in crate::app) fn ask_delete(&mut self, path: &Path) {
        self.ask_delete_all(vec![path.to_path_buf()]);
    }

    /// 一次删除 `paths` 里的各项（文件树里多选时）。
    ///
    /// 根目录和目录以外的路径不删；一项都不剩时什么也不做。
    pub(in crate::app) fn ask_delete_all(&mut self, mut paths: Vec<PathBuf>) {
        let Some(root) = self.editor.root() else {
            return;
        };
        paths.retain(|path| path != root && path.starts_with(root));
        if paths.is_empty() {
            return;
        }

        if self.window.is_some() {
            let message = match paths.as_slice() {
                [path] => {
                    let name = winc_editor::display_name(path);
                    if path.is_dir() {
                        t!("common-trash-confirm-folder", name = name)
                    } else {
                        t!("common-trash-confirm-item", name = name)
                    }
                }
                many => t!("common-trash-confirm-items", count = many.len()),
            };
            let detail = t!("common-trash-confirm-detail");
            self.show_confirm(&message, detail, t!("common-trash-move"), move |ok| {
                ok.then(|| UserEvent::Delete(paths.clone()))
            });
            return;
        }

        // 没有窗口（测试中）时直接删除。
        self.delete_entries(&paths);
    }

    /// 把 `paths` 里的各项移到废纸篓。
    ///
    /// 其中文件对应的标签：没有修改的关闭，有修改的保留，内容仍在，保存时会重新创建文件。
    /// 某一项删除失败时提示原因，其余各项仍然删除。
    pub(in crate::app) fn delete_entries(&mut self, paths: &[PathBuf]) {
        // 先等待后台保存完成，否则已删除的文件会被保存重新写出。
        self.wait_for_saves();
        let mut deleted = Vec::new();
        for path in paths {
            match (self.trash)(path) {
                Ok(()) => deleted.push(path.clone()),
                Err(e) => {
                    let name = winc_editor::display_name(path);
                    let error = e.to_string();
                    self.notify(t!("common-trash-delete-failed", name = name, error = error));
                }
            }
        }
        if deleted.is_empty() {
            return;
        }

        let ids: Vec<DocumentId> = self
            .editor
            .documents
            .iter()
            .filter(|d| {
                !d.is_dirty()
                    && d.path
                        .as_ref()
                        .is_some_and(|p| deleted.iter().any(|q| p.starts_with(q)))
            })
            .map(|d| d.id)
            .collect();
        self.remove_tabs(&ids);

        // 选中原位置上的下一行，便于继续用 `⌘⌫` 连续删除。
        if let Some(ws) = &mut self.editor.workspace {
            let was = ws.rows().iter().position(|e| {
                deleted.contains(&e.path) || e.chain().iter().any(|p| deleted.contains(p))
            });
            ws.refresh();
            let count = ws.rows().len();
            ws.select(was.filter(|_| count > 0).map(|i| i.min(count - 1)));
            self.tree_motion = None;
            self.clamp_tree_scroll();
        }

        let message = match deleted.as_slice() {
            [path] => {
                let name = winc_editor::display_name(path);
                t!("common-trash-moved-item", name = name)
            }
            many => t!("common-trash-moved-items", count = many.len()),
        };
        self.picker_touched(deleted);
        self.files_changed_under_results();
        self.flash(message);
        self.relayout();
        self.request_redraw();
    }

    /// 返回文件树里选中的各项（去掉上级目录也选中了的，见 `Workspace::selection_roots`）；没有
    /// 打开目录或没有选中时为空。
    pub(in crate::app) fn tree_targets(&self) -> Vec<PathBuf> {
        self.editor
            .workspace
            .as_ref()
            .map(|ws| ws.selection_roots())
            .unwrap_or_default()
    }

    /// 返回快捷键作用的目标路径：焦点在文件树时为选中的条目，否则为当前文档的文件。
    ///
    /// 没有目标时返回 `None`。
    pub(in crate::app) fn shortcut_target(&self) -> Option<PathBuf> {
        if self.focus == Focus::Results {
            return self.global.selected.as_ref().map(|(path, _)| path.clone());
        }
        if self.focus == Focus::Tree {
            return self
                .editor
                .workspace
                .as_ref()?
                .current_path()
                .map(Path::to_path_buf);
        }
        self.editor.active().and_then(|d| d.path.clone())
    }
}

/// 文件树工具栏「⋯」菜单里的命令对应的工具。
fn tree_tool_of(command: MenuCommand) -> Option<TreeTool> {
    Some(match command {
        MenuCommand::NewFile => TreeTool::NewFile,
        MenuCommand::NewFolder => TreeTool::NewFolder,
        MenuCommand::TreeRefresh => TreeTool::Refresh,
        MenuCommand::TreeCollapseAll => TreeTool::CollapseAll,
        MenuCommand::TreeLocate => TreeTool::Locate,
        _ => return None,
    })
}
