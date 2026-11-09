//! 浮在编辑区上的条和卡片：查找条、跳转条、快速打开卡片和右上角的提示条。
//!
//! 各元素的几何由 `crate::layout` 计算；本模块按当前状态绘制，并把可点击的按钮记入
//! `App` 上对应的 `*_hits` 表，点击处理在 `app` 各模块中。

use winc_core::Lines;
use winc_i18n::t;

use super::*;
use crate::app::{
    ConflictAction, Focus, NoticeAction, NoticeGhost, PICKER_ICON_X, PICKER_ROW, PICKER_TEXT_X,
    PickerPart, folder_label,
};
use crate::icons::Direction;
use crate::layout::{NOTE_LINES, PickerParts, SearchButton, goto_bar, note_bar};

/// 绘制查找条（⌘F、⌥⌘F）：浮在编辑区右上角，内含查找输入框，展开后含替换输入框。
pub(super) fn draw_search(app: &mut App, frame: &mut Frame<'_>) {
    // 绘制时需要更新输入框中记录的几何信息：先将输入框从 `app` 中取出，绘制后放回。
    let mut find = std::mem::take(&mut app.find_input);
    let mut replace = std::mem::take(&mut app.replace_input);
    let caret = draw_search_bar(app, frame, &mut find, &mut replace);
    app.find_input = find;
    app.replace_input = replace;

    if caret.is_some() {
        app.ime_cursor = caret;
    }
}

/// 绘制查找条的内容，返回有焦点的输入框的光标矩形（供输入法候选框定位）。
///
/// 第一行：左侧是展开替换的箭头和查找输入框，输入框右端是匹配计数；右侧是区分大小写、全字匹配、
/// 正则三个开关，竖线之后是上一个、下一个、关闭，以及「只显示匹配的行」开关。
/// 展开后第二行是替换输入框和两个替换按钮。
fn draw_search_bar(
    app: &App,
    frame: &mut Frame<'_>,
    find: &mut TextField,
    replace: &mut TextField,
) -> Option<Rect> {
    let p = app.palette;
    let bar = app.layout.search;
    let parts = SearchParts::new(bar);
    let search = &app.search;
    let hovered = |button| app.hover == Hover::Search(button);
    let expanded = parts.replace_field.w > 0.0;
    draw_card(frame, p, bar, RADIUS_BAR, p.popup_bg);

    draw_button(
        frame,
        p,
        parts.expand,
        bar,
        hovered(SearchButton::Expand),
        false,
    );
    let direction = if expanded {
        Direction::Down
    } else {
        Direction::Right
    };
    icons::chevron(
        frame,
        center(parts.expand),
        8.0,
        direction,
        p.text_muted,
        bar,
    );

    let query = &search.query;
    let matches = search.matches().len();
    // 截断了，或有行因正则回溯过多而放弃查找：计数是下限。
    let incomplete = search.is_truncated() || search.gave_up();
    let total = if incomplete {
        format!("{matches}+")
    } else {
        matches.to_string()
    };
    let searching = !search.is_complete();
    let count = match (matches, search.current) {
        (0, _) if query.is_empty() => String::new(),
        _ if search.error().is_some() => t!("workspace-find-regex-invalid").to_owned(),
        (0, _) if searching => t!("workspace-find-searching").to_owned(),
        (0, _) => t!("workspace-find-no-results").to_owned(),
        (_, Some(i)) => format!("{}/{total}{}", i + 1, if searching { "…" } else { "" }),
        (_, None) => {
            let more = if incomplete { "+" } else { "" };
            let count = t!("workspace-find-count", count = matches, more = more);
            format!("{count}{}", if searching { "…" } else { "" })
        }
    };
    let failed = search.error().is_some() || (!query.is_empty() && matches == 0 && !searching);
    let count_color = if failed { p.error } else { p.text_dim };

    // 匹配计数靠查找区域右端，查找输入框使用剩余的宽度。
    let field = parts.field;
    let count_x = field.right() - frame.measure(FontKind::Ui, app.fonts.ui_small, &count);
    ui_text_small(frame, app, count_x, field, &count, count_color, field);

    let text_right = if count.is_empty() {
        field.right()
    } else {
        count_x - 10.0
    };
    let text_area = Rect::new(field.x, field.y, (text_right - field.x).max(0.0), field.h);
    let focused = app.focus == Focus::Search;
    let mut caret = draw_bar_field(
        app,
        frame,
        text_area,
        find,
        t!("workspace-find-placeholder"),
        focused,
    );

    let toggles = [
        (
            parts.case_toggle,
            search.case_sensitive,
            hovered(SearchButton::Case),
        ),
        (
            parts.word_toggle,
            search.whole_word,
            hovered(SearchButton::Word),
        ),
        (
            parts.regex_toggle,
            search.regex,
            hovered(SearchButton::Regex),
        ),
    ];
    draw_search_options(app, frame, toggles, bar);

    frame.rect(parts.divider, p.popup_border);

    // 没有结果时，上一个、下一个、替换按钮都置灰，悬停也不响应。
    let can_step = matches > 0;
    let step = if can_step { p.text_muted } else { p.text_dim };
    for button in [SearchButton::Prev, SearchButton::Next] {
        let rect = parts.rect(button);
        draw_button(frame, p, rect, bar, can_step && hovered(button), false);
    }
    // 「只显示匹配的行」开关：开启时与三个选项开关一样铺强调色底色。差异标签里用不了（两侧
    // 按改动对齐），预览上也用不了（没有「行」），置灰。
    let on = app.filter_lines;
    let filter_usable = app.editor.active().is_none_or(|d| d.diff.is_none())
        && !app.markdown_focused()
        && !app.markdown_live();
    draw_toggle(
        frame,
        p,
        parts.filter,
        bar,
        filter_usable && hovered(SearchButton::Filter),
        on,
    );
    let filter_color = match (filter_usable, on) {
        (false, _) => p.text_dim,
        (true, true) => p.accent,
        (true, false) => p.text_muted,
    };
    icons::filter(frame, center(parts.filter), filter_color, bar);
    draw_button(
        frame,
        p,
        parts.close,
        bar,
        hovered(SearchButton::Close),
        false,
    );
    icons::chevron(frame, center(parts.prev), 8.0, Direction::Up, step, bar);
    icons::chevron(frame, center(parts.next), 8.0, Direction::Down, step, bar);
    icons::close(frame, center(parts.close), 7.0, p.text_muted, bar);

    if !expanded {
        return caret;
    }

    let line = Rect::new(
        field.x,
        bar.y + BAR_HEIGHT,
        parts.close.right() - field.x,
        1.0,
    );
    frame.rect(line, p.popup_border);

    let focused = app.focus == Focus::Replace;
    let area = parts.replace_field;
    let placeholder = t!("workspace-find-replace-placeholder");
    let replace_caret = draw_bar_field(app, frame, area, replace, placeholder, focused);
    caret = caret.or(replace_caret);

    // 「替换」「全部替换」是图标按钮，位于「下一个」「关闭」的正下方；没有结果时置灰。
    for button in [SearchButton::ReplaceOne, SearchButton::ReplaceAll] {
        let rect = parts.rect(button);
        draw_button(frame, p, rect, bar, can_step && hovered(button), false);

        if button == SearchButton::ReplaceOne {
            icons::replace_one(frame, center(rect), step, bar);
        } else {
            icons::replace_all(frame, center(rect), step, bar);
        }
    }

    caret
}

/// 绘制跳转条（⌃G）：样式与查找条相同，浮在编辑区顶部居中，右端显示总行数。
pub(super) fn draw_goto(app: &mut App, frame: &mut Frame<'_>) {
    let Some(mut field) = app.goto_input.take() else {
        return;
    };

    let caret = {
        let app = &*app;
        let p = app.palette;

        // 跳转到行、储藏说明，或者输入新标签的名字。
        let (title, placeholder, hint) = match &app.prompt_kind {
            crate::app::PromptKind::GoTo => {
                let total = app.editor.active().map_or(0, |d| d.text().line_count());
                (
                    t!("workspace-goto-title"),
                    t!("workspace-goto-placeholder"),
                    t!("workspace-goto-total", count = total),
                )
            }
            crate::app::PromptKind::Stash => (
                t!("workspace-stash-title"),
                t!("workspace-stash-placeholder"),
                t!("workspace-stash-hint").to_owned(),
            ),
            crate::app::PromptKind::RenameBranch { old } => (
                t!("workspace-rename-branch-title"),
                t!("workspace-rename-branch-placeholder"),
                t!("workspace-rename-branch-hint", name = old.as_str()),
            ),
            crate::app::PromptKind::NewTag { at } => (
                t!("workspace-tag-title"),
                t!("workspace-tag-placeholder"),
                t!(
                    "workspace-tag-hint",
                    commit = crate::app::git::log::short(at)
                ),
            ),
            crate::app::PromptKind::PdfComment(_) => (
                t!("preview-pdf-comment-title"),
                t!("preview-pdf-comment-placeholder"),
                t!("preview-pdf-comment-hint").to_owned(),
            ),
        };
        let focused = app.focus == Focus::Prompt;
        // 多行输入（PDF 批注）可能很长：用更宽的输入条，文字折行，输入条随行数增高。
        let note = field.is_multiline();
        let editor = app.layout.editor;
        let head = if note {
            note_bar(editor, 1)
        } else {
            goto_bar(editor)
        };
        // 右侧的说明最多占一半宽度，保证输入框留有空间。
        let (size, max) = (app.fonts.ui_small, head.w / 2.0);
        let hint = super::ellipsize(frame, &hint, size, max).into_owned();
        let hint_x = head.right() - 14.0 - frame.measure(FontKind::Ui, app.fonts.ui_small, &hint);
        let start = head.x + 14.0 + frame.measure(FontKind::Ui, app.fonts.ui, title) + 10.0;
        let width = (hint_x - 10.0 - start).max(0.0);
        let wrap = note.then(|| wrap_area_field(app, frame, width, &mut field, focused));
        let lines = wrap.as_ref().map_or(1, |w| w.lines() as u32);
        let bar = if note { note_bar(editor, lines) } else { head };
        draw_card(frame, p, bar, RADIUS_BAR, p.popup_bg);

        // 标题和说明与第一行文字对齐。
        ui_text_small(frame, app, hint_x, head, &hint, p.text_dim, bar);
        ui_text(frame, app, head.x + 14.0, head, title, p.text_muted, bar);
        if let Some(wrap) = wrap {
            let shown = lines.min(NOTE_LINES) as f32 * AREA_LINE;
            let area = Rect::new(start, bar.y + (BAR_HEIGHT - AREA_LINE) / 2.0, width, shown);
            draw_area_field(app, frame, area, &mut field, wrap, placeholder, focused)
        } else {
            let area = Rect::new(start, bar.y, width, bar.h);
            draw_bar_field(app, frame, area, &mut field, placeholder, focused)
        }
    };

    app.goto_input = Some(field);

    if caret.is_some() {
        app.ime_cursor = caret;
    }
}

/// 绘制面板中第 `i` 条结果所在行 `row` 的底色：选中的那一行铺选中色，鼠标悬停的那一行铺
/// 悬停色。
fn draw_pick_row(app: &App, frame: &mut Frame<'_>, row: Rect, i: usize, clip: Rect) {
    let p = app.palette;
    let color = if i == app.picker_selected {
        p.selected_inactive
    } else if app.hover == Hover::PickerRow(i) {
        p.hover
    } else {
        return;
    };
    frame.rect_clipped(row.inset(6.0, 1.0), color, RADIUS_ROW, clip);
}

/// 返回快速打开卡片（或位置浮窗）中结果列表可显示区域的高度（逻辑像素）。
///
/// 界面还没有排版（没有窗口）时为 0。
pub(crate) fn picker_span(app: &App) -> f32 {
    picker_parts(app, app.picker_rect()).list.h
}

/// 返回卡片 `card` 的各部件：列出位置（定义、用法、书签）时是带底栏的位置浮窗，否则是快速
/// 打开面板。
///
/// 绘制、点击和滚动都用这里的几何。
pub(crate) fn picker_parts(app: &App, card: Rect) -> PickerParts {
    if app.picker_places.is_some() {
        PickerParts::places(card)
    } else if app.picker_is_git() {
        use crate::app::git::branch::picker::{GIT_PICKER_ACTIONS, GIT_PICKER_FOOT};
        let actions = if app.picker_has_actions() {
            GIT_PICKER_ACTIONS
        } else {
            0.0
        };
        PickerParts::menu(card, actions, GIT_PICKER_FOOT)
    } else {
        PickerParts::new(card)
    }
}

/// 绘制快速打开卡片（⌘P）：上方是输入框，下方是结果列表，整体是一张带阴影的圆角卡片。
///
/// 文件列表中每条是「文件名 + 淡色的所在目录」，选中项底色高亮，输入框右端显示文件总数。
/// 输入框以 `>` 开头时是命令面板（⇧⌘P），每条是「命令 + 淡色的英文名」，右侧是快捷键；
/// 以 `@` 开头时列出当前文档的符号。列出最近的目录（⌃R）时，每条是「目录名 + 淡色的上级
/// 目录」，输入框右端是在新窗口打开的按键。
///
/// 文件索引尚未建好时，列表区显示「正在索引文件…」。
pub(super) fn draw_picker(app: &mut App, frame: &mut Frame<'_>) {
    if app.picker_is_git() {
        super::git::picker::draw(app, frame);
        return;
    }
    // 行内按钮每帧重新登记：换成没有按钮的列表时，不留下上一种列表的按钮。
    app.picker_part_hits.clear();
    let rect = app.layout.picker;
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }

    if app.picker_places.is_some() {
        if let Some(caret) = super::editor::places::draw_live(app, frame, rect) {
            app.ime_cursor = Some(caret);
        }
        return;
    }

    let parts = PickerParts::new(rect);
    let (bar, list) = (parts.bar, parts.list);

    let hint_x = {
        let app_mut = &mut *app;
        let p = app_mut.palette;

        draw_card(frame, p, rect, RADIUS_BAR, p.popup_bg);

        // 输入框一行：左侧是放大镜，中间是输入区，右侧显示条目总数。放大镜与下方各行的文件
        // 图标处于同一条竖直轴线上。
        let x = bar.x + PICKER_ICON_X + icons::FILE_WIDTH / 2.0;
        let cy = bar.y + bar.h / 2.0;
        icons::magnifier(frame, (x, cy), p.text_dim, bar);

        let total = if app_mut.picker_folders {
            t!("workspace-picker-folders-new-window").to_owned()
        } else if app_mut.picker_lists_commands() {
            t!(
                "workspace-picker-commands",
                count = app_mut.picker_hits.len()
            )
        } else if app_mut.picker_lists_symbols() {
            t!(
                "workspace-picker-symbols",
                count = app_mut.picker_hits.len()
            )
        } else {
            // 有查询时写成「命中数 / 总数」，否则只显示总数，无法看出筛选后的数量。
            let total = app_mut.picker.len();
            match app_mut.picker.matches() {
                Some(n) => t!("workspace-picker-files-matched", matched = n, total = total),
                None => t!("workspace-picker-files", count = total),
            }
        };
        let hint_w = frame.measure(FontKind::Ui, app_mut.fonts.ui_small, &total);
        let hint_x = bar.right() - 14.0 - hint_w;
        ui_text_small(frame, app_mut, hint_x, bar, &total, p.text_dim, bar);
        hint_x
    };

    // 输入文本与下方各行的名称从同一个 x 位置开始，右端不伸到输入框右端的文字下面。
    let text_x = bar.x + PICKER_TEXT_X;
    let text_w = (hint_x - 12.0 - text_x).min(bar.w - PICKER_TEXT_X - 100.0);
    let text_area = Rect::new(text_x, bar.y, text_w.max(0.0), bar.h);

    let placeholder = if app.picker_folders {
        t!("workspace-picker-folders-placeholder")
    } else {
        t!("workspace-picker-placeholder")
    };
    let caret = {
        let app_mut = &mut *app;
        let focused = app_mut.focus == Focus::Picker;
        let mut field = std::mem::take(&mut app_mut.picker_input);
        let caret = draw_bar_field(app_mut, frame, text_area, &mut field, placeholder, focused);
        app_mut.picker_input = field;
        caret
    };

    if caret.is_some() {
        app.ime_cursor = caret;
    }

    if rect.h <= BAR_HEIGHT {
        return;
    }

    let p = app.palette;
    frame.rect_clipped(
        Rect::new(bar.x + 1.0, bar.bottom(), bar.w - 2.0, 1.0),
        p.border,
        0.0,
        rect,
    );

    if app.picker_folders {
        app.picker_part_hits = draw_folder_rows(app, frame, &parts);
        return;
    }
    if app.picker_lists_commands() {
        draw_palette_rows(app, frame, &parts);
        return;
    }
    if app.picker_lists_symbols() {
        draw_symbol_rows(app, frame, &parts);
        return;
    }
    if app.editor.root().is_none() {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        let message = t!("workspace-picker-no-folder");
        ui_text(
            frame,
            app,
            list.x + PICKER_TEXT_X,
            row,
            message,
            p.text_dim,
            list,
        );
        return;
    }
    if app.picker_indexing() {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        ui_text(
            frame,
            app,
            list.x + PICKER_TEXT_X,
            row,
            t!("workspace-picker-indexing"),
            p.text_dim,
            list,
        );
        return;
    }

    if app.picker_hits.is_empty() {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        let goto = app.picker_goto.as_deref().map(|g| match g.split_once(':') {
            Some((line, col)) => t!(
                "workspace-picker-goto-line-column",
                line = line,
                column = col
            ),
            None => t!("workspace-picker-goto-line", line = g),
        });
        let message = match goto {
            Some(goto) if app.editor.active.is_some() => goto,
            _ if app.picker.is_empty() => t!("workspace-picker-folder-empty").to_owned(),
            _ => t!("workspace-picker-no-matching-files").to_owned(),
        };
        ui_text(
            frame,
            app,
            list.x + PICKER_TEXT_X,
            row,
            &message,
            p.text_dim,
            list,
        );
        return;
    }

    let scroll = app.picker_scroll;

    for i in parts.visible(scroll, app.picker_hits.len()) {
        let index = app.picker_hits[i];
        let row = parts.row(i, scroll);
        // 滚动时列表上下两端的行只露出一部分，绘制时裁剪到列表区域内。
        let clip = row.intersect(list);

        let Some(file) = app.picker.file(index) else {
            continue;
        };

        draw_pick_row(app, frame, row, i, clip);

        let (dir, name) = match file.path.rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => ("", file.path),
        };

        let cy = icon_center_y(app, row, app.fonts.ui);
        draw_file_icon(frame, p, (row.x + PICKER_ICON_X, cy), name, clip);

        // 查询命中的字用强调色标出，文件名和目录名分别标注。
        let matched = app.picker_matched(i);
        let dir_len = file.path.chars().count() - name.chars().count();
        let in_name: Vec<usize> = matched
            .iter()
            .filter_map(|&c| c.checked_sub(dir_len))
            .collect();
        let at = (row.x + PICKER_TEXT_X, row);
        let x = marked_label(frame, app, app.fonts.ui, at, name, &in_name, p.text, clip);

        // 在「最近打开」分组的第一行右侧显示「最近打开」标注。
        let mut right = row.right() - 12.0;
        if i == 0 && app.picker_recent > 0 {
            let label = t!("workspace-picker-recently-opened");
            right -= frame.measure(FontKind::Ui, app.fonts.ui_small, label);
            ui_text_small(frame, app, right, row, label, p.text_dim, clip);
            right -= 12.0;
        }

        if !dir.is_empty() {
            let room = (right - x - 8.0).max(0.0);
            if room > 20.0 {
                let shown = ellipsize(frame, dir, app.fonts.ui_small, room);
                // 从中间截短了的目录下标对不上，不标。
                let in_dir: Vec<usize> = if shown.len() == dir.len() {
                    matched.iter().copied().filter(|&c| c < dir_len).collect()
                } else {
                    Vec::new()
                };
                let at = (x + 8.0, row);
                let size = app.fonts.ui_small;
                marked_label(frame, app, size, at, &shown, &in_dir, p.text_dim, clip);
            }
        }
    }
}

/// 绘制命令面板的结果列表：命令名，其后是淡色的英文名，右侧是快捷键；最近执行过的命令
/// 分组的第一行标注「最近使用」。
fn draw_palette_rows(app: &App, frame: &mut Frame<'_>, parts: &PickerParts) {
    let p = app.palette;
    let list = parts.list;
    if app.picker_hits.is_empty() {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        ui_text(
            frame,
            app,
            list.x + PICKER_TEXT_X,
            row,
            t!("workspace-picker-no-matching-commands"),
            p.text_dim,
            list,
        );
        return;
    }

    let scroll = app.picker_scroll;
    for i in parts.visible(scroll, app.picker_hits.len()) {
        let Some(&action) = crate::app::palette().get(app.picker_hits[i]) else {
            continue;
        };
        let row = parts.row(i, scroll);
        let clip = row.intersect(list);
        draw_pick_row(app, frame, row, i, clip);

        let mut right = row.right() - 14.0;
        if let Some(shortcut) = keymap::shortcut(action) {
            right -= frame.measure(FontKind::Ui, app.fonts.ui_small, &shortcut);
            ui_text_small(frame, app, right, row, &shortcut, p.text_dim, clip);
            right -= 16.0;
        }
        if i == 0 && app.picker_recent > 0 {
            let label = t!("workspace-picker-recently-used");
            right -= frame.measure(FontKind::Ui, app.fonts.ui_small, label);
            ui_text_small(frame, app, right, row, label, p.text_dim, clip);
            right -= 16.0;
        }

        // 查询命中的字用强调色标出来：中文名、英文名谁命中标谁。
        let title = action.title();
        let marked = app.palette_matched(title);
        let at = (row.x + PICKER_TEXT_X, row);
        let x = marked_label(frame, app, app.fonts.ui, at, title, &marked, p.text, clip);
        let room = (right - x - 8.0).max(0.0);
        if room > 20.0 {
            let english = ellipsize(frame, action.english(), app.fonts.ui_small, room);
            let marked = if english.len() == action.english().len() {
                app.palette_matched(&english)
            } else {
                Vec::new()
            };
            let (at, size) = ((x + 8.0, row), app.fonts.ui_small);
            marked_label(frame, app, size, at, &english, &marked, p.text_dim, clip);
        }
    }
}

/// 绘制当前文档的符号列表：名称，其后是淡色的上级符号（所属的类型或标题），右侧是淡色的符号种类。
fn draw_symbol_rows(app: &App, frame: &mut Frame<'_>, parts: &PickerParts) {
    let p = app.palette;
    let list = parts.list;
    let empty = match &app.picker_symbols {
        None => Some(t!("workspace-no-open-file")),
        Some(s) if s.too_large => Some(t!("workspace-picker-symbols-too-large")),
        Some(s) if s.items.is_empty() => Some(t!("workspace-picker-no-symbols")),
        Some(_) if app.picker_hits.is_empty() => Some(t!("workspace-picker-no-matching-symbols")),
        Some(_) => None,
    };
    if let Some(message) = empty {
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        ui_text(
            frame,
            app,
            list.x + PICKER_TEXT_X,
            row,
            message,
            p.text_dim,
            list,
        );
        return;
    }
    let Some(symbols) = app.picker_symbols.as_ref() else {
        return;
    };

    let scroll = app.picker_scroll;
    for i in parts.visible(scroll, app.picker_hits.len()) {
        let Some(symbol) = symbols.items.get(app.picker_hits[i]) else {
            continue;
        };
        let row = parts.row(i, scroll);
        let clip = row.intersect(list);
        draw_pick_row(app, frame, row, i, clip);

        let label = symbol.kind.label();
        let right = row.right() - 14.0 - frame.measure(FontKind::Ui, app.fonts.ui_small, label);
        ui_text_small(frame, app, right, row, label, p.text_dim, clip);

        let room = (right - row.x - PICKER_TEXT_X - 16.0).max(0.0);
        let name = ellipsize(frame, &symbol.name, app.fonts.ui, room);
        let x = ui_text(frame, app, row.x + PICKER_TEXT_X, row, &name, p.text, clip);
        let room = (right - x - 8.0 - 16.0).max(0.0);
        if let Some(container) = &symbol.container
            && room > 20.0
        {
            let container = ellipsize(frame, container, app.fonts.ui_small, room);
            ui_text_small(frame, app, x + 8.0, row, &container, p.text_dim, clip);
        }
    }
}

/// 绘制最近打开的目录：目录图标、目录名，其后是淡色的上级目录；选中或悬停的那一行右端有
/// 「✕」，点击后从最近列表中移除该目录。返回画出的「✕」，点击和悬停按它们判断。
fn draw_folder_rows(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &PickerParts,
) -> Vec<(Rect, PickerPart)> {
    let p = app.palette;
    let list = parts.list;
    if app.picker_hits.is_empty() {
        let root = app.editor.root();
        let message = if app.state.recent.iter().any(|f| Some(f.as_path()) != root) {
            t!("workspace-picker-no-matching-folders")
        } else {
            t!("workspace-picker-no-recent-folders")
        };
        let row = Rect::new(list.x, list.y, list.w, list.h.min(PICKER_ROW));
        let x = list.x + PICKER_TEXT_X;
        ui_text(frame, app, x, row, message, p.text_dim, list);
        return Vec::new();
    }

    let mut hits = Vec::new();
    let scroll = app.picker_scroll;
    for i in parts.visible(scroll, app.picker_hits.len()) {
        let Some(folder) = app.state.recent.get(app.picker_hits[i]) else {
            continue;
        };
        let row = parts.row(i, scroll);
        let clip = row.intersect(list);
        let forget = PickerPart::ForgetFolder(i);
        let selected = i == app.picker_selected;
        let hovered = matches!(app.hover,
            Hover::PickerRow(r) | Hover::PickerPart(PickerPart::ForgetFolder(r)) if r == i);
        draw_pick_row(app, frame, row, i, clip);
        if !selected && app.hover == Hover::PickerPart(forget) {
            frame.rect_clipped(row.inset(6.0, 1.0), p.hover, RADIUS_ROW, clip);
        }

        // 目录图标与文件列表的文件图标、输入框的放大镜在同一条竖直轴线上。
        let cy = icon_center_y(app, row, app.fonts.ui);
        let x = row.x + PICKER_ICON_X + (icons::FILE_WIDTH - icons::FOLDER_WIDTH) / 2.0;
        icons::folder(frame, (x, cy - 6.0), false, p.folder, clip);

        // 「✕」只在选中或悬停时显示，但位置始终预留，名称和上级目录不会因它出现而移动。
        let close = Rect::new(
            row.right() - 8.0 - BUTTON_SMALL,
            row.y + 2.0,
            BUTTON_SMALL,
            row.h - 4.0,
        );
        if selected || hovered {
            let over = app.hover == Hover::PickerPart(forget);
            draw_button(frame, p, close, clip, over, false);
            icons::close(frame, center(close), 6.5, p.text_muted, clip);
            hits.push((close, forget));
        }

        // 查询命中的字用强调色标出；从中间截短了的文字下标对不上，不标。
        let (name, parent) = folder_label(folder);
        let (in_name, in_parent) = app.folder_matched(i);
        let room = (close.x - 8.0 - (row.x + PICKER_TEXT_X)).max(0.0);
        let shown = ellipsize(frame, &name, app.fonts.ui, room);
        let marks = if shown.len() == name.len() {
            in_name
        } else {
            Vec::new()
        };
        let at = (row.x + PICKER_TEXT_X, row);
        let x = marked_label(frame, app, app.fonts.ui, at, &shown, &marks, p.text, clip);
        let room = close.x - 8.0 - (x + 8.0);
        if !parent.is_empty() && room > 20.0 {
            let shown = ellipsize(frame, &parent, app.fonts.ui_small, room);
            let marks = if shown.len() == parent.len() {
                in_parent
            } else {
                Vec::new()
            };
            let (at, size) = ((x + 8.0, row), app.fonts.ui_small);
            marked_label(frame, app, size, at, &shown, &marks, p.text_dim, clip);
        }
    }
    hits
}

/// 提示条与编辑区右上角（查找条打开时为查找条下方）之间的距离（逻辑像素），上边和右边相同。
const NOTICE_MARGIN: f32 = 8.0;
/// 提示条出现时滑动的距离（逻辑像素）：从角上往下滑出来。
const NOTICE_SLIDE: f32 = 6.0;

/// 绘制编辑区右上角的提示条（出错、冲突、未保存等需要注意的消息），出现时从角上滑出并淡入；
/// 返回提示条和按钮的矩形，没有按钮时后者为 `None`。
pub(super) fn draw_live_notice(
    app: &App,
    frame: &mut Frame<'_>,
    message: &str,
) -> (Rect, Option<Rect>) {
    let label = app.notice_action.as_ref().map(NoticeAction::label);
    let (opacity, away) = app.notice_in.map_or((1.0, 0.0), |clock| {
        let t = clock.linear();
        (1.0 - (1.0 - t).powi(2), (1.0 - t).powi(3))
    });
    frame.with_opacity(opacity, |frame| {
        draw_notice(app, frame, message, label, -NOTICE_SLIDE * away)
    })
}

/// 编辑区右上角浮层的上边界（不含留白）：查找条打开时是查找条的下边。
fn corner_top(app: &App) -> f32 {
    let layout = app.layout;
    if layout.search.h > 0.0 {
        layout.search.bottom()
    } else {
        layout.editor.y
    }
}

/// 绘制当前文档的冲突卡片：靠在编辑区右上角，左边是提醒图标和一句「已被其他程序修改」的说明，
/// 右边依次是「比较」「使用磁盘版本」「保留当前修改」。
///
/// 返回卡片和各按钮的矩形；当前文档没有冲突时不绘制，返回 `None`。卡片一直显示到用户选择
/// 为止，不会自动消失。名字太长时从中间截短。
pub(super) fn draw_conflict(
    app: &App,
    frame: &mut Frame<'_>,
) -> Option<(Rect, Vec<(Rect, ConflictAction)>)> {
    let name = app.active_conflict()?;
    let p = app.palette;
    let editor = app.layout.editor;
    let font = app.fonts.ui;

    let labels: Vec<f32> = ConflictAction::ALL
        .iter()
        .map(|a| frame.measure(FontKind::Ui, font, a.label()) + 16.0)
        .collect();
    let buttons_w: f32 = labels.iter().sum::<f32>() + BUTTON_GAP * (labels.len() - 1) as f32;
    let fixed = frame.measure(
        FontKind::Ui,
        font,
        &t!("workspace-conflict-modified", name = ""),
    );
    // 图标和文字占 34，文字与按钮之间留 16，按钮右边留 8。
    let room =
        (editor.w - 2.0 * NOTICE_MARGIN - 20.0 - 34.0 - 16.0 - buttons_w - 8.0 - fixed).max(24.0);
    let name = ellipsize(frame, &name, font, room).into_owned();
    let message = t!("workspace-conflict-modified", name = name);
    let text_w = frame.measure(FontKind::Ui, font, &message);
    let width = 34.0 + text_w + 16.0 + buttons_w + 8.0;
    let top = corner_top(app) + NOTICE_MARGIN;
    let card = Rect::new(editor.right() - NOTICE_MARGIN - width, top, width, 36.0);

    draw_card(frame, p, card, RADIUS_BAR, p.popup_bg);
    let icon = (card.x + 16.0, icon_center_y(app, card, font));
    icons::warning(frame, icon, p.warning, card);
    ui_text(frame, app, card.x + 34.0, card, &message, p.text, card);

    let mut x = card.right() - 8.0 - buttons_w;
    let y = card.y + (card.h - BUTTON_SMALL) / 2.0;
    let mut buttons = Vec::new();
    for (&action, w) in ConflictAction::ALL.iter().zip(labels) {
        let button = Rect::new(x, y, w, BUTTON_SMALL);
        let hovered = app.hover == Hover::ConflictButton(action);
        draw_button(frame, p, button, card, hovered, false);
        ui_text(
            frame,
            app,
            button.x + 8.0,
            button,
            action.label(),
            p.accent,
            card,
        );
        buttons.push((button, action));
        x += w + BUTTON_GAP;
    }
    Some((card, buttons))
}

/// 绘制到期后正在原地淡出的提示条。
pub(super) fn draw_notice_ghost(app: &App, frame: &mut Frame<'_>, ghost: &NoticeGhost) {
    let t = ghost.clock.linear();
    frame.with_opacity(1.0 - t * t, |frame| {
        draw_notice(app, frame, &ghost.message, ghost.label, 0.0)
    });
}

/// 绘制提示条：靠在编辑区右上角，上边和右边留同样的距离；查找条打开时排在查找条下方。
///
/// `dy` 是动画中相对停靠位置的竖直偏移。
///
/// 消息放不下时从中间截短，使提示条不超出编辑区。带按钮的提示（如「查看输出」）把按钮放在右侧。
/// 返回提示条和按钮的矩形，没有按钮时后者为 `None`。
fn draw_notice(
    app: &App,
    frame: &mut Frame<'_>,
    message: &str,
    label: Option<&str>,
    dy: f32,
) -> (Rect, Option<Rect>) {
    let p = app.palette;
    let layout = app.layout;
    let button_w = label.map_or(0.0, |label| {
        frame.measure(FontKind::Ui, app.fonts.ui, label) + 16.0 + 8.0
    });
    // 左侧留给提醒图标：图标中心距左边 16，文字从 34 开始。
    let pad = 34.0 + 16.0;
    let room = (layout.editor.w - 2.0 * NOTICE_MARGIN - 20.0 - pad - button_w).max(0.0);
    let message = ellipsize(frame, message, app.fonts.ui, room);
    let width = frame.measure(FontKind::Ui, app.fonts.ui, &message) + pad + button_w;
    // 冲突卡片、合并冲突卡片打开时排在其下方。
    let disk = app.conflict_card.as_ref().map(|(card, _)| *card);
    let top = match app.merge.card.or(disk) {
        Some(card) => card.bottom(),
        None => corner_top(app),
    } + NOTICE_MARGIN;
    let x = layout.editor.right() - NOTICE_MARGIN - width;
    let rect = Rect::new(x, (top + dy).round(), width, 36.0);

    // 浮层统一用白底卡片，只用提醒图标的颜色表示需要注意，不整块铺底色。
    draw_card(frame, p, rect, RADIUS_BAR, p.popup_bg);
    let icon = (rect.x + 16.0, icon_center_y(app, rect, app.fonts.ui));
    icons::warning(frame, icon, p.warning, rect);
    ui_text(frame, app, rect.x + 34.0, rect, &message, p.text, rect);

    let Some(label) = label else {
        return (rect, None);
    };
    let button = Rect::new(
        rect.right() - 8.0 - (button_w - 8.0),
        rect.y + (rect.h - BUTTON_SMALL) / 2.0,
        button_w - 8.0,
        BUTTON_SMALL,
    );
    let hovered = app.hover == Hover::NoticeButton;
    draw_button(frame, p, button, rect, hovered, false);
    ui_text(frame, app, button.x + 8.0, button, label, p.accent, rect);
    (rect, Some(button))
}
