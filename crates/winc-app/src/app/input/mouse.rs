//! 鼠标：悬停、点击、拖动和滚轮。
//!
//! 绘制每帧把可点击的区域记入 `*_hits` 表（标签、文件树、编辑区、按钮等），鼠标事件按当前
//! 焦点和这些表找到目标。拖动包括标签排序、文件树拖动、滚动条和文本选区。

mod click;
mod drag;
mod regions;
mod target;

use target::*;

use std::time::{Duration, Instant};

use winit::event::{MouseButton, MouseScrollDelta};
use winit::window::CursorIcon;

use winc_core::{Lines, Selection, movement};
use winc_render::Renderer;

use crate::app::{
    App, Click, Clock, DOUBLE_CLICK, Drag, Focus, Hover, MenuCommand, PICKER_ROW, TreeDrag,
    WindowControl, resize_cursor, tree_max_scroll,
};
use crate::layout::{
    RowAction, Scrollbar, SearchParts, SettingControl, SettingsPart, SidebarView, hit,
    hscrollbar_thumb, row_actions, scroll_for_thumb, scrollbar_thumb, settings_thumb,
    settings_track,
};
use crate::theme::*;
use crate::view;

/// 鼠标滚轮滚动一格的距离（逻辑像素）；macOS 上不使用。
///
/// 依据滚轮一格的增量：Windows 的 `WHEEL_DELTA` 和 Wayland 的 `axis_value120` 都将一格记为 120，
/// 每单位合 1 逻辑像素。
#[cfg(not(target_os = "macos"))]
const WHEEL_NOTCH: f32 = 120.0;
/// Windows 上滚轮一格默认滚动的行数（行），依据 `SPI_GETWHEELSCROLLLINES` 的默认值。
#[cfg(windows)]
const WHEEL_LINES: f32 = 3.0;

/// 鼠标滚轮过渡的最短时长（毫秒）。
///
/// 等于 60 Hz 屏幕上的 3 帧：更短时过渡只有一至两帧，与直接滚动没有区别。
const WHEEL_GLIDE_MIN: Duration = Duration::from_millis(50);
/// 鼠标滚轮过渡时长与两格间隔之比。
///
/// 由缓出曲线决定：经过一半时长时已完成 87.5%。匀速转动滚轮时，每格到来之前尚未完成的滚动
/// 稳定在一格的七分之一左右，画面不会越来越落后于滚轮。
const WHEEL_GLIDE_RATIO: u32 = 2;

/// 返回鼠标滚轮一格的过渡时长：与上一格的间隔 `interval` 的 [`WHEEL_GLIDE_RATIO`] 倍，限制在
/// [`WHEEL_GLIDE_MIN`] 与 `SCROLL_GLIDE` 之间；没有上一格时为 `SCROLL_GLIDE`。
///
/// 快速转动滚轮时各格间隔短，过渡随之缩短，画面紧随滚轮；单独的一格按完整时长过渡。
pub(in crate::app) fn wheel_glide_length(interval: Option<Duration>) -> Duration {
    let full = crate::app::animation::SCROLL_GLIDE;
    interval.map_or(full, |interval| {
        (interval * WHEEL_GLIDE_RATIO).clamp(WHEEL_GLIDE_MIN, full)
    })
}

/// 鼠标滚轮单次滚动的过渡状态：一格的距离分多帧完成。
///
/// 每帧调用与直接滚动相同的分发逻辑，因此对各滚动区域都生效，各区域的滚动上限不变。
#[derive(Debug, Clone)]
pub struct WheelGlide {
    /// 本次过渡的总距离（横，纵），单位为逻辑像素。
    pub total: (f32, f32),
    /// 已完成的比例，取值 0 到 1。
    pub done: f32,
    /// 本次过渡的时钟。
    pub clock: Clock,
}

impl App {
    /// 处理鼠标按键 `button` 松开：结束拖动，并在需要时关闭标签。
    ///
    /// 左键按在关闭按钮上、或中键按在标签上之后，松开时鼠标仍在同一个标签上才关闭该标签。
    pub(in crate::app) fn on_mouse_up(&mut self, button: MouseButton) {
        // 拖动标签时悬停目标被清除，松开后重新判断鼠标下方的部件。
        let was_tab = self.dragged_tab().is_some();
        self.drag_scrolled_at = None;
        if button == MouseButton::Left {
            self.diagram_view_mouse_up();
        }
        if button == MouseButton::Left {
            self.release_tree_item();
            self.release_tab();
        }

        if matches!(button, MouseButton::Left | MouseButton::Middle)
            && let Some(id) = self.close_pressed.take()
            && let Some((index, _)) = self.tab_at(self.mouse.0, self.mouse.1)
            && self.editor.documents.get(index).is_some_and(|d| d.id == id)
        {
            self.tabs.width_lock = self.tabs.widths.get(index).map(|&w| (index, w));
            self.close_tab(index);
        }

        self.terminal_mouse_up();
        self.release_terminal_tab();
        let was_scrollbar = matches!(self.drag, Drag::Scrollbar { .. });
        let was_divider = matches!(
            self.drag,
            Drag::Splitter { .. }
                | Drag::TerminalDivider { .. }
                | Drag::SplitDivider { .. }
                | Drag::OutlineDivider { .. }
        );
        if matches!(self.drag, Drag::TerminalDivider { .. }) {
            self.terminal_dragging(false);
        }
        if matches!(self.drag, Drag::Splitter { .. }) {
            self.remember_panel_width();
        }
        self.drag = Drag::None;
        // 拖动分隔条期间不重新断行（`sync_wrap`）；松开后重画，重画前的布局按最终宽度断行。
        if was_divider {
            self.request_redraw();
        }
        if was_scrollbar || was_tab {
            self.set_hover(self.hover_at(self.mouse.0, self.mouse.1));
        }
        self.update_cursor_icon();
    }

    /// 处理滚轮事件 `delta`：滚动鼠标所在区域对应的内容。
    ///
    /// 鼠标滚轮的整格滚动分多帧完成（[`WheelGlide`]）；触控板的滚动量立即生效。
    pub(in crate::app) fn on_wheel(&mut self, delta: MouseScrollDelta) {
        self.dismiss_hint();
        // 滚轮和触控板的滚动由用户直接控制：平滑滚动立即停在目标位置，从该位置继续滚动。
        self.scroll_glide = None;
        let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
        let (dx, dy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                let (step_x, step_y) = self.wheel_step();
                (x * step_x, y * step_y)
            }
            MouseScrollDelta::PixelDelta(p) => (p.x as f32 / scale, p.y as f32 / scale),
        };
        // 鼠标位于全屏查看的图上时，滚动用于平移或缩放图，不再传给被遮住的预览。
        if self.diagram_view_wheel(dx, dy) {
            self.pace_wheel_frame();
            return;
        }
        let line = matches!(delta, MouseScrollDelta::LineDelta(..));
        let glide = self
            .wheel_glides(delta)
            .then(|| self.next_wheel_glide())
            .flatten()
            .and_then(|length| self.clock(length));
        match glide {
            Some(clock) => self.start_wheel_glide((dx, dy), clock),
            None => self.wheel_by(dx, dy, line),
        }
        self.pace_wheel_frame();
    }

    /// 是否对滚动量 `delta` 使用过渡：仅限鼠标滚轮的整格滚动。
    ///
    /// 不足一格的滚动来自高精度设备，例如 Windows 的高精度触控板，事件已足够密集。鼠标位于终端时
    /// 不使用过渡：终端把每格滚动转换为发给程序的按键，不可拆分。
    pub(in crate::app) fn wheel_glides(&self, delta: MouseScrollDelta) -> bool {
        let MouseScrollDelta::LineDelta(x, y) = delta else {
            return false;
        };
        let (mouse_x, mouse_y) = self.mouse;
        (x.abs() >= 1.0 || y.abs() >= 1.0) && !self.layout.terminal.contains(mouse_x, mouse_y)
    }

    /// 鼠标滚轮滚动一格的距离（逻辑像素），依次为横向和竖直。
    ///
    /// - 终端中为 3 行：每行作为一次滚轮事件发给终端中的程序。
    /// - macOS 上为 3 行：系统按滚动的速度增加每格的行数。
    /// - Windows 上为 `WHEEL_NOTCH`，按系统设置的行数相对默认的 `WHEEL_LINES` 缩放；设置为
    ///   一次滚动一屏时为编辑区的一屏。横向按系统设置的字符数，每个字符一个字符宽度。
    /// - 其他系统上为 `WHEEL_NOTCH`。
    pub(in crate::app) fn wheel_step(&self) -> (f32, f32) {
        let (x, y) = self.mouse;
        if self.layout.terminal.contains(x, y) {
            return (
                self.terminal_cell * 3.0,
                self.fonts.terminal_line_height * 3.0,
            );
        }
        #[cfg(target_os = "macos")]
        {
            (self.cell * 3.0, self.fonts.line_height * 3.0)
        }
        #[cfg(windows)]
        {
            let (rows, chars) = crate::windows::wheel_scroll_amounts();
            let line = self.fonts.line_height;
            let page = (self.layout.text.h - line).max(line);
            let y = rows.map_or(page, |rows| rows as f32 / WHEEL_LINES * WHEEL_NOTCH);
            (chars as f32 * self.cell, y)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            (WHEEL_NOTCH, WHEEL_NOTCH)
        }
    }

    /// 返回本格鼠标滚轮滚动的过渡时长（见 [`wheel_glide_length`]），并记录本格的时刻；不使用
    /// 过渡时返回 `None`，直接滚动。
    ///
    /// 只有 macOS 上存在显示链接时使用过渡。Windows 上每格立即滚动，不依赖显示链接的节奏；
    /// 其他系统上按计时器绘制，帧与屏幕刷新不同步，过渡只增加绘制量而不会更平滑。
    fn next_wheel_glide(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let interval = self.wheel_at.replace(now).map(|at| now.duration_since(at));
        #[cfg(target_os = "macos")]
        let follows_refresh = self.display_link.is_some();
        #[cfg(not(target_os = "macos"))]
        let follows_refresh = false;
        follows_refresh.then(|| wheel_glide_length(interval))
    }

    /// 追加滚动距离 `delta`（逻辑像素）：未完成的距离计入新的总距离，按 `clock` 重新计时。
    pub(in crate::app) fn start_wheel_glide(&mut self, delta: (f32, f32), clock: Clock) {
        let rest = self.wheel_glide.take().map_or((0.0, 0.0), |g| {
            let left = 1.0 - g.done;
            (g.total.0 * left, g.total.1 * left)
        });
        self.wheel_glide = Some(WheelGlide {
            total: (rest.0 + delta.0, rest.1 + delta.1),
            done: 0.0,
            clock,
        });
        self.request_redraw();
    }

    /// 应用本帧对应的滚动距离，绘制前调用；时钟结束时清除过渡状态。
    pub(in crate::app) fn advance_wheel_glide(&mut self) {
        let Some(glide) = &mut self.wheel_glide else {
            return;
        };
        let progress = glide.clock.progress();
        let step = progress - glide.done;
        glide.done = progress;
        let (dx, dy) = (glide.total.0 * step, glide.total.1 * step);
        if glide.clock.done() {
            self.wheel_glide = None;
        }
        self.wheel_by(dx, dy, true);
    }

    /// 把滚轮事件引起的绘制推迟到下一次屏幕刷新，每次刷新最多绘制一帧。
    ///
    /// winit 在 macOS 上收到重画请求后立即绘制，不等待屏幕刷新。滚轮事件与刷新不同步时，部分刷新
    /// 周期绘制两帧，部分周期没有新帧，滚动出现抖动。没有显示链接（macOS 14 以下）或窗口不可见时
    /// 不推迟。
    fn pace_wheel_frame(&mut self) {
        #[cfg(target_os = "macos")]
        if self.display_link.is_some() && !self.unseen() {
            self.dirty = false;
            self.retry_frame(Instant::now());
        }
    }

    /// 滚动鼠标所在区域的内容；`dx`、`dy` 的单位是逻辑像素，`line` 表示滚动量来自鼠标滚轮。
    fn wheel_by(&mut self, dx: f32, dy: f32, line: bool) {
        // 按住 `Shift` 时竖直滚动改为横向滚动，只用于鼠标滚轮。触控板本身能横向滚动，手指离开后
        // 还会持续发出惯性滚动事件；这时按 `⇧Tab` 等组合键，惯性滚动会被当成横向滚动，
        // 编辑区突然右移几列。
        let shift_wheel = self.modifiers.shift_key() && dx == 0.0 && line;

        let (x, y) = self.mouse;
        let l = self.layout;

        // 窗口内的提示框打开时什么也不滚；菜单打开时只滚动菜单。
        if self.alert.is_some() || self.popup_wheel(dy) {
            return;
        }
        // 对话框不滚动（交互式变基的列表除外）；下方的编辑区和文件树也不动。
        if self.form_dialog.is_some() {
            self.rebase_wheel(dy);
            return;
        }
        // 三栏合并的两侧随中间的文档滚动：滚轮落在两侧时按落在文档上处理。
        let x = if self.in_merge_pane(x, y) {
            l.text.x
        } else {
            x
        };
        // 设置浮层打开时滚轮只滚动浮层的分组区，下方的编辑区和文件树不动。
        if self.settings.open {
            if self.settings_layout().body.contains(x, y) {
                self.settings_scroll_by(dy);
            }
            return;
        }

        // 查找条覆盖在内容上：滚轮不滚动下方的终端，输入框内的文字过长时横向滚动。
        if !self.picker_open
            && self
                .terminal_find_bar()
                .is_some_and(|bar| bar.contains(x, y))
        {
            let on_field = self.field_at(x, y) == Some(Focus::TerminalFind);
            if let Some(find) = self.terminal.find.as_mut().filter(|_| on_field) {
                let delta = if dx == 0.0 { dy } else { dx };
                find.field.wheel_scroll(-delta);
                self.request_redraw();
            }
            return;
        }

        if self.picker_open && l.picker.contains(x, y) {
            // 滚轮只滚动列表，选中项不变。
            let rows = self.picker_hits.len();
            let max = (rows as f32 * PICKER_ROW - view::picker_span(self)).max(0.0);
            if self.picker_places.is_some() {
                // 位置浮窗按整行滚动，任何时候都只露出完整的行：滚动量攒够一行才滚一行，
                // 触控板的小幅滑动也不会让行停在半截上。
                self.places_wheel += dy;
                let steps = (self.places_wheel / PICKER_ROW).trunc();
                self.places_wheel -= steps * PICKER_ROW;
                let max = (max / PICKER_ROW).ceil() * PICKER_ROW;
                self.picker_scroll = (self.picker_scroll - steps * PICKER_ROW).clamp(0.0, max);
            } else {
                self.picker_scroll = (self.picker_scroll - dy).clamp(0.0, max);
            }
            // 列表滚动后，鼠标下方的行已改变。
            self.set_hover(self.hover_at(x, y));
        } else if l.tabs.contains(x, y) {
            // 竖直滚轮同样用于横向滚动标签栏。
            let delta = if dx.abs() > dy.abs() { dx } else { dy };
            let max = self.tabs.max_scroll();
            self.tabs.scroll = (self.tabs.scroll - delta).clamp(0.0, max);
            // 标签滚动后，鼠标下方的标签已改变。
            self.set_hover(self.hover_at(x, y));
        } else if self.commit_shown() && self.commit_parts().message.contains(x, y) {
            // 提交信息框里的文字超出最大行数时在框内滚动；上限在绘制时按文字的行数收紧。
            let view = &mut self.commit.message.view;
            view.scroll = (view.scroll - dy).max(0.0);
        } else if self.commit_shown()
            && (self.commit_parts().list.contains(x, y) || self.in_commit_overflow(x, y))
        {
            let max = self.commit_max_scroll();
            self.commit.scroll = (self.commit.scroll - dy).clamp(0.0, max);
            self.set_hover(self.hover_at(x, y));
        } else if self.outline_shown() && self.outline_list().contains(x, y) {
            let max = self.outline_max_scroll();
            self.outline.scroll = (self.outline.scroll - dy).clamp(0.0, max);
            self.set_hover(self.hover_at(x, y));
        } else if self.panel_shown() && l.sidebar.contains(x, y) {
            let max = self.results_max_scroll();
            self.global.scroll = (self.global.scroll - dy).clamp(0.0, max);
            self.set_hover(self.hover_at(x, y));
        } else if l.sidebar.contains(x, y) || self.in_tree_overflow(x, y) {
            let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
            let (dx, dy) = if shift_wheel { (dy, 0.0) } else { (dx, dy) };
            let max = tree_max_scroll(l.tree, rows).max(0.0);
            self.tree_scroll = (self.tree_scroll - dy).clamp(0.0, max);
            // 触控板的斜向滚动只取主方向，竖直滚动时不会同时产生左右位移。
            if dx != 0.0 {
                let max_x = self.tree_max_scroll_x();
                let delta = if dy == 0.0 || dx.abs() > dy.abs() {
                    -dx
                } else {
                    0.0
                };
                self.tree_scroll_x = (self.tree_scroll_x + delta).clamp(0.0, max_x);
            }

            // 文件树滚动后，鼠标下方的行已改变。
            self.set_hover(self.hover_at(x, y));
        } else if self.log_active() && self.log_parts().details.contains(x, y) {
            // 日志的详情一栏单独滚动。
            if let Some(view) = self.editor.active_mut().and_then(|d| d.log.as_deref_mut()) {
                view.details_scroll = (view.details_scroll - dy).clamp(0.0, self.log.details_max);
            }
            self.request_redraw();
        } else if l.terminal.contains(x, y)
            && y < l.terminal.y + crate::app::terminal::header_height(l.terminal)
        {
            // 与编辑区的标签栏相同，竖直滚轮同样用于横向滚动。
            let delta = if dx.abs() > dy.abs() { dx } else { dy };
            self.terminal.strip.wheel(delta);
            self.set_hover(self.hover_at(x, y));
        } else if l.terminal.contains(x, y) {
            self.terminal_wheel(dy);
        } else if self.markdown.area.contains(x, y) {
            self.scroll_markdown(dy);
        } else if self.pdf.area.contains(x, y) {
            self.scroll_pdf(dy);
        } else if self.table_at(x, y) {
            let (dx, dy) = if shift_wheel { (dy, 0.0) } else { (dx, dy) };
            self.scroll_table(dx, dy, (x, y));
        } else if l.editor.contains(x, y) {
            // 按住 `Shift` 时，竖直滚轮用于横向滚动。
            let (dx, dy) = if shift_wheel { (dy, 0.0) } else { (dx, dy) };

            if let Some(doc) = self.editor.active_mut() {
                doc.scroll_line -= dy / self.fonts.line_height;
                doc.scroll_x -= dx;
            }

            self.clamp_scroll();
        }

        self.request_redraw();
    }
}
