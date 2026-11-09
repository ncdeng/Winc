//! PDF 预览的绘制：各页从上到下居中排列，按显示宽度渲染，位图与物理像素一一对应。
//!
//! 页面宽度适应编辑区，最宽不超过 [`MAX_PAGE_WIDTH`]；只请求视口内和视口上下各一屏范围内的页
//! （见 `App::request_pdf_pages`），尚未渲染完成的页先显示白色底（文件刚被修改时显示旧的位图）。
//! 选中的字逐行画一层半透明的蓝色；批注由 PDF 引擎画在位图里。右下角显示当前页号和总页数。

use winc_preview::pdf::merge_lines;
use winc_render::{Color, FontKind, Frame, Rect};

use crate::app::App;
use crate::theme::RADIUS_POPUP;

/// 页面列四周的留白（逻辑像素），与 Markdown 预览的留白相同。
const PAD: f32 = 32.0;
/// 相邻两页之间的间距（逻辑像素），为页面列留白 [`PAD`] 的一半。
const GAP: f32 = 16.0;
/// 页面的最大显示宽度（逻辑像素）：A4 页在该宽度下放大约 1.45 倍，10.5 磅的正文约为 15 逻辑
/// 像素，便于阅读；更宽的编辑区不再放大，同时限制了每页位图的大小。
pub(crate) const MAX_PAGE_WIDTH: f32 = 860.0;

/// 各页在内容中的位置（逻辑像素，相对内容左上角）：页面宽度都按最宽的一页缩放到同一比例。
fn page_rects(pages: &[winc_preview::pdf::PageSize], width: f32) -> (Vec<Rect>, f32) {
    let widest = pages.iter().map(|p| p.width).fold(0.0, f64::max).max(1.0);
    let shown = (width - PAD * 2.0).clamp(1.0, MAX_PAGE_WIDTH);
    let zoom = f64::from(shown) / widest;
    let mut y = PAD;
    let rects = pages
        .iter()
        .map(|page| {
            let (w, h) = ((page.width * zoom) as f32, (page.height * zoom) as f32);
            let rect = Rect::new((width - w) / 2.0, y, w, h);
            y += h + GAP;
            rect
        })
        .collect();
    (rects, y - GAP + PAD)
}

/// 在 `area`（窗口坐标）里画当前标签的 PDF。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    app.pdf.area = area;
    let Some(Ok(pages)) = app.pdf.pages.clone() else {
        return;
    };
    let (rects, height) = page_rects(&pages, area.w);
    app.pdf.height = height;
    app.pdf.page_rects.clone_from(&rects);
    app.pdf.scroll = app.pdf.scroll.clamp(0.0, app.pdf.max_scroll());
    let scroll = app.pdf.scroll;
    let scale = frame.scale();
    // 位图按物理像素对齐：位置和大小都取整到物理像素，绘制时不经插值，线条和文字保持清晰。
    let snap = |v: f32| (v * scale).round() / scale;

    let mut wanted = Vec::new();
    let mut current = 0;
    for (i, rect) in rects.iter().enumerate() {
        let top = area.y + rect.y - scroll;
        // 视口上下各一屏范围内的页也提前请求，滚动到时通常已渲染完成。
        let reach = area.h;
        if top + rect.h < area.y - reach || top > area.bottom() + reach {
            continue;
        }
        let (w, h) = (
            (rect.w * scale).round().max(1.0) as u32,
            (rect.h * scale).round().max(1.0) as u32,
        );
        wanted.push((i, w, h));
        if top <= area.y + area.h / 2.0 {
            current = i;
        }
        if top + rect.h < area.y || top > area.bottom() {
            continue;
        }
        let page = Rect::new(
            snap(area.x + rect.x),
            snap(top),
            w as f32 / scale,
            h as f32 / scale,
        );
        // 页面四周一圈细边框与背景区分，裁剪到预览区域内：滚出顶部的页不画到标签栏上。
        frame.rect_clipped(page.inset(-1.0, -1.0), p.border, 0.0, area);
        frame.rect_clipped(page, winc_render::Color::hex(0xffffff), 0.0, area);
        let fits =
            |px: &&std::sync::Arc<winc_preview::image::Pixels>| px.width == w && px.height == h;
        let shown =
            (app.pdf.rendered.get(&i).filter(fits)).or_else(|| app.pdf.stale.get(&i).filter(fits));
        if let Some(pixels) = shown {
            frame.pixels((page.x, page.y), w, h, &pixels.rgba, area);
        }
        draw_selection(app, frame, i, page, area);
    }
    app.request_pdf_pages(&wanted);
    draw_page_number(app, frame, area, current, pages.len());
}

/// 选中的字上的底色：PDF 页面在两种主题下都是白底，颜色不随主题变化。
const SELECTION: u32 = 0x2563EB;

/// 在第 `index` 页（窗口中的位置为 `page`）上画选中的字：同一行相邻的字合成一个矩形，避免
/// 半透明的底色在字与字之间叠出深浅。
fn draw_selection(app: &App, frame: &mut Frame<'_>, index: usize, page: Rect, area: Rect) {
    let Some(range) = app.pdf.selected_on(index) else {
        return;
    };
    let Some(text) = app.pdf.texts.get(&index) else {
        return;
    };
    let end = range.end.min(text.chars.len());
    let boxes = text.chars[range.start.min(end)..end]
        .iter()
        .filter_map(|c| c.rect);
    let color = Color::hex(SELECTION).alpha(0.28);
    for line in merge_lines(boxes) {
        let rect = Rect::new(
            page.x + line.left * page.w,
            page.y + line.top * page.h,
            (line.right - line.left) * page.w,
            (line.bottom - line.top) * page.h,
        );
        frame.rect_clipped(rect, color, 0.0, area);
    }
}

/// 在右下角画「当前页 / 总页数」：当前页是视口中线所在的页。
fn draw_page_number(app: &App, frame: &mut Frame<'_>, area: Rect, current: usize, total: usize) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let text = format!("{} / {total}", current + 1);
    let tw = frame.measure(FontKind::Ui, size, &text);
    let (w, h) = ((tw + 16.0).round(), 24.0);
    let pill = Rect::new(area.right() - w - 16.0, area.bottom() - h - 16.0, w, h);
    frame.rect_clipped(pill, p.popup_border, RADIUS_POPUP, area);
    frame.rect_clipped(pill.inset(1.0, 1.0), p.popup_bg, RADIUS_POPUP - 1.0, area);
    let baseline = crate::view::ui_baseline(app, pill, size);
    frame.text(
        FontKind::Ui,
        size,
        (pill.x + (w - tw) / 2.0).round(),
        baseline,
        &text,
        p.text_muted,
        area,
    );
}
