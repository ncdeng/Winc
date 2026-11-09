//! 键盘和输入法。
//!
//! 设置浮层打开时只处理浮层自己的按键；否则先匹配键位表中的快捷键，再按焦点分发给编辑区、
//! 各输入框、文件树和搜索结果。

use std::time::Instant;

use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, KeyEvent};
use winit::keyboard::{Key, KeyLocation, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use winc_core::Selection;
use winc_editor::{Command, EntryKind, segment_name};

use crate::app::{App, Focus, ResultRow};
use crate::field::{TextField, Unit};
use crate::keymap::{self, Action, Context, Lookup, Os, Stroke};
use crate::theme::KEYS_ROW;

/// 文件树键入跳转的重置间隔（毫秒，经验值）。
///
/// 两次键入相隔超过该值时，重新从第一个字符开始匹配。该间隔足以连续键入文件名的前几个字符，
/// 停顿后的新一次查找也不会接在旧字符之后。
const TYPE_RESET: std::time::Duration = std::time::Duration::from_millis(800);

impl App {
    /// 处理键盘按下事件 `event`。
    ///
    /// 要退出时置 `quit_requested`，由应用层结束事件循环。
    ///
    /// 处理顺序：取消文件树拖动、关闭「⋯」菜单、设置浮层（模态）、键位表中的快捷键，
    /// 最后按焦点分发。
    pub(in crate::app) fn on_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        let m = self.modifiers;
        let cmd = if cfg!(target_os = "macos") {
            m.super_key()
        } else {
            m.control_key()
        };
        let base = match event.key_without_modifiers() {
            Key::Character(s) => s.to_lowercase(),
            _ => String::new(),
        };

        self.reset_blink();
        self.request_redraw();

        if matches!(event.logical_key, Key::Named(NamedKey::Escape))
            && self.tree_dragging().is_some()
        {
            self.cancel_tree_drag();
            return;
        }

        // 「⋯」菜单打开时，按任意键（单独按修饰键除外）都会先关闭菜单；按的是 `Esc` 时只关闭
        // 菜单，不再继续处理。
        let modifier = matches!(
            event.logical_key,
            Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
        );
        // 名字旁边的提示气泡在按下任意键（单独按修饰键除外）时淡出。
        if !modifier {
            self.dismiss_hint();
        }
        // 窗口内的提示框、菜单打开时，按键只给它们。
        if self.alert_key(&event.logical_key, cmd) || self.popup_key(event) {
            return;
        }
        // 设置浮层是模态的：打开期间只处理它自己的按键，其他快捷键不会传到下层。
        if self.settings.open {
            self.on_key_settings(event, &base, cmd);
            return;
        }
        // 新建工作树对话框也是模态窗口。
        if self.form_dialog.is_some() {
            self.on_key_dialog(event, &base, cmd);
            return;
        }

        // 全屏查看图时，关闭、缩放等按键先交给查看器处理。
        if self.diagram_view_key(&event.logical_key, &base, cmd) {
            return;
        }

        // 快捷键都定义在键位表中，先查表；不是快捷键的按键（打字、方向键、回车）再交给
        // 焦点所在的部件。
        let stroke = self.stroke_of(
            &event.logical_key,
            &base,
            event.physical_key,
            event.location,
        );
        if let Some(stroke) = stroke
            && self.on_shortcut(stroke)
        {
            // 快捷键把焦点挪走了：标签上正在改的名字照输入的算。
            if self.focus != Focus::TerminalRename {
                self.finish_terminal_rename(true);
            }
            return;
        }

        match self.focus {
            Focus::Editor => self.on_key_editor(event, cmd),
            Focus::Search | Focus::Replace => self.on_key_search(event, &base, cmd),
            Focus::Prompt => self.on_key_prompt(event, &base, cmd),
            Focus::Picker => self.on_key_picker(event, &base, cmd),
            // 设置浮层打开时已在上方处理，不会执行到这里。
            Focus::Setting
            | Focus::Settings
            | Focus::KeySearch
            | Focus::SettingsSearch
            | Focus::Dialog => {}
            Focus::Tree => self.on_key_tree(&event.logical_key),
            Focus::GlobalQuery
            | Focus::GlobalReplace
            | Focus::GlobalInclude
            | Focus::GlobalExclude => self.on_key_panel_field(event, &base, cmd),
            Focus::Results => self.on_key_results(event),
            Focus::CommitMessage => self.on_key_commit_message(event, &base, cmd),
            Focus::CommitList => self.on_key_commit_list(event),
            Focus::Outline => self.outline_key(&event.logical_key),
            Focus::LogFilter => self.on_key_log_filter(event, &base, cmd),
            Focus::Terminal => self.on_key_terminal(event),
            // 这两个输入框使用终端的键位表，Windows、Linux 的 Ctrl+V 不在表中，在此处理。
            Focus::TerminalFind | Focus::TerminalRename if cmd && base == "v" => self.paste(),
            Focus::TerminalFind => self.on_key_terminal_find(event, &base, cmd),
            Focus::TerminalRename => self.on_key_terminal_rename(event, &base, cmd),
        }
    }

    /// 返回当前焦点对应的键位上下文，键位表按它划分适用范围。
    pub(super) fn key_context(&self) -> Context {
        match self.focus {
            Focus::Editor => Context::Editor,
            Focus::Search => Context::Find,
            Focus::Replace => Context::Replace,
            Focus::Tree => Context::Tree,
            // 终端的查找框、标签上改名字的输入框也按终端的键位：`⌘W` 关的是终端标签，`⌘F` 回到
            // 终端的查找，不进入编辑区（编辑用的快捷键见 `run_action`，留给输入框）。
            Focus::Terminal | Focus::TerminalFind | Focus::TerminalRename => Context::Terminal,
            focus if focus.in_panel() => Context::Panel,
            _ => Context::Other,
        }
    }

    /// 将一次按键转换为 [`Stroke`]，参数含义见 [`Stroke::from_event`]。
    ///
    /// 输入法正在组字时，它接管的按键属于组字，不作为快捷键。
    pub(in crate::app) fn stroke_of(
        &self,
        key: &Key,
        base: &str,
        physical: PhysicalKey,
        location: KeyLocation,
    ) -> Option<Stroke> {
        let composing = !self.preedit.is_empty() || !self.field_preedit.is_empty();
        if composing && *key == Key::Named(NamedKey::Process) {
            return None;
        }
        Stroke::from_event(key, base, physical, location, self.modifiers)
    }

    /// 是否已作为快捷键处理按键 `stroke`。
    ///
    /// 是快捷键则执行并返回 `true`；是两步组合键的第一步则等待第二步，同样返回 `true`。
    /// 不是快捷键时返回 `false`，按键交给焦点所在的部件。
    pub(in crate::app) fn on_shortcut(&mut self, stroke: Stroke) -> bool {
        let table = keymap::current();
        let context = self.key_context();

        if let Some(first) = self.pending_chord.take() {
            self.status_message = None;
            if stroke.key == keymap::KeyCode::Escape {
                return true;
            }
            match keymap::lookup(table, stroke, Some(first), context) {
                Lookup::Run(actions)
                    if actions
                        .iter()
                        .any(|&a| self.run_action(self.keyboard_action(a))) => {}
                _ => {
                    let keys =
                        format!("{} {}", first.label(Os::CURRENT), stroke.label(Os::CURRENT));
                    self.flash(winc_i18n::t!("cmd-chord-unassigned", keys = keys));
                }
            }
            return true;
        }

        // 在提交列表里点文件后焦点留在列表上，差异显示在编辑区：在差异里导航的快捷键也要生效，焦点
        // 不移动，上下键仍然在列表里选文件。
        if self.focus == Focus::CommitList
            && self.diff_active()
            && let Lookup::Run(actions) = keymap::lookup(table, stroke, None, Context::Editor)
            && let Some(&action) = actions.iter().find(|a| a.navigates_diff())
        {
            return self.run_action(action);
        }

        match keymap::lookup(table, stroke, None, context) {
            Lookup::Run(actions) => actions
                .into_iter()
                .any(|a| self.run_action(self.keyboard_action(a))),
            Lookup::Chord => {
                self.pending_chord = Some(stroke);
                let key = stroke.label(Os::CURRENT);
                // 状态栏提示在第二步到来之前一直保留，不自动消失（`status_until` 为 `None`）。
                self.status_message = Some(winc_i18n::t!("cmd-chord-pending", key = key));
                self.status_until = None;
                self.request_redraw();
                true
            }
            Lookup::None => false,
        }
    }

    /// 返回按键触发 `action` 时实际执行的命令。
    ///
    /// macOS 上没有打开的标签时，「关闭标签页」的快捷键改为关闭窗口：关掉最后一个标签后再按
    /// 一次 `⌘W` 关闭窗口。只对按键生效，在命令面板里选「关闭标签页」不会关闭窗口。
    fn keyboard_action(&self, action: Action) -> Action {
        if cfg!(target_os = "macos")
            && action == Action::CloseTab
            && self.editor.documents.is_empty()
        {
            Action::CloseWindow
        } else {
            action
        }
    }

    /// 处理焦点在编辑区时的按键：移动光标、翻页、换行、缩进、删除和键入。
    pub(in crate::app) fn on_key_editor(&mut self, event: &KeyEvent, cmd: bool) {
        if self.editor.active.is_none() {
            return;
        }
        if self.log_active() {
            self.log_key(&event.logical_key);
            return;
        }
        if self.editor.active().is_some_and(|d| d.diff.is_some()) {
            if !self.diff_edit_key(event, cmd) {
                self.diff_key(&event.logical_key, cmd);
            }
            return;
        }
        if self.markdown_key(&event.logical_key, cmd)
            || self.table_key(&event.logical_key, cmd)
            || self.pdf_key(&event.logical_key)
        {
            return;
        }
        // 改动卡片开着时 `Esc` 先关卡片。
        if self.changes.popup.is_some() && event.logical_key == Key::Named(NamedKey::Escape) {
            self.changes.popup = None;
            self.request_redraw();
            return;
        }

        let shift = self.modifiers.shift_key();
        // 带 `⌘` / `Ctrl` 但键位表中没有的组合不处理，不会被当作输入或移动。
        let control = cmd || (cfg!(target_os = "macos") && self.modifiers.control_key());

        // macOS：`⌘←` / `⌘→` 到行首行尾，`⌘↑` / `⌘↓` 到文件首尾，`⌥←` / `⌥→` 按词移动；
        // 其他平台用 `Ctrl←` / `Ctrl→` 按词移动。
        let extend = shift;
        let word = if cfg!(target_os = "macos") {
            self.modifiers.alt_key()
        } else {
            cmd
        };
        let jump = cfg!(target_os = "macos") && cmd;

        let command = match &event.logical_key {
            Key::Named(NamedKey::ArrowLeft) if jump => Command::LineStart { extend },
            Key::Named(NamedKey::ArrowRight) if jump => Command::LineEnd { extend },
            Key::Named(NamedKey::ArrowUp) if jump => Command::DocStart { extend },
            Key::Named(NamedKey::ArrowDown) if jump => Command::DocEnd { extend },
            Key::Named(NamedKey::ArrowLeft) => Command::MoveLeft { word, extend },
            Key::Named(NamedKey::ArrowRight) => Command::MoveRight { word, extend },
            // 移动行、复制行在键位表中定义；没有匹配的 `⌥↑` / `⌥↓` 组合不处理。
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if self.modifiers.alt_key() => {
                return;
            }
            Key::Named(NamedKey::ArrowUp) => Command::MoveUp { extend },
            Key::Named(NamedKey::ArrowDown) => Command::MoveDown { extend },
            Key::Named(NamedKey::PageUp) => Command::PageUp { extend },
            Key::Named(NamedKey::PageDown) => Command::PageDown { extend },
            Key::Named(NamedKey::Home) if cmd => Command::DocStart { extend },
            Key::Named(NamedKey::End) if cmd => Command::DocEnd { extend },
            Key::Named(NamedKey::Home) => Command::LineStart { extend },
            Key::Named(NamedKey::End) => Command::LineEnd { extend },
            Key::Named(NamedKey::Enter | NamedKey::Tab) if control => return,
            Key::Named(NamedKey::Enter) => Command::Newline,
            Key::Named(NamedKey::Tab) if shift => Command::Outdent,
            Key::Named(NamedKey::Tab) => Command::Indent,
            Key::Named(NamedKey::Backspace) => Command::Backspace { word },
            Key::Named(NamedKey::Delete) => Command::Delete { word },
            Key::Named(NamedKey::Escape) => {
                if self.search_open {
                    self.close_search();
                } else if let Some(doc) = self.editor.active_mut() {
                    doc.selection = Selection::point(doc.selection.head);
                }
                return;
            }
            _ => {
                // 普通字符输入；输入法提交的文字由 `Ime::Commit` 处理。
                if !control
                    && let Some(text) = &event.text
                    && !text.chars().any(char::is_control)
                {
                    self.run(Command::Insert(text.to_string()));
                }
                return;
            }
        };

        self.run(command);
    }

    /// 处理查找条（查找、替换输入框）中的按键。
    ///
    /// - 查找输入框中，`↩` 查找下一个，`⇧↩` 查找上一个；替换输入框中，`↩` 替换当前匹配
    ///   （全部替换在键位表中定义）。
    /// - `↑` / `↓`（或 `⌥↑` / `⌥↓`）翻阅历史，翻过最新一条时清空；`⇧↑` / `⇧↓` 选到开头或末尾。
    /// - `Tab` 在两个输入框之间切换。其余按键交给输入框。
    fn on_key_search(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let shift = self.modifiers.shift_key();
        let replacing = self.focus == Focus::Replace;

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_search(),
            Key::Named(NamedKey::Enter) if replacing => self.replace_current(),
            Key::Named(NamedKey::Enter) => self.find_next(shift),
            Key::Named(NamedKey::ArrowUp) if !shift => self.edit_field(|f| f.recall(true)),
            Key::Named(NamedKey::ArrowDown) if !shift => self.edit_field(|f| f.recall(false)),
            Key::Named(NamedKey::Tab) if self.replace_open => {
                self.focus = if replacing {
                    Focus::Search
                } else {
                    Focus::Replace
                };
            }
            Key::Named(NamedKey::Tab) => self.focus = Focus::Editor,
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 处理侧栏搜索面板四个输入框中的按键。
    fn on_key_panel_field(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let shift = self.modifiers.shift_key();

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.leave_panel(),
            // 替换输入框中 `↩` 执行替换；其余输入框中 `↩` 跳到下一处，`⇧↩` 跳到上一处。
            Key::Named(NamedKey::Enter) => {
                if let Some(field) = self.focused_field() {
                    field.remember();
                }

                match self.focus {
                    Focus::GlobalReplace => self.replace_from_field(),
                    _ => self.step_from_field(shift),
                }
            }
            Key::Named(NamedKey::ArrowUp) if !shift => self.edit_field(|f| f.recall(true)),
            Key::Named(NamedKey::ArrowDown) if !shift => self.edit_field(|f| f.recall(false)),
            Key::Named(NamedKey::Tab) => self.cycle_panel_focus(shift),
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 处理搜索结果列表中的按键。
    ///
    /// `↑` `↓` 移动选中项，`Home` `End` 选中第一行、最后一行，`PageUp` `PageDown` 翻一页，
    /// `←` `→` 折叠或展开文件，回车打开并将焦点移到编辑区，
    /// 空格打开但焦点留在列表，`⌫` 从结果中移除。
    fn on_key_results(&mut self, event: &KeyEvent) {
        self.results_key(&event.logical_key);
    }

    /// 执行搜索结果列表中按键 `key` 的操作，见 [`App::on_key_results`]。
    pub(in crate::app) fn results_key(&mut self, key: &Key) {
        let shift = self.modifiers.shift_key();
        let selected = self.global.selected_row();
        let row = selected.and_then(|i| self.global.rows().get(i).copied());

        match key {
            Key::Named(NamedKey::Escape) => self.focus = Focus::GlobalQuery,
            Key::Named(NamedKey::ArrowUp) => self.step_result(-1, false),
            Key::Named(NamedKey::ArrowDown) => self.step_result(1, false),
            Key::Named(NamedKey::Home) => self.jump_result(0),
            Key::Named(NamedKey::End) => self.jump_result(usize::MAX),
            Key::Named(NamedKey::PageDown) => {
                let page = self.result_page();
                self.jump_result(selected.map_or(0, |i| i.saturating_add(page)));
            }
            Key::Named(NamedKey::PageUp) => {
                let page = self.result_page();
                self.jump_result(selected.map_or(0, |i| i.saturating_sub(page)));
            }
            Key::Named(NamedKey::ArrowLeft) => match row {
                Some(ResultRow::File(f)) => self.set_result_collapsed(f, true),
                // 在匹配上按 `←` 回到该匹配所在文件的那一行。
                Some(ResultRow::Match(f, _)) => {
                    let path = self.global.engine.results[f].path.clone();
                    self.global.selected = Some((path, None));
                }
                None => {}
            },
            Key::Named(NamedKey::ArrowRight) => {
                if let Some(ResultRow::File(f)) = row {
                    self.set_result_collapsed(f, false);
                }
            }
            Key::Named(NamedKey::Enter | NamedKey::Space) => match (row, selected) {
                (Some(ResultRow::File(_)), Some(i)) => self.click_result(i),
                (Some(ResultRow::Match(..)), Some(i)) => {
                    let to_editor = matches!(key, Key::Named(NamedKey::Enter));
                    self.open_result(i, to_editor);
                }
                _ => {}
            },
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => {
                if let Some(i) = selected {
                    self.dismiss_row(i);
                }
            }
            Key::Named(NamedKey::Tab) => self.cycle_panel_focus(shift),
            _ => {}
        }

        self.request_redraw();
    }

    /// 处理 `Esc`：离开搜索面板，焦点回到编辑区；没有打开的文档时回到搜索输入框。
    fn leave_panel(&mut self) {
        self.field_preedit.clear();
        self.focus = if self.editor.active.is_some() {
            Focus::Editor
        } else {
            Focus::GlobalQuery
        };
    }

    /// 处理跳转条和文件树「输入名字」行中的按键。
    fn on_key_prompt(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let name_input = self.name_input.is_some();
        // 多行输入（PDF 批注）：`⇧↩` 换行，`↑` `↓` 在折好的行之间移动。
        let note = !name_input
            && self
                .goto_input
                .as_ref()
                .is_some_and(TextField::is_multiline);
        let shift = self.modifiers.shift_key();

        match &event.logical_key {
            Key::Named(NamedKey::Enter) if note && shift => {
                self.edit_field(|f| f.insert("\n"));
            }
            Key::Named(key @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) if note && !cmd => {
                let down = *key == NamedKey::ArrowDown;
                self.edit_field(|f| {
                    f.move_vertical(down, shift);
                    false
                });
            }
            Key::Named(NamedKey::Escape) => {
                if name_input {
                    self.close_name_input();
                    self.focus = Focus::Tree;
                } else {
                    self.close_prompt();
                    self.focus = Focus::Editor;
                }
            }
            Key::Named(NamedKey::Enter) => self.submit_prompt(),
            Key::Named(NamedKey::Tab) if name_input => self.focus = Focus::Tree,
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 处理设置浮层打开时的全部按键。
    ///
    /// - 在字体输入框里：`↩` 提交（名字不对时停留在输入框），`Esc` 放弃这次改动（不关闭浮层），
    ///   `Tab` 离开输入框并跳到下一项（名字不对时退回原值），其余交给输入框。
    /// - 不在输入框里：`↑` `↓`（`Tab` / `⇧Tab`）在设置项之间移动，`←` `→` 改值，`↩` 或空格激活，
    ///   `Esc`、`⌘,` 或 `⌘W` 关闭浮层。
    fn on_key_settings(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let shift = self.modifiers.shift_key();

        if self.focus == Focus::KeySearch {
            self.on_key_search_keys(event, base, cmd);
            return;
        }
        if self.focus == Focus::SettingsSearch {
            self.on_key_settings_search(event, base, cmd);
            return;
        }

        if cmd && matches!(base, "," | "w") {
            self.close_settings();
            return;
        }

        if self.focus == Focus::Setting {
            match &event.logical_key {
                Key::Named(NamedKey::Enter) => self.settings_commit_field(),
                Key::Named(NamedKey::Escape) => self.settings_cancel_field(),
                Key::Named(NamedKey::Tab) => self.settings_move(!shift),
                // 复制、剪切和全选由输入框自己处理；粘贴不在 `on_key_field` 中，
                // 需要单独调用 `paste`。
                _ if cmd && base == "v" => self.paste(),
                _ => self.on_key_field(event, base, cmd),
            }
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_settings(),
            Key::Named(NamedKey::ArrowDown) => self.settings_move(true),
            Key::Named(NamedKey::ArrowUp) => self.settings_move(false),
            Key::Named(NamedKey::Tab) => self.settings_move(!shift),
            Key::Named(NamedKey::ArrowLeft) => self.settings_step(false),
            Key::Named(NamedKey::ArrowRight) => self.settings_step(true),
            Key::Named(NamedKey::Enter | NamedKey::Space) => self.settings_activate(),
            _ if cmd && base == "f" => self.focus_settings_search(),
            // 直接打字即开始搜索：焦点移到搜索框，这个字写进去。
            Key::Character(_) if !cmd && !self.modifiers.control_key() => {
                self.focus = Focus::SettingsSearch;
                self.on_key_field(event, base, cmd);
            }
            _ => {}
        }
    }

    /// 处理设置分类栏顶部搜索框中的按键：`↓`、`↩`、`Tab` 进入搜索结果，`Esc` 先清空搜索框，
    /// 搜索框为空时关闭设置，其余交给输入框。
    fn on_key_settings_search(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Tab) => {
                self.settings_enter_results();
            }
            Key::Named(NamedKey::Escape) if self.settings.search.text().is_empty() => {
                self.close_settings();
            }
            Key::Named(NamedKey::Escape) => self.edit_field(|f| {
                f.set_text("");
                true
            }),
            _ if cmd && matches!(base, "," | "w") => self.close_settings(),
            _ if cmd && base == "v" => self.paste(),
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 处理表单对话框打开时的全部按键。
    ///
    /// `Tab`、`⇧Tab` 在各部件之间移动键盘焦点，`Esc`、`⌘W` 关闭对话框。焦点在名称上时
    /// `↩` 创建，其余按键交给输入框；焦点在其他部件上时，空格和 `↩` 激活该部件。
    fn on_key_dialog(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        if !self.dialog_key(&event.logical_key, base, cmd) {
            self.on_key_field(event, base, cmd);
        }
    }

    /// 处理对话框自身响应的按键（见 [`App::on_key_dialog`]）；按键须交给名称输入框时返回 `false`。
    pub(in crate::app) fn dialog_key(&mut self, key: &Key, base: &str, cmd: bool) -> bool {
        let on_name = self.form_dialog.as_ref().is_none_or(|d| {
            matches!(
                d.focus,
                crate::layout::DialogPart::Branch | crate::layout::DialogPart::Second
            )
        });
        // 交互式变基的信息框是多行的：回车换行，⌘回车开始变基。
        let multiline = on_name
            && (self.form_dialog.as_ref())
                .is_some_and(|d| matches!(d.kind, crate::app::git::form::FormKind::Rebase(_)));
        if !cmd
            && !matches!(key, Key::Named(NamedKey::Escape | NamedKey::Tab))
            && self.rebase_list_key(key, base)
        {
            return true;
        }
        match key {
            Key::Named(NamedKey::Escape) => self.close_form_dialog(),
            Key::Named(NamedKey::Tab) => self.form_focus_move(self.modifiers.shift_key()),
            _ if cmd && base == "w" => self.close_form_dialog(),
            Key::Named(NamedKey::Enter) if multiline && !cmd => {
                self.edit_field(|field| field.insert("\n"));
            }
            Key::Named(NamedKey::Enter) if on_name => self.submit_form_dialog(),
            Key::Named(NamedKey::Enter | NamedKey::Space) if !on_name => self.form_activate(),
            _ if !on_name => {}
            _ if cmd && base == "v" => self.paste(),
            _ => {
                // 名称即将被修改，上一次提交失败的原因不再适用。
                if let Some(dialog) = &mut self.form_dialog {
                    dialog.error = None;
                }
                return false;
            }
        }
        true
    }

    /// 处理快捷键页搜索框中的按键：先交给 `keys_page_stroke`，未处理的再交给输入框。
    fn on_key_search_keys(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let stroke = self.stroke_of(&event.logical_key, base, event.physical_key, event.location);
        if let Some(stroke) = stroke
            && self.keys_page_stroke(stroke)
        {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.settings_scroll_by(-KEYS_ROW),
            Key::Named(NamedKey::ArrowUp) => self.settings_scroll_by(KEYS_ROW),
            // `⇧Tab` 回到上一项（格式化页的最后一项）；`Tab` 不处理，搜索框已是最后一项。
            Key::Named(NamedKey::Tab) if self.modifiers.shift_key() => self.settings_move(false),
            Key::Named(NamedKey::Tab) => {}
            _ if cmd && matches!(base, "," | "w") => self.close_settings(),
            _ if cmd && base == "v" => self.paste(),
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 是否已处理快捷键页上的一次按键 `stroke`。
    ///
    /// 录制按键时，按键写入搜索框，`Esc` 停止录制；不录制时，`Esc` 先清空搜索框，搜索框为空时
    /// 关闭设置。「录制按键」的快捷键在两种状态下都用于开关录制。
    pub(in crate::app) fn keys_page_stroke(&mut self, stroke: Stroke) -> bool {
        let action = match keymap::lookup(keymap::current(), stroke, None, Context::Keys) {
            Lookup::Run(actions) => actions.first().copied(),
            _ => None,
        };
        if action == Some(keymap::Action::RecordKeys) {
            self.toggle_recording();
            return true;
        }
        let escape = stroke.key == keymap::KeyCode::Escape && stroke.mods == keymap::Mods::NONE;
        if self.settings.recording {
            if escape {
                self.toggle_recording();
            } else {
                self.record_stroke(stroke);
            }
            return true;
        }
        if !escape {
            return false;
        }
        if self.settings.keys_query.text().is_empty() {
            self.close_settings();
        } else {
            self.edit_field(|f| {
                f.set_text("");
                true
            });
        }
        true
    }

    /// 处理快速打开面板中的按键。
    ///
    /// `↑` `↓`（macOS 上还有 `⌃P` / `⌃N`）在结果之间移动选中项，`↩` 打开，`Esc` 关闭面板。
    /// 其余按键交给输入框，因为面板的输入部分就是一个普通的单行输入框。
    fn on_key_picker(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let ctrl = cfg!(target_os = "macos") && self.modifiers.control_key();

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.close_picker();
                return;
            }
            Key::Named(NamedKey::Enter) => {
                self.picker_accept();
                return;
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.picker_move(true);
                return;
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.picker_move(false);
                return;
            }
            // 分支、位置选择器没有输入查询时，左右键展开、收起目录，右键打开操作菜单。
            Key::Named(NamedKey::ArrowRight)
                if self.branch_picker_side(true) || self.location_picker_side(true) =>
            {
                return;
            }
            Key::Named(NamedKey::ArrowLeft)
                if self.branch_picker_side(false) || self.location_picker_side(false) =>
            {
                return;
            }
            // macOS 上 `⌃N` / `⌃P` 也可以上下选择（Cocoa 文本系统的标准按键绑定）。
            Key::Character(c) if ctrl && c.as_str() == "n" => {
                self.picker_move(true);
                return;
            }
            Key::Character(c) if ctrl && c.as_str() == "p" => {
                self.picker_move(false);
                return;
            }
            _ => {}
        }

        self.on_key_field(event, base, cmd);
    }

    /// 处理输入框通用的按键：移动光标、选择、删除、剪贴板、撤销和键入。
    ///
    /// macOS：`⌥` 按词，`⌘←` / `⌘→` 到开头末尾，`⌘⌫` 删到开头；其他平台 `Ctrl` 按词。
    /// `Shift` 扩展选区。
    ///
    /// `⌘V` 粘贴是全局快捷键，按焦点分派，见 `App::paste`。
    pub(in crate::app) fn on_key_field(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        let shift = self.modifiers.shift_key();
        let mac = cfg!(target_os = "macos");
        let unit = if mac && cmd {
            Unit::Line
        } else if (mac && self.modifiers.alt_key()) || (!mac && cmd) {
            Unit::Word
        } else {
            Unit::Char
        };

        if cmd {
            match base {
                "a" => {
                    return self.edit_field(|f| {
                        f.select_all();
                        false
                    });
                }
                "c" | "x" => return self.copy_field(base == "x"),
                "z" if shift => return self.edit_field(TextField::redo),
                "z" => return self.edit_field(TextField::undo),
                "y" if !mac => return self.edit_field(TextField::redo),
                _ => {}
            }
        }

        let movement = |forward: bool, unit: Unit| {
            move |f: &mut TextField| {
                f.move_caret(unit, forward, shift);
                false
            }
        };

        match &event.logical_key {
            Key::Named(NamedKey::ArrowLeft) => self.edit_field(movement(false, unit)),
            Key::Named(NamedKey::ArrowRight) => self.edit_field(movement(true, unit)),
            // 单行输入框中 `↑` / `↓` 移到开头或末尾，加 `Shift` 则选到开头或末尾。
            Key::Named(NamedKey::ArrowUp | NamedKey::Home) => {
                self.edit_field(movement(false, Unit::Line))
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::End) => {
                self.edit_field(movement(true, Unit::Line))
            }
            Key::Named(NamedKey::Backspace) => self.edit_field(|f| f.delete(unit, false)),
            Key::Named(NamedKey::Delete) => self.edit_field(|f| f.delete(unit, true)),
            _ => {
                if !cmd
                    && let Some(text) = &event.text
                    && !text.chars().any(char::is_control)
                {
                    self.edit_field(|f| f.insert(text));
                }
            }
        }
    }

    /// 处理文件树中的按键。
    ///
    /// `↑` `↓` 移动选中项，`Home` `End` 选中第一行、最后一行，`PageUp` `PageDown` 翻一屏，
    /// 按住 `⇧` 时均扩展选区；`←` `→` 折叠、展开或跳到上一级目录；键入字母、数字跳到名称以其
    /// 开头的行（见 [`TYPE_RESET`]）；`Esc` 先将多选收回为当前行，再次按下时回到编辑区。
    /// 打开、重命名、删除在键位表中定义。
    pub(in crate::app) fn on_key_tree(&mut self, key: &Key) {
        let shift = self.modifiers.shift_key();
        let plain = !(self.modifiers.super_key()
            || self.modifiers.control_key()
            || self.modifiers.alt_key());
        // 翻一屏移动的行数为可见整行数减一，翻页后原来的最后一行仍然可见。
        let page = ((self.layout.tree.h / crate::theme::TREE_ROW).floor() as usize)
            .saturating_sub(1)
            .max(1);
        let Some(ws) = &mut self.editor.workspace else {
            self.focus = Focus::Editor;
            return;
        };

        let count = ws.rows().len();

        if count == 0 {
            return;
        }

        // 还没有选中任何一行时，`↑` `↓` 先选中第一行。
        if ws.selected.is_none()
            && matches!(key, Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp))
        {
            ws.select(Some(0));
            self.scroll_tree_to(0, false);
            return;
        }

        let selected = ws.selected.unwrap_or(0).min(count - 1);
        let entry = &ws.rows()[selected];
        let is_dir = entry.kind == EntryKind::Dir;
        let mut expand = None;

        // 键入的字符追加到上一次键入的字符之后匹配，间隔超过 `TYPE_RESET` 时重新开始。只有一个
        // 字符时从下一行开始查找，连按同一字母可在以其开头的各行间轮换；多个字符时从当前行开始
        // 查找，当前行仍然匹配时选中项不变。
        if let Key::Character(typed) = key
            && plain
            && typed.chars().all(|c| !c.is_control())
        {
            let now = Instant::now();
            let mut word = match self.tree_typed.take() {
                Some((word, at)) if now.duration_since(at) < TYPE_RESET => word,
                _ => String::new(),
            };
            word.push_str(&typed.to_lowercase());
            let start = if word.chars().count() == 1 {
                selected + 1
            } else {
                selected
            };
            let rows = ws.rows();
            let hit = (start..count)
                .chain(0..start)
                .map(|i| i % count)
                .find(|&i| {
                    let matches = |name: &str| name.to_lowercase().starts_with(&word);
                    matches(&rows[i].name)
                        || rows[i]
                            .chain()
                            .iter()
                            .any(|path| matches(&segment_name(path)))
                });
            self.tree_typed = Some((word, now));
            let Some(hit) = hit else {
                return;
            };
            ws.select(Some(hit));
            self.scroll_tree_to(hit, false);
            self.request_redraw();
            return;
        }

        let jump = |to: usize, ws: &mut winc_editor::Workspace| {
            if shift {
                ws.select_range(to);
            } else {
                ws.select(Some(to));
            }
        };
        let chain = entry.chain();
        let at = chain
            .iter()
            .position(|path| Some(path.as_path()) == ws.current_path());
        let previous = at.and_then(|i| chain.get(i.checked_sub(1)?)).cloned();
        let next = at.and_then(|i| chain.get(i + 1)).cloned();

        match key {
            Key::Named(NamedKey::Home) => jump(0, ws),
            Key::Named(NamedKey::End) => jump(count - 1, ws),
            Key::Named(NamedKey::PageDown) => jump((selected + page).min(count - 1), ws),
            Key::Named(NamedKey::PageUp) => jump(selected.saturating_sub(page), ws),
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let next = if *key == Key::Named(NamedKey::ArrowDown) {
                    (selected + 1).min(count - 1)
                } else {
                    selected.saturating_sub(1)
                };
                if shift {
                    ws.select_range(next);
                } else {
                    ws.select(Some(next));
                }
            }
            // 合并显示的一行：左右键先在各级目录之间移动，到头才收起、展开或换行。
            Key::Named(NamedKey::ArrowLeft) if previous.is_some() => {
                ws.select_segment(selected, previous.as_deref());
            }
            Key::Named(NamedKey::ArrowRight) if next.is_some() => {
                ws.select_segment(selected, next.as_deref());
            }
            Key::Named(NamedKey::ArrowRight) if is_dir && !entry.expanded => expand = Some(true),
            Key::Named(NamedKey::ArrowRight) if is_dir => {
                ws.select(Some((selected + 1).min(count - 1)));
            }
            Key::Named(NamedKey::ArrowLeft) if is_dir && entry.expanded => expand = Some(false),
            Key::Named(NamedKey::ArrowLeft) => {
                // 跳到上一级目录。
                if let Some(parent) = entry.parent {
                    ws.select(Some(parent));
                }
            }
            Key::Named(NamedKey::Escape) if ws.multi_selected() => {
                ws.select(Some(selected));
            }
            Key::Named(NamedKey::Escape) => {
                if self.editor.active.is_some() {
                    self.focus = Focus::Editor;
                }
                return;
            }
            _ => return,
        }

        if let Some(expand) = expand {
            self.set_dir_expanded(selected, Some(expand));
        }

        if let Some(index) = self.editor.workspace.as_ref().and_then(|ws| ws.selected) {
            self.scroll_tree_to(index, false);
        }
        self.request_redraw();
    }

    /// 处理输入法事件 `ime`。
    ///
    /// 组字中的文本（`Preedit`）暂存起来用于显示；提交的文本（`Commit`）插入焦点所在的
    /// 编辑区或输入框。
    pub(in crate::app) fn on_ime(&mut self, ime: Ime) {
        match ime {
            Ime::Preedit(text, _) => {
                if self.focus.is_field() {
                    self.field_preedit = text;
                } else {
                    self.preedit = text;
                }
            }
            Ime::Commit(text) => {
                self.preedit.clear();
                self.field_preedit.clear();

                let focus = self.focus;
                match focus {
                    focus if focus.is_field() => self.edit_field(|field| field.insert(&text)),
                    Focus::Editor if self.diff_edit_text(&text) => {}
                    Focus::Editor => self.run(Command::Insert(text)),
                    Focus::Terminal => self.terminal_commit(&text),
                    _ => {}
                }
            }
            Ime::Disabled => {
                self.preedit.clear();
                self.field_preedit.clear();
            }
            Ime::Enabled => {}
        }

        self.request_redraw();
    }

    /// 把输入法候选窗的位置（`ime_cursor`，逻辑像素）同步给窗口；位置没有变化时不重复设置。
    ///
    /// 窗口按系统的逻辑坐标接收位置，界面的逻辑像素乘以界面缩放倍数后传入。
    pub(in crate::app) fn update_ime_area(&mut self) {
        if let (Some(area), Some(window)) = (self.ime_cursor, &self.window)
            && self.last_ime_area != Some(area)
        {
            self.last_ime_area = Some(area);
            let zoom = f64::from(self.zoom());
            window.set_ime_cursor_area(
                LogicalPosition::new(f64::from(area.x) * zoom, f64::from(area.y) * zoom),
                LogicalSize::new(f64::from(area.w) * zoom, f64::from(area.h) * zoom),
            );
        }
    }
}
