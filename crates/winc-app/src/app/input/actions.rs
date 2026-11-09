//! 执行键位表中的命令。
//!
//! 快捷键和命令面板都经由 [`App::run_action`] 执行，每个命令只有一处实现。

use winc_core::transform::Case;
use winc_editor::{Command, EntryKind, FoldCommand};

use super::*;
use crate::field::TextField;
use crate::keymap::{Action, Context};
use crate::layout::{PanelButton, SearchButton, SettingsPage, SidebarView};

impl App {
    /// 执行 `action`，返回是否已处理。
    ///
    /// 当前上下文无法执行时（例如查找条未打开时的「替换这一处」）返回 `false`，
    /// 按键事件交还给焦点所在的控件继续处理。
    pub(in crate::app) fn run_action(&mut self, action: Action) -> bool {
        use Action::*;

        if !self.action_available(action) {
            return false;
        }
        if let Some(done) = self.diff_edit_action(action) {
            return done;
        }

        let editing = |app: &mut App, command: Command| {
            if app.editor.active.is_some() {
                app.focus = Focus::Editor;
                app.run(command);
            }
        };
        let panel = self.key_context() == Context::Panel;
        // 焦点在终端的输入框里：编辑用的快捷键作用于输入框。
        if matches!(self.focus, Focus::TerminalFind | Focus::TerminalRename)
            && matches!(action, Copy | Cut | Paste | SelectAll | Undo | Redo)
        {
            match action {
                Copy => self.copy_field(false),
                Cut => self.copy_field(true),
                Paste => self.paste(),
                SelectAll => self.edit_field(|field| {
                    field.select_all();
                    false
                }),
                Undo => self.edit_field(TextField::undo),
                _ => self.edit_field(TextField::redo),
            }
            return true;
        }
        // 焦点在终端里：复制、全选作用于终端的选区（粘贴见 `paste`）。
        if self.focus == Focus::Terminal {
            match action {
                Copy => {
                    self.terminal_copy();
                    return true;
                }
                SelectAll => {
                    self.terminal_select_all();
                    return true;
                }
                _ => {}
            }
        }

        match action {
            ShowCommands => self.toggle_palette(),
            QuickOpen => {
                if self.picker_open
                    && !self.picker_lists_commands()
                    && !self.picker_lists_symbols()
                    && !self.picker_folders
                {
                    self.close_picker();
                } else {
                    self.open_picker();
                }
            }
            // 和 `⌘P` 一样，已经在列符号时再按一次关掉。
            GoToSymbol if self.picker_open && self.picker_lists_symbols() => self.close_picker(),
            GoToSymbol => self.open_symbols(),
            GoToLine => self.open_goto(),
            GoToDefinition => self.go_to_definition(),
            FindReferences => self.find_references(),
            // 有日志标签且可以后退时先在日志里后退；日志里已经找不到的提交（例如被跳过的
            // 父提交）直接跳过，继续找回更早的记录。
            GoBack if self.log_navigate(false) => {}
            GoForward if self.log_navigate(true) => {}
            GoBack => self.navigate(false),
            GoForward => self.navigate(true),
            Settings => self.open_settings(),
            KeyboardShortcuts => {
                self.open_settings();
                self.settings_show_page(SettingsPage::Keys);
            }
            RecordKeys if self.settings.open => self.toggle_recording(),
            RecordKeys => return false,

            NewFile => self.new_file(),
            NewWindow => self.new_window(None),
            OpenFile => self.pick(Pick::Open),
            OpenFolder => self.pick(Pick::OpenFolder),
            OpenRecent => self.open_recent_folders(),
            Save => self.save_active(),
            SaveAs => {
                if let Some(id) = self.editor.active().map(|d| d.id) {
                    self.pick(Pick::SaveAs(id));
                }
            }
            CloseTab => {
                if let Some(index) = self.editor.active {
                    self.close_tab(index);
                }
            }
            CloseOtherTabs => {
                if let Some(id) = self.editor.active().map(|d| d.id) {
                    let others = self.editor.documents.iter().map(|d| d.id);
                    self.close_tabs(others.filter(|&other| other != id).collect());
                }
            }
            CloseAllTabs => {
                let all = self.editor.documents.iter().map(|d| d.id).collect();
                self.close_tabs(all);
            }
            ReopenClosed => self.reopen_closed(),
            NextTab => self.cycle_tab(true),
            PreviousTab => self.cycle_tab(false),
            CloseFolder => self.close_folder(),
            // 只有一个窗口，关闭窗口就是退出，未保存的内容按退出的流程备份或询问。
            CloseWindow => self.request_close_window(),
            CopyPath | CopyRelativePath | RevealInFinder => match self.shortcut_target() {
                Some(path) if action == RevealInFinder => self.reveal_in_file_manager(&path),
                Some(path) => self.copy_path(&path, action == CopyRelativePath),
                None => self.flash(winc_i18n::t!("cmd-no-selected-file")),
            },
            CopyKeyPath => self.copy_key_path(),
            CopyReference => self.copy_reference(false),
            CopyAbsoluteReference => self.copy_reference(true),
            OpenInTerminal => {
                let target = self.shortcut_target();
                self.open_in_terminal(target.as_deref());
            }
            CopyActivePath | RevealActiveFile => {
                match self.editor.active().and_then(|d| d.path.clone()) {
                    Some(path) if action == RevealActiveFile => self.reveal_in_file_manager(&path),
                    Some(path) => self.copy_path(&path, false),
                    None => self.flash(winc_i18n::t!("cmd-no-file-for-tab")),
                }
            }
            Quit => self.request_quit(),

            Undo => editing(self, Command::Undo),
            Redo => editing(self, Command::Redo),
            // 文件树中剪切选中的文件或目录：粘贴到文件树时移过去。
            Cut if self.focus == Focus::Tree => {
                let paths = self.tree_targets();
                if paths.is_empty() {
                    return false;
                }
                self.cut_entries(&paths);
            }
            Cut => self.copy(true),
            // 文件树中复制选中的文件或目录本身，可以粘贴到系统文件管理器、文件树或
            // Markdown 文档中。
            Copy if self.focus == Focus::Tree => {
                let paths = self.tree_targets();
                if paths.is_empty() {
                    return false;
                }
                self.copy_entries(&paths);
            }
            Copy => self.copy(false),
            Paste => self.paste(),
            SelectAll if self.markdown_focused() => self.markdown_select_all(),
            SelectAll if self.pdf_focused() => self.pdf_select_all(),
            SelectAll if self.table_focused() => self.table_select_all(),
            SelectAll if self.focus == Focus::Tree => {
                if let Some(ws) = &mut self.editor.workspace {
                    ws.select_all();
                }
            }
            SelectAll => editing(self, Command::SelectAll),
            ToggleComment => editing(self, Command::ToggleComment),
            DeleteLine => editing(self, Command::DeleteLine),
            DeleteAllLeft => editing(self, Command::DeleteToLineStart),
            IndentLines => editing(self, Command::IndentLines),
            OutdentLines => editing(self, Command::Outdent),
            MoveLinesUp => editing(self, Command::MoveLines { down: false }),
            MoveLinesDown => editing(self, Command::MoveLines { down: true }),
            CopyLinesUp => editing(self, Command::CopyLines { down: false }),
            CopyLinesDown => editing(self, Command::CopyLines { down: true }),
            JoinLines => editing(self, Command::JoinLines),
            ExpandSelection => editing(self, Command::ExpandSelection),
            ShrinkSelection => editing(self, Command::ShrinkSelection),
            AddCursorAbove => self.add_cursor_vertically(false),
            AddCursorBelow => self.add_cursor_vertically(true),
            AddNextOccurrence => self.add_next_occurrence(false),
            SkipToNextOccurrence => self.add_next_occurrence(true),
            SelectAllOccurrences => self.select_all_occurrences(),
            // 只有一个光标时不处理，`Esc` 交给编辑区（关闭查找条、收起选区）。
            // 表格上的 `Esc`：只留当前格。
            RemoveSecondaryCursors if self.table_focused() => {
                return self.table_key(
                    &winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape),
                    false,
                );
            }
            RemoveSecondaryCursors => return self.remove_secondary_cursors(),
            AddCursorsToLineEnds => self.add_cursors_to_line_ends(),
            CursorUndo => self.step_cursors(false),
            CursorRedo => self.step_cursors(true),
            ColumnSelectUp => self.extend_columns(0, -1),
            ColumnSelectDown => self.extend_columns(0, 1),
            ColumnSelectLeft => self.extend_columns(-1, 0),
            ColumnSelectRight => self.extend_columns(1, 0),
            // 查找条没开着时不处理，`⌥↩` 交给编辑区（换行）。
            SelectAllMatches => return self.select_all_matches(),
            JumpToBracket => editing(self, Command::JumpToBracket),
            TransformToUppercase => editing(self, Command::ChangeCase(Case::Upper)),
            TransformToLowercase => editing(self, Command::ChangeCase(Case::Lower)),
            TransformToTitlecase => editing(self, Command::ChangeCase(Case::Title)),
            SortLinesAscending => editing(self, Command::SortLines { descending: false }),
            SortLinesDescending => editing(self, Command::SortLines { descending: true }),
            TrimTrailingWhitespace => editing(self, Command::TrimTrailingWhitespace),
            IndentUsingSpaces => self.use_indent(false),
            IndentUsingTabs => self.use_indent(true),
            ConvertIndentationToSpaces => self.convert_indentation(false),
            ConvertIndentationToTabs => self.convert_indentation(true),
            DetectIndentation => self.detect_indentation(),
            ChangeEndOfLine => self.open_line_ending_menu(),
            ChangeLanguageMode => self.open_language_menu(),
            Fold => self.run_fold(FoldCommand::Fold),
            Unfold => self.run_fold(FoldCommand::Unfold),
            ToggleFold => self.run_fold(FoldCommand::Toggle),
            FoldRecursively => self.run_fold(FoldCommand::FoldRecursively),
            UnfoldRecursively => self.run_fold(FoldCommand::UnfoldRecursively),
            FoldAll => self.run_fold(FoldCommand::FoldAll),
            UnfoldAll => self.run_fold(FoldCommand::UnfoldAll),
            FoldLevel1 => self.run_fold(FoldCommand::FoldLevel(1)),
            FoldLevel2 => self.run_fold(FoldCommand::FoldLevel(2)),
            FoldLevel3 => self.run_fold(FoldCommand::FoldLevel(3)),
            FoldLevel4 => self.run_fold(FoldCommand::FoldLevel(4)),
            FoldLevel5 => self.run_fold(FoldCommand::FoldLevel(5)),
            FoldLevel6 => self.run_fold(FoldCommand::FoldLevel(6)),
            FoldLevel7 => self.run_fold(FoldCommand::FoldLevel(7)),
            ToggleBookmark => self.toggle_bookmark(),
            NextBookmark => self.cycle_bookmark(true),
            PreviousBookmark => self.cycle_bookmark(false),
            ListBookmarks => self.list_bookmarks(),
            ClearBookmarks => self.clear_bookmarks(),
            Format => self.format_active(),
            FormatSelection => {
                if self.editor.active().is_some_and(|d| d.selection.is_empty()) {
                    self.flash(winc_i18n::t!("cmd-no-selection"));
                } else {
                    self.format_selection();
                }
            }
            Fix => self.fix_active(),

            // 日志标签里 `⌘F` 是筛选输入框。
            Find if self.log_active() => {
                self.focus = Focus::LogFilter;
                self.log.filter.select_all();
                self.request_redraw();
            }
            Find => self.open_search(false),
            Replace => self.open_search(true),
            FindNext => self.find_again(false),
            FindPrevious => self.find_again(true),
            ReplaceOne if self.search_open => self.replace_current(),
            ReplaceAll if self.search_open => self.replace_all(),
            ReplaceOne | ReplaceAll => return false,
            ToggleCase | ToggleWord | ToggleRegex if panel => {
                self.press_panel_button(match action {
                    ToggleCase => PanelButton::Case,
                    ToggleWord => PanelButton::Word,
                    _ => PanelButton::Regex,
                })
            }
            ToggleCase | ToggleWord | ToggleRegex if self.search_open => {
                self.press_search_button(match action {
                    ToggleCase => SearchButton::Case,
                    ToggleWord => SearchButton::Word,
                    _ => SearchButton::Regex,
                })
            }
            ToggleCase | ToggleWord | ToggleRegex => return false,
            FilterLines => {
                if !self.search_open {
                    self.open_search(false);
                }
                self.press_search_button(SearchButton::Filter);
            }
            FindInFiles => self.open_global_search(false),
            ReplaceInFiles => self.open_global_search(true),
            ReplaceAllInFiles => self.ask_replace_all(),
            NextResult => self.next_result(false),
            PreviousResult => self.next_result(true),

            RenameFile | DeleteFile | FindInFolder => {
                let Some(path) = self.shortcut_target() else {
                    return false;
                };
                match action {
                    RenameFile => self.start_rename(&path),
                    // 文件树里删除选中的各项。
                    DeleteFile if self.focus == Focus::Tree => {
                        self.ask_delete_all(self.tree_targets());
                    }
                    DeleteFile => self.ask_delete(&path),
                    _ => self.find_in_folder(&path),
                }
            }
            OpenSelected => self.open_selected(false),
            PreviewSelected => self.open_selected(true),

            ToggleSidebar => self.toggle_sidebar(),
            FocusSidebar => self.focus_sidebar(),
            ShowFiles => self.show_files(),
            IncreaseFontSize => self.step_font_size(Some(true)),
            DecreaseFontSize => self.step_font_size(Some(false)),
            ResetFontSize => self.step_font_size(None),
            ZoomIn => self.step_zoom(Some(true)),
            ZoomOut => self.step_zoom(Some(false)),
            ResetZoom => self.step_zoom(None),
            ThemeAuto => self.set_theme_choice(ThemeChoice::Auto),
            ThemeLight => self.set_theme_choice(ThemeChoice::Light),
            ThemeDark => self.set_theme_choice(ThemeChoice::Dark),
            ToggleStickyScroll => self.settings_toggle(crate::layout::Setting::StickyScroll),
            TogglePreview => self.toggle_preview(),
            ToggleWordWrap => self.settings_toggle(crate::layout::Setting::WordWrap),
            ToggleLineNumbers => self.settings_toggle(crate::layout::Setting::LineNumbers),
            ToggleStatusBar => self.settings_toggle(crate::layout::Setting::StatusBar),
            // 其他选项都按「全部显示」切换；再次切换时不显示。
            ToggleWhitespace => {
                use winc_config::RenderWhitespace::{All, None};
                let all = self.config.render_whitespace == Some(All);
                self.set_render_whitespace(if all { None } else { All });
            }
            ToggleMinimap => self.settings_toggle(crate::layout::Setting::Minimap),
            // 差异标签里在两侧的改动之间跳；普通文件里在行号栏标出的改动之间跳。
            NextDifference | PreviousDifference => {
                let forward = action == NextDifference;
                if self.editor.active().is_some_and(|d| d.diff.is_some()) {
                    self.jump_difference(forward);
                } else if self.jump_conflict(forward) {
                    // 有合并冲突的文件里先在冲突之间跳。
                } else if !self.jump_change(forward) {
                    return false;
                }
            }
            NextDiffFile | PreviousDiffFile => {
                if !self.editor.active().is_some_and(|d| d.diff.is_some()) {
                    return false;
                }
                self.step_diff_file(action == NextDiffFile);
            }
            ToggleDiffLayout => {
                if !self.diff_active() {
                    return false;
                }
                self.settings_toggle(crate::layout::Setting::DiffInline);
            }
            GitShowChanges => self.git_show_changes(),
            GitShowLog => self.git_show_log(),
            GitShowOutput => self.show_git_output(),
            GitClone => self.open_clone_dialog(),
            GitInit => self.git_init(),
            GitSwitchRepository => self.switch_location(),
            GitSwitchWorktree => self.switch_location(),
            GitNewWorktree => self.open_worktree_dialog(),
            // 这几个命令要在仓库里：不在时说明，不弹空的选择器、不跑注定失败的 `git`。
            GitSwitchBranch | GitNewBranch | GitStash | GitSync | GitFetch | GitPull
            | GitPullRebase | GitPush
                if self.git.root.is_none() =>
            {
                self.flash(winc_i18n::t!("git-not-a-repository"));
            }
            GitSwitchBranch => self.switch_branch_from_command(),
            GitNewBranch => self.open_branch_dialog(None, None),
            GitStash => self.open_stash_prompt(),
            GitSync => self.git_sync(),
            GitFetch | GitPull | GitPullRebase | GitPush => {
                use crate::app::input::context_menu::MenuCommand;
                let command = match action {
                    GitFetch => MenuCommand::GitFetch,
                    GitPull => MenuCommand::GitPull,
                    GitPullRebase => MenuCommand::GitPullRebase,
                    _ => MenuCommand::GitPush,
                };
                self.run_branch_command(command);
            }
            GitFileHistory => self.git_current_file_history(),
            GitBlame => self.toggle_blame(),
            GitRevertChange if self.editor.active().is_some_and(|d| d.diff.is_some()) => {
                self.revert_diff_hunk_here();
            }
            GitRevertChange => self.revert_change_at_caret(),
            ShowCommit => self.show_commit(),
            ShowOutline => self.show_outline(),
            ToggleTerminal => self.toggle_terminal(),
            RunConfiguration => self.run_current_config(),
            StopConfiguration => self.stop_current_config(),
            EditRunConfigurations => self.edit_run_configs(),
            ClearTerminal => self.clear_terminal(),
            NewTerminal => self.new_terminal_tab(),
            CloseTerminal => self.close_terminal_tab(self.terminal.active),
            NextTerminal => self.cycle_terminal_tab(true),
            PreviousTerminal => self.cycle_terminal_tab(false),
            // 焦点在查找框里时关闭；否则打开，或者回到开着的查找框。
            FindInTerminal if self.focus == Focus::TerminalFind => self.close_terminal_find(),
            FindInTerminal if self.terminal.tab().is_some() => self.open_terminal_find(),
            FindInTerminal => return false,
            MaximizeTerminal => self.toggle_terminal_maximized(),
            ToggleTerminalPosition => self.toggle_terminal_position(),
            InstallShellCommand => self.install_shell_command(),
            UninstallShellCommand => self.uninstall_shell_command(),
            RenameTerminal => self.open_terminal_rename(self.terminal.active),
            PreviousCommand => self.jump_terminal_command(false),
            NextCommand => self.jump_terminal_command(true),
            FoldOutput => self.fold_terminal_output(true),
            UnfoldOutput => self.fold_terminal_output(false),
            NextError => self.step_terminal_error(true),
            PreviousError => self.step_terminal_error(false),
            SendToTerminal => self.send_to_terminal(),
        }
        self.request_redraw();
        true
    }

    /// 命令当前是否可用：属于某个模块的命令只在模块开着时可用。
    pub(in crate::app) fn action_available(&self, action: Action) -> bool {
        match action {
            Action::RunConfiguration if self.editor.root().is_none() => return false,
            // 只有打开的目录还不是仓库时才能初始化。
            Action::GitInit if self.editor.root().is_none() || self.git.root.is_some() => {
                return false;
            }
            Action::StopConfiguration if !self.run_config_running() => return false,
            Action::EditRunConfigurations if self.editor.root().is_none() => return false,
            // Windows 安装包、deb 包已经把 `winc` 放进 `PATH`，没有要安装的。
            Action::InstallShellCommand | Action::UninstallShellCommand
                if crate::shell_command::placement().is_none() =>
            {
                return false;
            }
            _ => {}
        }
        match action.module() {
            Some(crate::keymap::Module::Git) => self.git_enabled(),
            Some(crate::keymap::Module::Terminal) => self.terminal_enabled(),
            None => true,
        }
    }

    /// 打开文件树中选中的条目：目录切换展开状态，文件在编辑区打开，选中了多项时打开其中的
    /// 各个文件。
    ///
    /// `keep_focus` 为真时焦点保留在文件树，便于继续用方向键选择下一项。
    fn open_selected(&mut self, keep_focus: bool) {
        let Some(ws) = &self.editor.workspace else {
            return;
        };
        if ws.multi_selected() {
            self.open_tree_selection();
            return;
        }
        let Some(index) = ws.selected else {
            return;
        };
        let entry = &ws.rows()[index];
        if entry.kind == EntryKind::Dir {
            self.set_dir_expanded(index, None);
            return;
        }
        let path = entry.path.clone();
        self.open_path(&path);
        if keep_focus {
            self.focus = Focus::Tree;
        }
    }

    /// 将焦点移到侧栏（`⌘0`）：侧栏已收起时先展开，焦点落在当前显示的视图上。
    fn focus_sidebar(&mut self) {
        if !self.panel_visible {
            self.set_sidebar(true);
        }
        self.focus = match self.sidebar_view {
            SidebarView::Files => Focus::Tree,
            SidebarView::Search => Focus::GlobalQuery,
            SidebarView::Commit => self.commit_view_focus(),
        };
    }
}
