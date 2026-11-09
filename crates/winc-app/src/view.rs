//! 界面绘制。
//!
//! [`draw`] 规定一帧的绘制顺序；各区域的绘制在子模块中，本模块还提供各子模块共用的绘制辅助函数。
//!
//! 几何量一律以逻辑像素为单位（缩放倍数由 [`Frame`] 换算到物理像素），原点在窗口左上角，
//! x 向右、y 向下；各区域的位置由 `crate::layout` 计算。

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;
use winc_editor::EntryKind;
use winc_i18n::t;
use winc_render::{Color, Faux, FontKind, Frame, Rect};

use crate::app::animation::{Card, dialog_curve, reveal};
use crate::app::{App, DropZone, Hover, MenuButton, ScrollArea, ThumbLook};
use crate::field::{FieldView, TextField};
use crate::icons;
use crate::keymap::{self, Action};
use crate::layout::{SearchParts, scrollbar_thumb};
use crate::theme::*;

mod alert;
mod dialog;
mod diff;
mod editor;
mod fields;
mod git;
mod overlays;
mod popup_menu;
pub(crate) mod preview;
mod settings;
mod status;
mod terminal;
mod workspace;

pub(crate) use workspace::top_bar::{tab_folder, tab_icon_name};

use editor::draw_editor;
pub(crate) use editor::{PictureCache, RulerCache};
use fields::*;
use overlays::{
    draw_conflict, draw_goto, draw_live_notice, draw_notice_ghost, draw_picker, draw_search,
};
pub(crate) use overlays::{picker_parts, picker_span};
use status::draw_status;
use workspace::sidebar::{
    draw_sidebar, draw_sidebar_button, draw_tree_overflow, draw_tree_overflow_closing,
    tree_overflow,
};
use workspace::top_bar::{draw_top_bar, draw_window_controls};

/// 绘制一帧界面，并重置、重新收集本帧的命中区域（文件树工具栏、项目名等）。
///
/// 绘制顺序为：先绘制侧栏、顶栏、编辑区、状态栏，再绘制查找条、跳转条、快速打开卡片、
/// 提示条等浮层；设置浮层打开时覆盖整个窗口并直接返回，其余情况最后绘制悬停提示。
pub fn draw(app: &mut App, frame: &mut Frame<'_>) {
    // 设置浮层出现、消失的动画期间，下方的窗口和卡片内容不变，用动画开始时保存的快照绘制。
    if settings::draw_cached(app, frame) || settings::draw_scrolling(app, frame) {
        return;
    }
    let layout = app.layout;
    app.tree_overflow = tree_overflow(app, frame);
    let before = app.commit.overflow.map(|o| o.row);
    app.commit.overflow = git::commit::commit_overflow(app, frame);
    app.track_commit_actions(before);

    // 鼠标停在名称展开的行上并滚动文件树后，该位置对应的行可能不再展开名称；此时鼠标的位置
    // 已不在文件树范围内，需要清除悬停状态。
    let (x, y) = app.mouse();
    if app.tree_overflow.is_none()
        && matches!(app.hover, Hover::TreeRow(_))
        && !layout.tree.contains(x, y)
    {
        app.set_hover(Hover::None);
    }

    app.tree_tools.clear();
    app.tree_more = None;
    app.project_hit = Rect::default();
    app.begin_scrollbars();

    if layout.sidebar.right() > 0.0 {
        draw_sidebar(app, frame);
    }
    // 这一帧没有绘制工具栏（侧栏收起或处于搜索视图）：收起动画直接结束，不需要额外一帧。
    if app.tree_tools.is_empty() && app.tree_more.is_none() {
        app.tool_fold.settle();
    }

    draw_top_bar(app, frame);
    draw_sidebar_button(app, frame);
    app.sync_merge_panes();
    draw_editor(app, frame);
    editor::draw_split(app, frame);
    preview::markdown::draw_diagram_view(app, frame);
    git::merge::draw_side_panes(app, frame);
    terminal::draw_terminal(app, frame);
    draw_window_controls(app, frame);
    draw_status(app, frame);
    draw_island_corners(app, frame);
    draw_file_drop(app, frame);
    draw_tree_overflow_closing(app, frame);
    draw_tree_overflow(app, frame);
    git::commit::draw_commit_overflow_closing(app, frame);
    git::commit::draw_commit_overflow(app, frame);
    git::changes::draw_change_popup(app, frame);

    if layout.search.h > 0.0 {
        draw_search(app, frame);
    }

    // 位置浮窗显示在名称旁边，有独立的动画（滑出、淡出）。
    let bar_open = app.goto_input.is_some();
    let picker_open = app.picker_open && app.picker_places.is_none();
    app.track_card(Card::Bar, bar_open);
    app.track_card(Card::Picker, picker_open);
    // 正在淡出的先绘制：在 ⌘P 面板中按 ⌃G 时，跳转条绘制在淡出的面板之上。
    draw_card_ghost(app, frame, Card::Bar);
    draw_card_ghost(app, frame, Card::Picker);
    if bar_open {
        let rect = app.prompt_bar();
        draw_moving_card(app, frame, Card::Bar, rect, draw_goto);
    }
    if picker_open {
        let rect = app.layout.picker;
        draw_moving_card(app, frame, Card::Picker, rect, draw_picker);
    } else if app.picker_open {
        draw_picker(app, frame);
    } else if let Some(ghost) = &app.places_out {
        editor::places::draw_ghost(app, frame, ghost);
    }

    editor::hint::draw_name_hint(app, frame);

    app.conflict_card = draw_conflict(app, frame);
    // 合并冲突的卡片排在磁盘冲突卡片下面。
    let top = match &app.conflict_card {
        Some((card, _)) => card.bottom(),
        None if app.layout.search.h > 0.0 => app.layout.search.bottom(),
        None => app.layout.editor.y,
    } + 8.0;
    let mut hits = std::mem::take(&mut app.merge.hits);
    app.merge.card = git::merge::draw_merge_card(app, frame, top, &mut hits);
    app.merge.hits = hits;
    (app.notice_card, app.notice_button) = match app.notice.as_deref() {
        Some(message) => {
            let (card, button) = draw_live_notice(app, frame, message);
            (Some(card), button)
        }
        None => (None, None),
    };
    if let Some(ghost) = &app.notice_out {
        draw_notice_ghost(app, frame, ghost);
    }

    draw_tree_drag(app, frame);

    // 新建工作树对话框覆盖整个窗口，打开时同样不绘制悬停提示。
    let dialog_open = app.form_dialog.is_some();
    app.track_card(Card::Dialog, dialog_open);
    dialog::draw_ghost(app, frame);
    if dialog_open {
        dialog::draw(app, frame);
        draw_modal(app, frame);
        return;
    }

    // 设置浮层盖住整个窗口（侧栏、编辑区、状态栏）；绘制后直接返回，不再绘制主界面的悬停提示。
    // 关闭后淡出期间仍继续绘制（`SettingsState::motion`）。
    if app.settings.open {
        settings::draw(app, frame);
        draw_modal(app, frame);
        return;
    }
    if app.settings.motion.is_some() {
        settings::draw(app, frame);
    }

    // 窗口内的菜单、提示框绘制在所有内容之上；打开期间不显示悬停提示。关闭后的提示框还需要
    // 若干帧完成淡出，因此每帧都调用一次绘制。
    let modal = app.popup().is_some() || app.alert.is_some();
    draw_modal(app, frame);
    if modal {
        return;
    }
    draw_tooltip(app, frame);
}

/// 绘制打开着的窗口内菜单和提示框：提示框盖在菜单上面。
fn draw_modal(app: &mut App, frame: &mut Frame<'_>) {
    popup_menu::draw(app, frame);
    alert::draw(app, frame);
}

/// 把侧栏、编辑区（以及终端面板）的四角改为圆角。
///
/// 卡片内容按直角矩形绘制，最后用窗口底色覆盖四角圆弧外侧的部分；之后绘制的浮层不受影响。
fn draw_island_corners(app: &App, frame: &mut Frame<'_>) {
    let l = &app.layout;
    let p = app.palette;

    if l.sidebar.right() > 0.0 {
        frame.round_corners(l.sidebar, RADIUS_ISLAND, p.frame_bg);
    }

    let editor = Rect::new(
        l.top_bar.x,
        l.top_bar.y,
        l.top_bar.w,
        l.editor.bottom() - l.top_bar.y,
    );
    frame.round_corners(editor, RADIUS_ISLAND, p.frame_bg);
    if l.terminal.h > 0.0 {
        frame.round_corners(l.terminal, RADIUS_ISLAND, p.frame_bg);
    }
}

/// 拖动文件经过窗口时，为松手后接收该文件的终端面板或编辑区（含标签栏）绘制放置目标底色。
///
/// 底色与文件树中放置目标的颜色相同；拖到文件树上时由文件树标出目标目录。
fn draw_file_drop(app: &App, frame: &mut Frame<'_>) {
    let l = &app.layout;
    let area = match app.file_hover() {
        Some(DropZone::Terminal) => l.terminal,
        Some(DropZone::Open) => Rect::new(
            l.top_bar.x,
            l.top_bar.y,
            l.top_bar.w,
            l.editor.bottom() - l.top_bar.y,
        ),
        Some(DropZone::Tree(_)) | None => return,
    };
    let area = area.inset(6.0, 6.0);
    frame.rect_clipped(area, app.palette.drop_target, RADIUS_ISLAND - 4.0, area);
}

/// 绘制拖动文件树条目时跟随鼠标的小卡片，位于鼠标右下方，内容为被拖动条目的图标和名称。
///
/// 拖动多项时名称写「N 项」，图标取被按住的那一项的图标。名称最多占 240 逻辑像素，超出时
/// 从中间截短；该上限取自调参：常见的文件名在 240 逻辑像素内可以完整显示，更长的名称已难以
/// 辨认。卡片宽度按图标与内边距（逻辑像素）算出。
fn draw_tree_drag(app: &App, frame: &mut Frame<'_>) {
    let Some(drag) = app.tree_dragging() else {
        return;
    };
    let p = app.palette;
    let window = app.layout.window;
    let (x, y) = app.mouse();

    let name = match drag.paths.len() {
        1 => ellipsize(frame, &drag.name, app.fonts.ui, 240.0),
        n => Cow::Owned(t!("workspace-tree-drag-items", count = n)),
    };
    // 按着 ⌥ 时松开是复制。
    let name = if app.modifiers.alt_key() {
        Cow::Owned(t!("workspace-tree-drag-copy", name = name.as_ref()))
    } else {
        name
    };
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &name) + 40.0;
    let card = Rect::new(
        (x + 14.0).min(window.right() - w - 4.0),
        y + 10.0,
        w,
        TREE_ROW,
    );
    draw_card(frame, p, card, RADIUS_ROW, p.popup_bg);

    let cy = icon_center_y(app, card, app.fonts.ui);
    let icon_x = card.x + 12.0;
    if drag.kind == EntryKind::Dir {
        icons::folder(frame, (icon_x, (cy - 6.0).round()), false, p.folder, card);
    } else {
        draw_file_icon(frame, p, (icon_x + 1.5, cy), &drag.name, card);
    }
    ui_text(frame, app, icon_x + 18.0, card, &name, p.text, card);
}

/// 绘制按钮的悬停提示。
///
/// 鼠标在按钮上停留满 `TOOLTIP_DELAY`（500 毫秒）才显示，避免鼠标扫过时闪现。提示位于按钮
/// 下方并水平居中；下方放不下时（状态栏上的按钮）改在上方；概览标尺上的提示位于标尺左侧。
fn draw_tooltip(app: &App, frame: &mut Frame<'_>) {
    // 停留时间不足时直接返回：不必先拼出提示文字、查找命中框。
    let rested = app
        .tooltip_since
        .is_some_and(|since| since.elapsed() >= TOOLTIP_DELAY);
    if !rested {
        return;
    }
    let (button, label): (Rect, String) = match workspace::panel::tooltip(app) {
        Some(tip) => tip,
        None => match app.hover {
            // PDF 批注的注释：贴着鼠标显示，批注的区域可能很大。只有带注释的批注才会悬停到。
            Hover::PdfNote(page, i) => {
                let Some(note) = app.pdf.note(page, i) else {
                    return;
                };
                let (mx, my) = app.mouse();
                (Rect::new(mx, my + 6.0, 0.0, 0.0), note.comment.clone())
            }
            Hover::Search(button) => (
                SearchParts::new(app.layout.search).rect(button),
                button.label(),
            ),
            Hover::TreeTool(tool) => match app.tree_tools.iter().find(|(_, t)| *t == tool) {
                Some(&(button, _)) => (button, tool.label().to_owned()),
                None => return,
            },
            Hover::TreeMore => match app.tree_more {
                Some(button) => (button, t!("workspace-tip-more-actions").to_owned()),
                None => return,
            },
            Hover::SidebarButton => (
                app.layout.sidebar_button,
                crate::layout::with_key(t!("cmd-toggle-sidebar"), Some(Action::ToggleSidebar)),
            ),
            Hover::Browser => (app.layout.browser, t!("preview-browser-open").to_owned()),
            Hover::Preview(mode) => {
                use winc_editor::PreviewMode::*;
                let [source, split, preview] = crate::layout::preview_segments(app.layout.preview);
                // 「预览」对应 ⇧⌘V（在源代码与预览之间切换）。
                match mode {
                    Source => (source, t!("preview-mode-source").to_owned()),
                    Split => (split, t!("preview-mode-split").to_owned()),
                    Preview => (
                        preview,
                        crate::layout::with_key(
                            t!("preview-mode-preview"),
                            Some(Action::TogglePreview),
                        ),
                    ),
                }
            }
            // 日志列表里的作者头像：名字和邮箱。
            Hover::Log(crate::app::git::log::LogTarget::Avatar(i)) => {
                let hit = app
                    .log
                    .hits
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::Avatar(i));
                let commit = app
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref())
                    .and_then(|v| v.commits.get(i));
                match (hit, commit) {
                    (Some(&(rect, _)), Some(c)) => (rect, format!("{} <{}>", c.author, c.email)),
                    _ => return,
                }
            }
            // 提交图中线段的目标：点击后跳转到的提交。
            Hover::Log(crate::app::git::log::LogTarget::Line(row, line)) => {
                let target = crate::app::git::log::LogTarget::Line(row, line);
                if !app.log.hits.iter().any(|(_, t)| *t == target) {
                    return;
                }
                // 线段的命中框很小：提示放在鼠标下方，不按命中框居中。
                let (mx, my) = app.mouse();
                let rect = Rect::new(mx, my + 6.0, 0.0, 0.0);
                let commit = app.log_line_target(row, line).and_then(|j| {
                    app.editor
                        .active()
                        .and_then(|d| d.log.as_deref())
                        .and_then(|v| v.commits.get(j))
                });
                let label = match commit {
                    Some(c) => t!(
                        "workspace-tip-log-go-to",
                        commit = crate::app::git::log::short(&c.oid),
                        subject = c.subject.as_str()
                    ),
                    None => t!("workspace-tip-log-target-not-loaded").to_owned(),
                };
                (rect, label)
            }
            // 日志详情中目录被截短或文件被改名的条目：显示完整路径。
            Hover::Log(crate::app::git::log::LogTarget::File(i))
                if app.log.long_files.contains(&i) =>
            {
                let hit = app
                    .log
                    .hits
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::File(i));
                match (hit, app.log_file_tip(i)) {
                    (Some(&(rect, _)), Some(tip)) => (rect, tip),
                    _ => return,
                }
            }
            // 日志列表里右端也放不下的说明：显示整句。
            Hover::Log(crate::app::git::log::LogTarget::Row(i)) if app.log.clipped == Some(i) => {
                let hit = app
                    .log
                    .hits
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::Row(i));
                let commit = app
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref())
                    .and_then(|v| v.commits.get(i));
                match (hit, commit) {
                    (Some(&(rect, _)), Some(c)) => (rect, c.subject.clone()),
                    _ => return,
                }
            }
            _ => return,
        },
    };
    let label = label.as_str();

    let p = app.palette;
    // 提示分「名称   快捷键」两栏，快捷键以淡色显示在右侧，两栏间距至少 16 逻辑像素。含 `\n` 时
    // 第一行为正文，其余各行为淡色说明（如标签的文件名和所在目录）。
    let (name, key) = match label.split_once(crate::layout::TIP_KEY) {
        Some((name, key)) => (name, Some(key)),
        None => (label, None),
    };
    let lines: Vec<&str> = name.split('\n').collect();
    let size = app.fonts.ui_small;
    let name_w = lines
        .iter()
        .map(|line| frame.measure(FontKind::Ui, size, line))
        .fold(0.0, f32::max);
    let key_w = key.map_or(0.0, |k| 16.0 + frame.measure(FontKind::Ui, size, k));
    // 很长的提示（日志里整句的说明）不超出窗口。
    let width = (name_w + key_w + 20.0).min(app.layout.window.w - 8.0);
    // 单行高 26 逻辑像素，每多一行增加 18。
    const LINE: f32 = 18.0;
    let height = 8.0 + LINE * lines.len() as f32;
    let rect = if matches!(app.hover, Hover::RulerChange(_)) {
        // 概览标尺贴着窗口右边：提示放在它左边、和鼠标同高，不盖住指着的那道标记。
        Rect::new(
            button.x - 8.0 - width,
            button.y + button.h / 2.0 - height / 2.0,
            width,
            height,
        )
    } else {
        let below = button.bottom() + 6.0;
        let y = if below + height > app.layout.window.bottom() {
            button.y - 6.0 - height
        } else {
            below
        };
        Rect::new(
            (button.x + (button.w - width) / 2.0)
                .clamp(4.0, app.layout.window.right() - width - 4.0),
            y,
            width,
            height,
        )
    };
    draw_card(frame, p, rect, RADIUS_ROW, p.popup_bg);
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::new(rect.x, rect.y + 4.0 + LINE * i as f32, rect.w, LINE);
        let color = if i == 0 { p.text } else { p.text_dim };
        ui_text_small(frame, app, rect.x + 10.0, row, line, color, rect);
    }
    if let Some(key) = key {
        let w = frame.measure(FontKind::Ui, size, key);
        let row = Rect::new(rect.x, rect.y + 4.0, rect.w, LINE);
        ui_text_small(
            frame,
            app,
            rect.right() - 10.0 - w,
            row,
            key,
            p.text_dim,
            rect,
        );
    }
}

/// 返回文本在 `rect` 内垂直居中时的基线 y 坐标（逻辑像素，取整）。
fn baseline_in(rect: Rect, ascent: f32, descent: f32) -> f32 {
    (rect.y + (rect.h - (ascent + descent)) / 2.0 + ascent).round()
}

/// 返回字号为 `size` 的界面文本的视觉中心在基线上方的高度（逻辑像素）。
///
/// 视觉中心取小写字母中心与大写字母中心的中点，即（x 高度 + 大写高度）÷ 4。按字体的升降部
/// 高度居中时，界面字体的下沿比上沿短，小写字母会比行的中心低约 1 像素，与按几何中心放置的
/// 关闭按钮、行底色不对齐；按视觉中心居中后，文字、图标和按钮落在同一基线上。
fn optical_center(app: &App, size: f32) -> f32 {
    let k = size / app.fonts.ui;
    (app.ui_metrics.x_height + app.ui_metrics.cap_height) * k / 4.0
}

/// 返回字号为 `size` 的界面文本在 `row` 内垂直居中时的基线 y 坐标（逻辑像素，取整）。
fn ui_baseline(app: &App, row: Rect, size: f32) -> f32 {
    (row.y + row.h / 2.0 + optical_center(app, size)).round()
}

/// 返回与 `row` 内字号为 `size` 的界面文本对齐的图标中心 y 坐标（逻辑像素）：文本的视觉中心，
/// 与行的垂直中心只差基线取整的不到半个像素。
///
/// 结果取整到 0.5 逻辑像素：缩放倍数为 2 时，结果落在整数物理像素上，线条边缘清晰。
fn icon_center_y(app: &App, row: Rect, size: f32) -> f32 {
    let center = ui_baseline(app, row, size) - optical_center(app, size);
    (center * 2.0).round() / 2.0
}

/// 在 `row` 内垂直居中绘制一段字号为 `size` 的界面文本，返回文本末尾的 x 坐标。
///
/// `(x, row)` 为文本起点的 x 和所在的行；超出 `clip` 的部分不绘制。
fn label(
    frame: &mut Frame<'_>,
    app: &App,
    size: f32,
    (x, row): (f32, Rect),
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    let baseline = ui_baseline(app, row, size);
    frame.text(FontKind::Ui, size, x, baseline, text, color, clip)
}

/// 与 [`label`] 相同，但把 `marked` 中的字符（字符下标，升序）用强调色绘制。
///
/// 用于在快速打开和命令面板中标出查询命中的字符。文本按命中与否分成若干段依次绘制；界面
/// 字体不做字距调整，段与段之间没有可见间隙。
#[allow(clippy::too_many_arguments)]
fn marked_label(
    frame: &mut Frame<'_>,
    app: &App,
    size: f32,
    (x, row): (f32, Rect),
    text: &str,
    marked: &[usize],
    color: Color,
    clip: Rect,
) -> f32 {
    let accent = app.palette.accent;
    let (mut x, mut start, mut on) = (x, 0, false);
    for (i, (at, _)) in text.char_indices().enumerate() {
        let hit = marked.binary_search(&i).is_ok();
        if hit != on {
            if at > start {
                let c = if on { accent } else { color };
                x = label(frame, app, size, (x, row), &text[start..at], c, clip);
            }
            (start, on) = (at, hit);
        }
    }
    let c = if on { accent } else { color };
    label(frame, app, size, (x, row), &text[start..], c, clip)
}

/// 绘制输入框的外框：平时没有边框，只在底色 `base` 上铺一层略深的中性色。
///
/// 有焦点时改为编辑区底色并加一圈强调色外环，出错时外环为错误色。侧栏的搜索框、提交信息框和
/// 日志标签的筛选框共用这段绘制。输入框不加边框：多个浅色边框叠在接近白色的侧栏上会互相干扰，
/// 只保留一层底色时层次更清楚。
///
/// 外环宽度为整数个物理像素（缩放倍数四舍五入，至少为 1）：输入框四边先对齐到物理像素，再向外
/// 扩出外环。缩放倍数不是整数（系统缩放 125%、界面缩放 110% 等）时，若按逻辑像素外扩 1，四边
/// 各自取整后宽度在 0 到 2 个物理像素之间，宽度不一致。
fn field_box(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    base: Color,
    focused: bool,
    error: bool,
) {
    let p = app.palette;
    let ring = if error {
        Some(p.error)
    } else if focused {
        Some(p.accent)
    } else {
        None
    };
    match ring {
        Some(color) => {
            let scale = frame.scale();
            let snap = |v: f32| (v * scale).round() / scale;
            let (x, y) = (snap(rect.x), snap(rect.y));
            let inner = Rect::new(x, y, snap(rect.right()) - x, snap(rect.bottom()) - y);
            let ring = scale.round().max(1.0) / scale;
            frame.rounded(inner.inset(-ring, -ring), color, RADIUS_ROW + ring);
            frame.rounded(inner, p.editor_bg, RADIUS_ROW);
        }
        None => frame.rounded(rect, workspace::panel::over(base, p.hover), RADIUS_ROW),
    }
}

/// 侧栏中空列表说明的居中范围（逻辑像素）：顶栏以下的整个侧栏。
fn sidebar_span(app: &App) -> Rect {
    let s = app.layout.sidebar;
    Rect::new(s.x, TOP_BAR_END, s.w, (s.bottom() - TOP_BAR_END).max(0.0))
}

/// 绘制列表为空时（没有结果、没有更改、尚无提交）的说明文字。
///
/// 外观统一：一行标题用正文字号和次要色，下面可以再写一行小字补充（淡色），放不下时按宽度
/// 折行。整块在 `span`（纵向范围）内上下居中，中线位于 `span` 高度的 45% 处，列表很矮时顶部
/// 至少留 16 逻辑像素。
///
/// 居中参照 `span` 而非各自的列表：各视图的列表起始高度不同，按列表居中会使切换视图时说明
/// 文字上下跳动。返回最后一行的下沿，说明下面还要放按钮时从这里往下排。
fn draw_empty_state(
    app: &App,
    frame: &mut Frame<'_>,
    (area, span): (Rect, Rect),
    title: &str,
    detail: Option<&str>,
) -> f32 {
    // 每行高 22 逻辑像素：正文一行 20，两行之间留 2。
    const ROW: f32 = 22.0;
    let p = app.palette;
    let room = (area.w - 32.0).max(40.0);
    let mut lines = vec![(title, app.fonts.ui, p.text_muted)];
    if let Some(detail) = detail {
        let size = app.fonts.ui_small;
        // 逐字累加宽度（界面字体不做字距调整），放不下就换行；每行至少一个字。
        let (mut start, mut width) = (0, 0.0);
        for (i, c) in detail.char_indices() {
            let w = frame.measure(FontKind::Ui, size, c.encode_utf8(&mut [0; 4]));
            if width + w > room && i > start {
                lines.push((&detail[start..i], size, p.text_dim));
                (start, width) = (i, 0.0);
            }
            width += w;
        }
        lines.push((&detail[start..], size, p.text_dim));
    }
    let block = lines.len() as f32 * ROW;
    let top = (span.y + span.h * 0.45 - block / 2.0).max(area.y + 16.0);
    for (i, (text, size, color)) in lines.iter().enumerate() {
        let row = Rect::new(area.x, top + i as f32 * ROW, area.w, ROW);
        let w = frame.measure(FontKind::Ui, *size, text);
        let x = area.x + ((area.w - w) / 2.0).max(16.0);
        label(frame, app, *size, (x, row), text, *color, area);
    }
    top + block
}

/// 在 `rect` 内垂直居中绘制一段字号为 `size`、没有小写字母的界面文本，返回文本末尾的 x 坐标。
///
/// 用于独自放在小框里的快捷键（⌘⇧F）、项目首字母、数字：按大写字母的高度居中（数字与大写字母
/// 等高）。按 [`label`] 的视觉中心居中时，小写字母的中心也算在内，这样的文本会偏高约半个像素。
/// 在一行内与普通文字并列的（状态栏、查找条的计数）仍用 [`label`]，与它们共用基线。
fn caps_label(
    frame: &mut Frame<'_>,
    app: &App,
    size: f32,
    (x, rect): (f32, Rect),
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    let cap = app.ui_metrics.cap_height * size / app.fonts.ui;
    let baseline = (rect.y + rect.h / 2.0 + cap / 2.0).round();
    frame.text(FontKind::Ui, size, x, baseline, text, color, clip)
}

/// 在 `row` 内按正文字号垂直居中绘制一段界面文本，返回文本末尾的 x 坐标。
fn ui_text(
    frame: &mut Frame<'_>,
    app: &App,
    x: f32,
    row: Rect,
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    label(frame, app, app.fonts.ui, (x, row), text, color, clip)
}

/// 与 [`ui_text`] 相同，文字倾斜；用于预览标签的名称。
fn ui_text_italic(
    frame: &mut Frame<'_>,
    app: &App,
    x: f32,
    row: Rect,
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    let size = app.fonts.ui;
    let baseline = ui_baseline(app, row, size);
    let italic = Faux {
        italic: true,
        ..Faux::default()
    };
    frame.text_faux(FontKind::Ui, italic, size, x, baseline, text, color, clip)
}

/// 在 `row` 内按小字号垂直居中绘制一段界面文本，返回文本末尾的 x 坐标。
fn ui_text_small(
    frame: &mut Frame<'_>,
    app: &App,
    x: f32,
    row: Rect,
    text: &str,
    color: Color,
    clip: Rect,
) -> f32 {
    label(frame, app, app.fonts.ui_small, (x, row), text, color, clip)
}

/// 返回列表行（文件树、搜索结果）的底色；不需要底色时返回 `None`。
///
/// 选中的行在有焦点时用强调的选中色，没有焦点时用不透明度减半的选中色，以便与悬停时的浅灰
/// 区分。未选中但悬停的行用浅色，其余行没有底色。
fn row_color(p: &Palette, selected: bool, focused: bool, hovered: bool) -> Option<Color> {
    match (selected, hovered) {
        (true, _) if focused => Some(p.selected),
        (true, _) => Some(p.selected_unfocused),
        (false, true) => Some(p.hover),
        (false, false) => None,
    }
}

/// 绘制文件图标，垂直中心在 `cy`，返回图标右边缘的 x 坐标。
fn draw_file_icon(
    frame: &mut Frame<'_>,
    p: &Palette,
    (x, cy): (f32, f32),
    file_name: &str,
    clip: Rect,
) -> f32 {
    let (glyph, color) = p.file_icon(file_name);
    // 不取整：文件图标都是线条，`cy` 本来就落在半像素上（[`icon_center_y`]）；取整上沿会使图标
    // 比同一行的侧栏开关、修改圆点、加号低约半个像素。
    icons::file(frame, (x, cy - 7.5), glyph, color, clip);
    x + icons::FILE_WIDTH
}

fn center(rect: Rect) -> (f32, f32) {
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

/// 对话框、提示框底部按钮的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    /// 主按钮：强调色的实底、白字，按回车等同于点击它。
    Primary,
    /// 其他按钮：浅灰底。
    Normal,
    /// 危险操作（如「不保存」）：浅灰底，文字用错误色。
    Destructive,
}

/// 绘制对话框、提示框底部的一个按钮，文字居中。
///
/// `enabled` 为假时按钮不响应悬停。主按钮不可用时改为灰底淡字，而不用减淡的强调色填充：
/// 减淡后的强调色与可用状态不易区分。
fn draw_action_button(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    text: &str,
    role: ButtonRole,
    enabled: bool,
    hovered: bool,
) {
    let p = app.palette;
    let hovered = enabled && hovered;
    let ink = match role {
        ButtonRole::Destructive => p.error,
        _ => p.text,
    };
    let (fill, color) = match (role == ButtonRole::Primary, enabled, hovered) {
        (true, false, _) => (p.hover, p.text_dim),
        (true, true, false) => (p.accent_fill, p.text_on_accent),
        (true, true, true) => (p.accent_fill.alpha(0.85), p.text_on_accent),
        (false, false, _) => (p.hover, p.text_dim),
        (false, _, false) => (p.hover, ink),
        (false, _, true) => (p.selected_inactive, ink),
    };
    frame.rounded(rect, fill, RADIUS_ROW);
    let w = frame.measure(FontKind::Ui, app.fonts.ui, text);
    let at = (center(rect).0 - w / 2.0).round();
    ui_text(frame, app, at, rect, text, color, rect);
}

/// 在 `row` 中绘制一行勾选项，返回名称右端的 x 坐标。
///
/// 勾选框的左边与 `row` 的左边对齐，名称跟在勾选框之后。
fn draw_check_option(app: &App, frame: &mut Frame<'_>, row: Rect, text: &str, on: bool) -> f32 {
    let p = app.palette;
    let at = (row.x + 6.0, row.y + row.h / 2.0);
    let state = on.then_some(true);
    icons::checkbox(
        frame,
        at,
        state,
        p.accent,
        p.text_dim,
        p.text_on_accent,
        row,
    );
    ui_text(frame, app, row.x + 20.0, row, text, p.text, row)
}

/// 绘制按钮底色：按下（`on`）时用中性的选中色，悬停时再叠加一层浅色；超出 `clip` 的部分不绘制。
fn draw_button(
    frame: &mut Frame<'_>,
    p: &Palette,
    rect: Rect,
    clip: Rect,
    hovered: bool,
    on: bool,
) {
    if on {
        frame.rect_clipped(rect, p.selected_inactive, RADIUS_ROW, clip);
    }
    if hovered {
        frame.rect_clipped(rect, p.hover, RADIUS_ROW, clip);
    }
}

/// 绘制选项开关（`Aa`、`ab`、`.*`）的底色：打开时用淡强调色，悬停时再叠加一层浅色。
fn draw_toggle(
    frame: &mut Frame<'_>,
    p: &Palette,
    rect: Rect,
    clip: Rect,
    hovered: bool,
    on: bool,
) {
    if on {
        frame.rect_clipped(rect, p.selected, RADIUS_ROW, clip);
    }
    draw_button(frame, p, rect, clip, hovered, false);
}

/// 绘制查找选项的三个开关：区分大小写 `Aa`、全字匹配 `ab`、正则 `.*`。
///
/// 查找条和搜索面板共用这一控件。`toggles` 的每一项依次为：位置、是否开启、鼠标是否悬停。
fn draw_search_options(
    app: &App,
    frame: &mut Frame<'_>,
    toggles: [(Rect, bool, bool); 3],
    clip: Rect,
) {
    let p = app.palette;

    for (i, (rect, on, hovered)) in toggles.into_iter().enumerate() {
        draw_toggle(frame, p, rect, clip, hovered, on);
        let color = if on { p.accent } else { p.text_muted };
        let (cx, cy) = center(rect);

        match i {
            0 => {
                let w = frame.measure(FontKind::Ui, app.fonts.ui_small, "Aa");
                ui_text_small(frame, app, cx - w / 2.0, rect, "Aa", color, clip);
            }
            1 => {
                let w = frame.measure(FontKind::Ui, app.fonts.ui_small, "ab");
                let raised = Rect::new(rect.x, rect.y - 2.0, rect.w, rect.h);
                ui_text_small(frame, app, cx - w / 2.0, raised, "ab", color, clip);
                icons::word_bracket(frame, (cx, cy + 6.0), 13.0, color, clip);
            }
            _ => icons::regex(frame, (cx, cy), color, clip),
        }
    }
}

/// 绘制浮动卡片：柔和阴影、细边框和圆角底色。
///
/// `background` 必须不透明：卡片覆盖的区域不绘制阴影和边框，二者只显示在卡片外侧。
///
/// # Panics
///
/// 调试构建中 `background` 的不透明度小于 1 时触发断言。
fn draw_card(frame: &mut Frame<'_>, p: &Palette, rect: Rect, radius: f32, background: Color) {
    debug_assert!(background.a >= 1.0, "卡片的底色要不透明");
    draw_card_frame(frame, p, rect, radius);
    frame.rounded(rect, background, radius);
}

/// 绘制卡片外侧的阴影与细边框（`rect` 内部不绘制）。
fn draw_card_frame(frame: &mut Frame<'_>, p: &Palette, rect: Rect, radius: f32) {
    let covered = frame.covered(rect, radius);
    frame.shadow(rect, radius, 16.0, 4.0, p.shadow, Some(covered));
    frame.rect_except(
        rect.inset(-1.0, -1.0),
        p.popup_border,
        radius + 1.0,
        covered,
    );
}

/// 卡片阴影超出卡片矩形的范围（逻辑像素）。
///
/// 由阴影参数决定：模糊半径 16、向下偏移 4、细边框 1 像素，合计 24。
const CARD_SHADOW: f32 = 24.0;

/// 把存下的卡片快照 `shot`（原位于 `rect`）作为一个图层叠加绘制。
///
/// 整张快照按 `fade` 调整不透明度、以中心缩放 `scale`，阴影和细边绘制在缩放后的位置上，与
/// 快照一同淡入淡出。设置浮层、`⌘P` 面板、输入条的出现和消失都用这种方式：逐个图形分别
/// 叠加不透明度时，半透明卡片会透出下层内容和自身边框，边缘模糊。
fn blend_card(
    frame: &mut Frame<'_>,
    p: &Palette,
    (rect, shot): (Rect, &winc_render::Snapshot),
    radius: f32,
    (fade, scale): (f32, f32),
) {
    let shown = Rect::new(
        rect.x + rect.w * (1.0 - scale) / 2.0,
        rect.y + rect.h * (1.0 - scale) / 2.0,
        rect.w * scale,
        rect.h * scale,
    );
    frame.with_opacity(fade, |frame| {
        draw_card_frame(frame, p, shown, radius * scale)
    });
    frame.blend_snapshot(shot, radius, fade, scale);
}

/// 绘制一张带出现、消失动画的浮层卡片（`⌘P` 面板、输入条）。
///
/// `draw` 照常绘制卡片内容，卡片位于 `rect`。淡入期间先照常绘制并保存卡片快照，再恢复卡片
/// 下方（含阴影范围）的画面，然后整张淡入并放大；平时每帧也保存一份快照，关闭后由
/// [`draw_card_ghost`] 用最后一份快照淡出。
fn draw_moving_card(
    app: &mut App,
    frame: &mut Frame<'_>,
    card: Card,
    rect: Rect,
    draw: impl FnOnce(&mut App, &mut Frame<'_>),
) {
    // 两份快照复用上一帧的缓冲区：面板打开期间每帧都要保存快照，若每帧新分配数 MB，
    // macOS 的分配器会保留已释放的大块内存，物理内存占用增加数十 MB。
    let mut shot = app
        .card_fx(card)
        .shot
        .take()
        .map(|(_, s)| s)
        .unwrap_or_default();
    let Some((true, clock)) = app.card_fx(card).motion else {
        draw(app, frame);
        frame.snapshot_into(rect, &mut shot);
        app.card_fx(card).shot = Some((rect, shot));
        return;
    };
    let mut base = app.card_fx(card).base.take().unwrap_or_default();
    frame.snapshot_into(rect.inset(-CARD_SHADOW, -CARD_SHADOW), &mut base);
    draw(app, frame);
    frame.snapshot_into(rect, &mut shot);
    frame.restore_tinted(&base, Color::hex(0).alpha(0.0));
    let curve = dialog_curve(true, clock);
    blend_card(frame, app.palette, (rect, &shot), card.radius(), curve);
    let fx = app.card_fx(card);
    fx.shot = Some((rect, shot));
    fx.base = Some(base);
}

/// 在关闭后的淡出期间绘制卡片，使用最后一帧保存的快照。
///
/// 窗口尺寸或缩放倍数变化后快照与当前画面不再对应，此时不绘制。
fn draw_card_ghost(app: &mut App, frame: &mut Frame<'_>, card: Card) {
    let p = app.palette;
    let scale = frame.scale();
    if let Some((clock, rect, shot)) = app.card_fx(card).closing()
        && shot.covers(rect, scale)
    {
        let curve = dialog_curve(false, clock);
        blend_card(frame, p, (rect, shot), card.radius(), curve);
    }
}

/// 绘制区域 `area` 的竖直滚动条，返回本帧滑块的显示状态。
///
/// 滚动条平时隐藏，按 [`App::scrollbar_look`] 淡入、淡出和变色，`owner` 的含义见该方法。
/// 内容高度不超过 `track` 时没有滑块，不绘制。
fn draw_scrollbar(
    app: &mut App,
    frame: &mut Frame<'_>,
    area: ScrollArea,
    owner: u64,
    track: Rect,
    scroll: f32,
    content: f32,
) -> ThumbLook {
    let thumb = scrollbar_thumb(track, scroll, content);
    let look = app.scrollbar_look(area, owner, scroll, thumb.map(|_| track));

    if let Some(thumb) = thumb {
        draw_thumb(frame, app.palette, thumb, look);
    }
    look
}

/// 绘制滚动条的滑块：宽 6 逻辑像素（`SCROLLBAR_THUMB`）的细条，颜色和不透明度按 `look`。
///
/// 鼠标位于滑块的可点击区域内或滑块正在被拖动时颜色加深；宽度不随颜色变化，鼠标进出时
/// 滑块宽度保持一致。
fn draw_thumb(frame: &mut Frame<'_>, p: &Palette, thumb: Rect, look: ThumbLook) {
    if look.opacity > 0.0 {
        frame.rounded(thumb, look.color(p.scrollbar, p.scrollbar_active), 3.0);
    }
}

/// 把字号为 `size` 的界面文本截短到 `max`（逻辑像素）以内，超出时从末尾截断并接上「…」。
///
/// 保留开头部分，「…」前不留空白。项目名、分支名、提交说明都从头读，开头部分足以辨认条目。
fn truncate_end<'a>(frame: &mut Frame<'_>, text: &'a str, size: f32, max: f32) -> Cow<'a, str> {
    fit(frame, text, size, max, false)
}

/// 把字号为 `size` 的界面文本截短到 `max`（逻辑像素）以内，超出时从中间截断，保留开头和
/// 结尾（例如文件扩展名）。
fn ellipsize<'a>(frame: &mut Frame<'_>, text: &'a str, size: f32, max: f32) -> Cow<'a, str> {
    fit(frame, text, size, max, true)
}

/// 把路径 `path` 截短到 `max`（逻辑像素）宽以内，返回截短后的文本。
///
/// 先截短目录部分（从中间截），文件名保持完整；文件名本身放不下时对整个路径从中间截短。
fn shorten_path<'a>(frame: &mut Frame<'_>, path: &'a str, size: f32, max: f32) -> Cow<'a, str> {
    if frame.measure(FontKind::Ui, size, path) <= max {
        return Cow::Borrowed(path);
    }
    let Some(slash) = path.rfind('/') else {
        return ellipsize(frame, path, size, max);
    };
    let (dir, file) = path.split_at(slash + 1);
    let room = max - frame.measure(FontKind::Ui, size, file);
    // 目录部分至少要能显示开头两个字和「…」：目录更短时无法辨认是哪个目录，改为截短整个路径。
    if room < frame.measure(FontKind::Ui, size, "/…") * 2.0 {
        return ellipsize(frame, path, size, max);
    }
    Cow::Owned(format!("{}{file}", ellipsize(frame, dir, size, room)))
}

/// 把文本截短到 `max`（逻辑像素）宽以内并接上「…」，是 [`truncate_end`] 与 [`ellipsize`] 的实现。
///
/// `middle` 为真时从中间截短（开头比结尾多留一个字素），否则从末尾截短。截短按字素进行
/// （带肤色修饰或组合字符的 emoji、带声调的字母各算一个字素），不会切开字素。每个字素只测量
/// 一次宽度，保留的字素个数按宽度逐个累加得出，结果只拼接一次。从中间截短时至少截掉一个字素，
/// 不会出现原文一个字素未少却多出一个「…」的情况。
fn fit<'a>(
    frame: &mut Frame<'_>,
    text: &'a str,
    size: f32,
    max: f32,
    middle: bool,
) -> Cow<'a, str> {
    if text.is_empty() || frame.measure(FontKind::Ui, size, text) <= max {
        return Cow::Borrowed(text);
    }
    // 每个字素在 `text` 里的字节偏移和宽度。
    let chars: Vec<(usize, f32)> = text
        .grapheme_indices(true)
        .map(|(i, g)| (i, frame.measure(FontKind::Ui, size, g)))
        .collect();
    let n = chars.len();
    // 保留 `keep` 个字时，开头留 `keep - keep / 2` 个、结尾留 `keep / 2` 个；多留一个字时，
    // `keep` 为偶数则开头多一个，为奇数则结尾多一个，宽度随 `keep` 只增不减。
    let added = |keep: usize| match (middle, keep % 2) {
        (true, 1) => chars[n - 1 - keep / 2].1,
        (true, _) => chars[keep / 2].1,
        (false, _) => chars[keep].1,
    };
    let limit = if middle { n - 1 } else { n };
    let (mut keep, mut width) = (0, frame.measure(FontKind::Ui, size, "…"));
    while keep < limit && width + added(keep) <= max {
        width += added(keep);
        keep += 1;
    }
    let byte = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
    let (head, tail) = if middle {
        (keep - keep / 2, keep / 2)
    } else {
        (keep, 0)
    };
    // 「…」前面不留空白（截在两个词之间时开头会以空格结尾）。
    Cow::Owned(format!(
        "{}…{}",
        text[..byte(head)].trim_end(),
        &text[byte(n - tail)..]
    ))
}

/// 绘制横向渐隐：从 `rect` 一端的 `color` 渐变到另一端的透明。
///
/// `opaque_left` 为真时左端不透明、右端透明，否则相反。逐列绘制 1 逻辑像素宽的竖条，不透明度
/// 按平方曲线变化，中段变化更快。
fn fade(frame: &mut Frame<'_>, rect: Rect, color: Color, opaque_left: bool) {
    let steps = rect.w.round().max(1.0) as usize;

    for i in 0..steps {
        let t = (i as f32 + 0.5) / steps as f32;
        let a = if opaque_left { 1.0 - t } else { t };
        let strip = Rect::new(rect.x + i as f32, rect.y, 1.0, rect.h);
        frame.rect(strip, color.alpha(color.a * a * a));
    }
}

/// 在 `row` 的右端垂直居中绘制快捷键的圆角键帽。
fn draw_keycap(app: &App, frame: &mut Frame<'_>, row: Rect, key: &str, clip: Rect) {
    let p = app.palette;
    let key_w = frame.measure(FontKind::Ui, app.fonts.ui_small, key);
    let h = 20.0;
    let chip = Rect::new(
        row.right() - key_w - 14.0,
        (row.y + (row.h - h) / 2.0).round(),
        key_w + 14.0,
        h,
    );
    frame.rect_clipped(chip, p.selected_inactive, RADIUS_ROW, clip);
    let size = app.fonts.ui_small;
    caps_label(
        frame,
        app,
        size,
        (chip.x + 7.0, chip),
        key,
        p.text_muted,
        clip,
    );
}

#[cfg(test)]
pub(crate) mod tests;
