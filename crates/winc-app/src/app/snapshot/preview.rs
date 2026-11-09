//! Markdown 预览的测试：三段切换、后台解析、分栏联动滚动、点链接、本地与远程图片、全屏查看
//! mermaid 图、截图与计时。

use winc_editor::PreviewMode;

use super::*;

/// 覆盖常用 Markdown 写法的 README 样本。
const SAMPLE: &str = r#"<p align="center"><img src="logo.svg" width="64" alt="logo"></p>

# winc 编辑器

一个用 **Rust** 写的轻量编辑器，*启动快*、占内存少。支持 `cargo xtask test`、~~旧的写法~~，
中文和 English 混排时按字换行，链接比如 [GitHub](https://github.com) 和 [安装](#安装)。

> [!NOTE]
> 预览跟着左边的文字实时更新。

## 安装

1. 下载发行版
2. 解压到 `/Applications`
   - 第一次打开要在设置里允许
   - 也可以自己编译

- [x] 三段切换
- [ ] Typora 式编辑

```rust
fn main() {
    // 打印一行
    println!("hello, {}", 42);
}
```

| 功能 | 状态 | 说明 |
|:-----|:----:|-----:|
| 分栏 | ✅ | 左边改，右边看 |
| 预览 | ✅ | 只读 |
| 表格 | ✅ | 隔行铺底色 |

> 引用的文字是灰色的，
> 可以有多行。

---

脚注写在最后[^1]。

[^1]: 这是脚注的内容。
"#;

const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect x="5" y="5" width="90" height="90" rx="22" fill="#3296FC"/>
  <path d="M28 34 L40 68 L50 44 L60 68 L72 34" fill="none" stroke="#fff" stroke-width="8"
        stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##;

/// 打开 README，等预览完成解析与图片加载，返回应用和临时目录。
///
/// 读取与绘制图片的请求在绘制一帧时发出，因此用 `settle` 多绘制几轮。
fn readme(name: &str, dark: bool, mode: PreviewMode) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let path = dir.join("README.md");
    std::fs::write(&path, SAMPLE).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(mode);
    settle(&mut app);
    (app, dir)
}

fn settle(app: &mut App) {
    for _ in 0..4 {
        app.wait_for_markdown();
        paint(app);
    }
}

#[test]
fn markdown_files_get_the_three_way_switch() {
    let (mut app, dir) = readme("md-modes", false, PreviewMode::Split);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    assert!(doc.shows_text() && !doc.readonly);
    assert!(app.layout.preview.w > 0.0);
    // 右边是预览：绘制在分栏的右半边，已排出内容和链接。
    assert_eq!(app.markdown.area, app.layout.split);
    assert!(app.markdown.height > app.markdown.area.h);
    assert!(app.markdown.links.len() >= 2);
    // 图片已读取，并按显示大小绘制。
    assert!(matches!(
        app.markdown.images.values().next(),
        Some(crate::app::preview::markdown::ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));

    // 只看预览：整个编辑区，不显示源代码，但可以直接编辑；切回代码照常编辑。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(!doc.readonly && !doc.shows_text() && doc.edits_text());
    assert!(app.markdown_live());
    assert_eq!(app.markdown.area, app.layout.editor);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().readonly);
    assert_eq!(app.markdown.area, Rect::default());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_preview_follows_edits_and_scrolls() {
    let (mut app, dir) = readme("md-edit", false, PreviewMode::Split);
    let before = app.markdown.height;
    // 在文末追加两段：预览变高。
    app.run(winc_editor::Command::DocEnd { extend: false });
    app.run(winc_editor::Command::Insert(
        "\n\n多一段\n\n再多一段\n".into(),
    ));
    settle(&mut app);
    assert!(app.markdown.height > before);

    // 滚轮滚预览。
    let area = app.markdown.area;
    app.mouse = (area.x + 50.0, area.y + 50.0);
    app.markdown.scroll_to(0.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -120.0,
    )));
    assert!(app.markdown.scroll() > 0.0);
    // 不会滚动超过底部。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -100_000.0,
    )));
    paint(&mut app);
    let max = app.markdown.height - area.h;
    assert!((app.markdown.scroll() - max).abs() < 0.5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_sides_scroll_together() {
    // 文档足够长，两侧都能滚动。
    let dir = sample_folder("md-sync");
    let path = dir.join("LONG.md");
    let text: String = (0..60)
        .map(|i| format!("## 第 {i} 节\n\n第 {i} 节的正文。\n\n"))
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    let heading_y = |app: &App, i: usize| {
        crate::view::preview::markdown::anchor_top(app, &format!("第-{i}-节")).unwrap()
    };

    // 左侧滚到第 20 节的标题（第 80 行）：右侧的第 20 节也到达顶部。
    app.editor.active_mut().unwrap().scroll_line = 80.0;
    paint(&mut app);
    // 差值来自预览顶部的留白。
    let pad = app.markdown.scroll() - heading_y(&app, 20);
    assert!((19.0..=33.0).contains(&pad), "{pad}");

    // 右侧滚到第 22 节：左侧同步到第 88 行。
    let target = heading_y(&app, 22) + pad;
    app.scroll_markdown(app.markdown.scroll() - target);
    paint(&mut app);
    let line = app.editor.active().unwrap().scroll_line;
    assert!((line - 88.0).abs() < 0.5, "{line}");
    // 左侧没有再次滚动，右侧也不会被拉回。
    assert!((app.markdown.scroll() - target).abs() < 1.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_scroll_the_full_preview() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = readme("md-keys", false, PreviewMode::Preview);
    // 只读的文档：预览不能编辑，按键滚动预览。
    app.editor.active_mut().unwrap().readonly = true;
    assert!(!app.markdown_live());
    // 焦点在其他位置时点击预览，键盘焦点交给预览。
    app.focus = Focus::Tree;
    let area = app.markdown.area;
    super::click(&mut app, (area.x + area.w / 2.0, area.y + area.h / 2.0));
    assert_eq!(app.focus, Focus::Editor);
    let max = app.markdown.height - app.markdown.area.h;
    assert!(max > 0.0);
    let key = |app: &mut App, key: Key, cmd: bool| {
        assert!(app.markdown_key(&key, cmd));
        app.markdown.scroll()
    };
    let down = key(&mut app, Key::Named(NamedKey::ArrowDown), false);
    assert!(down > 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowUp), false), 0.0);
    // 一屏比一行多，保留部分上一屏的内容。
    let page = key(&mut app, Key::Named(NamedKey::PageDown), false);
    assert!(page > down && page < app.markdown.area.h);
    assert_eq!(key(&mut app, Key::Named(NamedKey::PageUp), false), 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::Space), false), page);
    assert_eq!(key(&mut app, Key::Named(NamedKey::End), false), max);
    assert_eq!(key(&mut app, Key::Named(NamedKey::Home), false), 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowDown), true), max);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowUp), true), 0.0);
    // 其他键交给编辑区。
    assert!(!app.markdown_key(&Key::Character("x".into()), false));

    // Esc 清掉选区；没有选区时不管。
    app.markdown_select_all();
    assert!(app.markdown_key(&Key::Named(NamedKey::Escape), false));
    assert!(app.markdown.selection.is_none());
    assert!(!app.markdown_key(&Key::Named(NamedKey::Escape), false));

    // 分栏时焦点在左侧文字上：方向键仍然移动光标，不滚动预览。
    app.editor.active_mut().unwrap().readonly = false;
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert!(!app.markdown_key(&Key::Named(NamedKey::PageDown), false));
    // 点击右侧预览：键盘滚动预览（左侧同步滚动）；按键输入时焦点回到左侧。
    let area = app.markdown.area;
    super::click(&mut app, (area.x + area.w / 2.0, area.y + area.h / 2.0));
    let before = app.markdown.scroll();
    assert!(app.markdown_key(&Key::Named(NamedKey::PageDown), false));
    assert!(app.markdown.scroll() > before);
    assert!(!app.markdown_key(&Key::Character("x".into()), false));
    assert!(!app.markdown_focused());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 按着 ⌘（其他平台 Ctrl）在 `at` 处单击。
fn command_click(app: &mut App, at: (f32, f32)) {
    app.modifiers = if cfg!(target_os = "macos") {
        command_modifiers()
    } else {
        winit::keyboard::ModifiersState::CONTROL
    };
    super::click(app, at);
    app.modifiers = winit::keyboard::ModifiersState::empty();
}

/// 文字光标在原文中的位置（字节偏移）之后的一段原文。
fn after_caret(app: &App, len: usize) -> String {
    let doc = app.editor.active().unwrap();
    let text = doc.text().rope().unwrap().to_string();
    let at = app.live_caret_source().unwrap();
    text[at..].chars().take(len).collect()
}

/// 光标在屏幕上的高度（窗口坐标）。
fn caret_screen_y(app: &App) -> f32 {
    let laid = app.markdown.layout.as_ref().unwrap();
    let (_, y, _) = laid.caret_point(app.live_caret_source().unwrap()).unwrap();
    app.markdown.area.y + y - app.markdown.scroll()
}

#[test]
fn the_full_preview_edits_the_document() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = readme("md-live", false, PreviewMode::Preview);
    // 刚打开时整篇都是排好的样子。
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(!shown.contains("**Rust**"));
    // 点击「启动快」：光标移到原文中对应的位置，被点击的文字位置不变。该段落仍按排版显示，
    // 只显示光标接触到的标记，即「启动快」两侧的 `*`；未接触的 `**Rust**` 不显示标记。
    let at = shown[..shown.find("启动快").unwrap()].chars().count();
    let (x, y) = point_of(&app, at);
    super::click(&mut app, (x, y));
    assert_eq!(after_caret(&app, 3), "启动快");
    let before = caret_screen_y(&app);
    paint(&mut app);
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(
        shown.contains("一个用 Rust 写的") && shown.contains("*启动快*"),
        "{shown}"
    );
    assert!(!shown.contains("# winc"));
    assert!((caret_screen_y(&app) - before).abs() < 1.0);

    // 打字改的是文档，重新解析之前预览就显示刚打的字。
    app.run(winc_editor::Command::Insert("很".into()));
    paint(&mut app);
    assert!(
        crate::view::preview::markdown::full_text(&app)
            .unwrap()
            .contains("很启动快")
    );
    settle(&mut app);
    assert!(
        app.editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .to_string()
            .contains("*很启动快*")
    );

    // ↓ 按显示的行往下走，走出这一段后到下一段；↑ 回来。
    let start = app.live_caret_source().unwrap();
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    paint(&mut app);
    let down = app.live_caret_source().unwrap();
    assert!(down > start);
    // 继续往下：经过提示块的标题（没有对应的原文，取块的开头），到它正文的最后一行；保持原来
    // 的横坐标，落在这一行的末尾。一段折成几行随字体而定，按行走到那里为止，每一步都往前。
    let mut last = down;
    for _ in 0..8 {
        if after_caret(&app, 12).starts_with("\n\n## 安装") {
            break;
        }
        assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
        paint(&mut app);
        let now = app.live_caret_source().unwrap();
        assert!(now > last);
        last = now;
    }
    assert!(
        after_caret(&app, 12).starts_with("\n\n## 安装"),
        "{}",
        after_caret(&app, 12)
    );
    // 继续向下：越过块间留白到达下一个标题（保持横坐标，落在该行末尾），
    // 不因留白高于一行而停在原处或跳到文末。
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    paint(&mut app);
    assert!(
        after_caret(&app, 5).starts_with("\n\n1. "),
        "{:?}",
        after_caret(&app, 8)
    );
    // ⇧↑ 扩展选区。
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowUp), false));
    app.modifiers = winit::keyboard::ModifiersState::empty();
    paint(&mut app);
    assert!(!app.editor.active().unwrap().selection.is_empty());
    // 其他键交给编辑区。
    assert!(!app.markdown_key(&Key::Character("x".into()), false));
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));

    // 光标到了文末：预览滚到能看见光标。
    app.run(winc_editor::Command::DocEnd { extend: false });
    paint(&mut app);
    paint(&mut app);
    let y = caret_screen_y(&app);
    let area = app.markdown.area;
    assert!(y >= area.y && y < area.bottom(), "{y} {area:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_paragraph_keeps_its_look() {
    use crate::view::preview::markdown::full_text;
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = live_doc(
        "md-reveal",
        "第一段 **粗** 和 [链接](https://a.dev)。\n\n第二段\n",
    );
    let laid = full_text(&app).unwrap();
    // 光标未接触标记时保持排版后的外观，不显示原文。
    put_caret(&mut app, "第一段", true);
    assert_eq!(full_text(&app).unwrap(), laid);
    // 只显示光标所接触元素的标记。
    put_caret(&mut app, "粗", false);
    let shown = full_text(&app).unwrap();
    assert!(
        shown.contains("**粗**") && !shown.contains("](https"),
        "{shown}"
    );
    put_caret(&mut app, "链接", true);
    let shown = full_text(&app).unwrap();
    assert!(
        shown.contains("[链接](https://a.dev)") && !shown.contains("**"),
        "{shown}"
    );
    // 段落末尾回车：光标落在下面单独的一行上。
    put_caret(&mut app, "第二段", true);
    let line = caret_screen_y(&app);
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    settle(&mut app);
    assert!(caret_screen_y(&app) > line + 10.0);
    // 输入法组字排入光标所在的行，其后的文字向后移动，不被组字覆盖。
    put_caret(&mut app, "粗", true);
    app.preedit = "中文".into();
    paint(&mut app);
    let shown = full_text(&app).unwrap();
    assert!(shown.contains("粗中文** 和"), "{shown}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 预览的滚动条能拖：只看预览和分栏时都能，不会选中文字；分栏时左边跟着滚。
#[test]
fn the_preview_scrollbar_can_be_dragged() {
    use crate::layout::{Scrollbar, scrollbar_thumb};

    for (mode, name) in [
        (PreviewMode::Preview, "md-drag-full"),
        (PreviewMode::Split, "md-drag-split"),
    ] {
        let (mut app, dir) = readme(name, false, mode);
        let area = app.markdown.area;
        let height = app.markdown.height;
        assert!(height > area.h, "{height} {}", area.h);
        let thumb = scrollbar_thumb(area, 0.0, height).unwrap();
        let (x, y) = (thumb.x + thumb.w / 2.0, thumb.y + thumb.h / 2.0);

        app.on_mouse_move(x, y);
        assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::Markdown));
        app.on_mouse_down(MouseButton::Left);
        assert!(matches!(app.drag, crate::app::Drag::Scrollbar { .. }));

        // 拖到底：滚到最后，滑块跟到底。
        app.on_mouse_move(x, area.bottom() + 100.0);
        let end = height - area.h;
        assert!((app.markdown.scroll() - end).abs() < 1.0, "{name}");
        paint(&mut app);
        assert!(app.markdown.selection.is_none());
        if mode == PreviewMode::Split {
            assert!(app.editor.active().unwrap().scroll_line > 0.0);
        }

        // 拖回顶上。
        app.on_mouse_move(x, area.y - 100.0);
        assert!(app.markdown.scroll() < 1.0, "{name}");
        app.on_mouse_up(MouseButton::Left);
        assert!(matches!(app.drag, crate::app::Drag::None));

        // 点轨道：滑块跳过去。
        app.on_mouse_move(x, area.bottom() - 2.0);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        assert!(app.markdown.scroll() > 0.0, "{name}");

        // 滚动条左边还是选字。
        app.on_mouse_move(area.x + 50.0, area.y + 50.0);
        app.on_mouse_down(MouseButton::Left);
        assert!(matches!(app.drag, crate::app::Drag::MarkdownText));
        app.on_mouse_up(MouseButton::Left);

        // 回到代码模式：同一个位置是编辑区的滚动条。
        app.set_preview_mode(PreviewMode::Source);
        settle(&mut app);
        let track = app.scroll_track();
        app.on_mouse_move(track.right() - 2.0, track.y + 4.0);
        assert_ne!(app.hover, Hover::Scrollbar(Scrollbar::Markdown));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn clicking_where_markers_appear_does_not_select() {
    let (mut app, dir) = live_doc(
        "md-reveal-click",
        "# winc 模块设计\n\n正文 **粗** 字\n\n| 名 |\n|---|\n| **甲** |\n\n```rust\nfn main() {}\n```\n",
    );
    // 点击标题开头、粗体、表格、代码块：按下时不显示标记（文字不移动），松开后才显示，
    // 因此单击不会选中一段文字。
    for (needle, marker) in [
        ("winc", "# winc"),
        ("粗", "**粗**"),
        ("甲", "**甲**"),
        ("main", "fn main() {}"),
    ] {
        let shown = crate::view::preview::markdown::full_text(&app).unwrap();
        let at = shown[..shown.find(needle).unwrap()].chars().count();
        let (x, y) = point_of(&app, at);
        app.on_mouse_move(x - 1.0, y);
        app.on_mouse_down(MouseButton::Left);
        paint(&mut app);
        app.on_mouse_move(x, y);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        assert!(app.editor.active().unwrap().selection.is_empty());
        assert!(after_caret(&app, 1) == needle[..needle.chars().next().unwrap().len_utf8()]);
        let shown = crate::view::preview::markdown::full_text(&app).unwrap();
        assert!(shown.contains(marker), "{shown}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_an_image_shows_its_source_above_it() {
    let dir = sample_folder("md-image-click");
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let path = dir.join("LIVE.md");
    std::fs::write(&path, "前\n\n![标志](logo.svg)\n\n后\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    // 点击图片中部：光标进入图片的原文，原文显示在图片上方，图片另起一行。
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    let (x, top) = point_of(&app, shown.find('前').unwrap());
    let (_, bottom) = point_of(&app, shown.chars().position(|c| c == '后').unwrap());
    app.on_mouse_move(x + 20.0, (top + bottom) / 2.0 + 20.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let at = app.live_caret_source().unwrap();
    let text = doc_text(&app);
    let image = text.find("![").unwrap();
    assert!((image..=text.find("后").unwrap() - 2).contains(&at), "{at}");
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(shown.contains("![标志](logo.svg)\n"), "{shown:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_an_empty_table_cell_puts_the_caret_in_it() {
    let (mut app, dir) = live_doc(
        "md-table-empty",
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n|  |  |\n\n后文\n",
    );
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    let at = |c: char| shown.chars().position(|s| s == c).unwrap();
    let (a, b, c) = (
        point_of(&app, at('a')),
        point_of(&app, at('b')),
        point_of(&app, at('c')),
    );
    let row = c.1 - a.1;
    let click = |app: &mut App, (x, y): (f32, f32)| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        settle(app);
    };
    // 新增行、列中的空单元格：光标进入被点击的单元格，不进入相邻的非空单元格。
    click(&mut app, (b.0 + 8.0, c.1 + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | | |\n|  |  |\n\n后文\n"
    );
    click(&mut app, (c.0 + 8.0, c.1 + row + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n| | |  |\n\n后文\n"
    );
    click(&mut app, (b.0 + 8.0, c.1 + row + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n|  | | |\n\n后文\n"
    );
    // 点击非空单元格中的空白处：光标仍在该单元格。
    click(&mut app, (a.0 - 6.0, a.1 + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| |a | b |\n|---|---|\n| c |  |\n|  |  |\n\n后文\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_table_cells_once_keeps_the_page_still() {
    let filler = "前文\n\n".repeat(40);
    let (mut app, dir) = live_doc(
        "md-table-click",
        &format!("{filler}| a | b |\n|---|---|\n| c |  |\n|  |  |\n\n{filler}"),
    );
    // 将表格滚动到屏幕中部。
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    let a = shown.chars().position(|s| s == 'a').unwrap();
    let top = crate::view::preview::markdown::offset_top(&app, a).unwrap();
    app.markdown.scroll_to(top - app.markdown.area.h / 2.0);
    settle(&mut app);
    let at = |c: char| shown.chars().position(|s| s == c).unwrap();
    let (a, b, c) = (
        point_of(&app, at('a')),
        point_of(&app, at('b')),
        point_of(&app, at('c')),
    );
    let row = c.1 - a.1;
    app.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 2);
    let table = |caret: &str| format!("{filler}{caret}\n\n{filler}");
    // 模拟实际点击：按住鼠标期间绘制数帧并小幅移动指针，松开后再绘制。
    // 单击一次即进入该单元格，页面不滚动。
    let scroll = app.markdown.scroll();
    let mut click = |(x, y): (f32, f32), want: &str| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        paint(&mut app);
        app.on_mouse_move(x + 1.0, y + 1.0);
        paint(&mut app);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        assert_eq!(with_caret(&app), table(want));
        assert_eq!(app.markdown.scroll(), scroll, "{want}");
    };
    click(
        (b.0 + 8.0, c.1 + 4.0),
        "| a | b |\n|---|---|\n| c | | |\n|  |  |",
    );
    click(
        (c.0 + 8.0, c.1 + row + 4.0),
        "| a | b |\n|---|---|\n| c |  |\n| | |  |",
    );
    click(
        (c.0 + 1.0, c.1 + 4.0),
        "| a | b |\n|---|---|\n| |c |  |\n|  |  |",
    );
    click(
        (b.0 + 8.0, c.1 + row + 4.0),
        "| a | b |\n|---|---|\n| c |  |\n|  | | |",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_table_cell_takes_one_click_from_anywhere() {
    use winc_preview::markdown::cells;
    let filler = "前文\n\n".repeat(40);
    let table = "| 名称 | `code` | **粗** |\n|:--|:-:|--:|\n| 中文内容比较长的一格 |  | x |\n\
                 |  |  |  |\n";
    let (mut app, dir) = live_doc(
        "md-table-every",
        &format!("{filler}上面 **粗** 段\n\n{table}\n下面 `码` 段\n\n{filler}"),
    );
    app.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 2);
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    let a = shown
        .find("名称")
        .map(|b| shown[..b].chars().count())
        .unwrap();
    let top = crate::view::preview::markdown::offset_top(&app, a).unwrap();
    app.markdown.scroll_to(top - app.markdown.area.h / 2.0);
    settle(&mut app);
    let start = doc_text(&app).find("| 名称").unwrap();
    // 光标所在的行号（不计 `|---|`）和列号。
    let cell_of = |app: &App| {
        let text = doc_text(app);
        let caret = app.live_caret_source()?;
        let mut at = start;
        let mut row = 0;
        for line in text[start..].split('\n').take(4) {
            if (at..=at + line.len()).contains(&caret) {
                let col = cells::cells(line)
                    .iter()
                    .position(|c| caret - at <= c.segment.end)?;
                return Some((row, col));
            }
            if !cells::is_delimiter(line) {
                row += 1;
            }
            at += line.len() + 1;
        }
        None
    };
    let click = |app: &mut App, (x, y): (f32, f32)| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        paint(app);
        app.on_mouse_move(x + 1.0, y + 1.0);
        paint(app);
        app.on_mouse_up(MouseButton::Left);
        settle(app);
    };
    let grid = crate::view::preview::markdown::tests::table_cells(&app);
    assert_eq!((grid.len(), grid[0].len()), (3, 3));
    let mut failures = Vec::new();
    for from in ["上面", "下面", "x"] {
        for r in 0..3 {
            for c in 0..3 {
                put_caret(&mut app, from, true);
                let scroll = app.markdown.scroll();
                let grid = crate::view::preview::markdown::tests::table_cells(&app);
                let cell = grid[r][c];
                // 单元格中部和左侧（空单元格的光标位于左侧）。
                for (x, y) in [
                    (cell.x + cell.w / 2.0, cell.y + cell.h / 2.0),
                    (cell.x + 3.0, cell.y + cell.h / 2.0),
                ] {
                    click(&mut app, (x, y));
                    let got = cell_of(&app);
                    let moved = app.markdown.scroll() - scroll;
                    // 新显示的标记正常淡入；块不应滑动。
                    let motion = app
                        .markdown
                        .motion
                        .as_ref()
                        .is_some_and(|m| m.from.iter().any(|d| d.abs() >= 0.5));
                    if got != Some((r, c)) || moved != 0.0 || motion {
                        failures.push(format!(
                            "从 {from} 点 ({r},{c}) 于 ({x:.0},{y:.0})：进了 {got:?}，\
                             滚了 {moved}，滑动 {motion}"
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 打开内容为 `text` 的 Markdown 文件，只看预览，等解析、排版完成。
fn live_doc(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("LIVE.md");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    (app, dir)
}

/// 回归：打字之后、重新解析之前，光标所在块的范围来自上一版文档。文档变短或范围落到多字节字符
/// 中间时，曾在绘制或按回车时 panic，发布版中表现为闪退。
#[test]
fn edits_before_the_next_parse_keep_the_live_block_inside_the_text() {
    use winit::keyboard::{Key, NamedKey};

    let text =
        "第一段中文\n\n第二段中文内容\n\n| 表头 | 中 |\n| - | - |\n| 一 | 二 |\n\n最后一段\n";
    for keep in ["", "中", "中文\n\n表", "第一段中文\n\n第"] {
        let (mut app, dir) = live_doc("md-stale", text);
        let doc = app.editor.active_mut().unwrap();
        let count = doc.text().rope().unwrap().len_chars();
        doc.selection = winc_core::Selection::new(0, count);
        // 整篇换成更短的文字，不等重新解析：后台的解析结果还留在上一版。
        app.run(winc_editor::Command::Insert(keep.into()));
        let count = app
            .editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .len_chars();
        for at in 0..=count {
            app.editor.active_mut().unwrap().selection = winc_core::Selection::point(at);
            let rope = app.editor.active().unwrap().text().rope().unwrap().clone();
            if let Some(live) = app.live_leaf() {
                let _ = rope.byte_slice(live.range.clone());
                let _ = rope.byte_slice(live.leaf.clone());
            }
            app.markdown_key(&Key::Named(NamedKey::Enter), false);
            paint(&mut app);
            app.run(winc_editor::Command::Undo);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn deeply_nested_documents_lay_out_and_paint() {
    // 嵌套超过上限的引用、列表、公式：排版和绘制同样按层递归，测试线程与后台线程的栈都是 2 MB。
    let lists: String = (0..300)
        .map(|i| format!("{}- x{i}\n", "  ".repeat(i)))
        .collect();
    let text = format!(
        "{} deep\n\n{lists}\n$${}x$$\n\n${}x$\n",
        ">".repeat(20_000),
        "{".repeat(50_000),
        "\\hat".repeat(50_000),
    );
    let (app, dir) = live_doc("md-deep", &text);
    assert!(app.markdown.height > app.markdown.area.h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文档现在的文字。
fn doc_text(app: &App) -> String {
    app.editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string()
}

/// 光标放到第一处 `needle` 之后（`after` 为假时之前），等排版完成。
fn put_caret(app: &mut App, needle: &str, after: bool) {
    let text = doc_text(app);
    let byte = text.find(needle).unwrap() + if after { needle.len() } else { 0 };
    let doc = app.editor.active_mut().unwrap();
    let pos = doc.text().rope().unwrap().byte_to_char(byte);
    doc.selection = winc_core::Selection::point(pos);
    settle(app);
}

/// 文档中光标处插着 `|` 的文字。
fn with_caret(app: &App) -> String {
    let mut text = doc_text(app);
    text.insert(app.live_caret_source().unwrap(), '|');
    text
}

#[test]
fn enter_after_a_fence_closes_the_block() {
    use winit::keyboard::{Key, NamedKey};
    let enter = Key::Named(NamedKey::Enter);

    let (mut app, dir) = live_doc("md-fence", "前文\n\n后文\n");
    put_caret(&mut app, "前文", true);
    for text in ["\n\n", "```rust"] {
        app.run(winc_editor::Command::Insert(text.into()));
    }
    // 尚未解析（仍为正文段落）时同样补全闭围栏，光标位于两行围栏之间；后文不会变为代码。
    assert!(app.markdown_key(&enter, false));
    assert_eq!(with_caret(&app), "前文\n\n```rust\n|\n```\n\n后文\n");
    settle(&mut app);
    app.run(winc_editor::Command::Insert("fn x() {}".into()));
    settle(&mut app);
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(
        shown.contains("fn x() {}") && shown.contains("后文"),
        "{shown}"
    );
    // 在已闭合的围栏上按回车：按普通换行处理。
    put_caret(&mut app, "```rust", true);
    let _ = app.markdown_key(&enter, false);
    assert_eq!(doc_text(&app).matches("```").count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backspace_after_a_marker_removes_it() {
    use winit::keyboard::{Key, NamedKey};
    let backspace = Key::Named(NamedKey::Backspace);

    let (mut app, dir) = live_doc(
        "md-backspace",
        "- [ ] 甲\n- 乙\n\n> 引\n\n```\n- x\n```\n\n## 题\n",
    );
    put_caret(&mut app, "甲", false);
    // 逐层移除任务框和列表记号；「乙」成为上一项中的另一段。
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).starts_with("- |甲\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    put_caret(&mut app, "乙", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).contains("\n  |乙\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    put_caret(&mut app, "引", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(with_caret(&app).contains("\n|引\n"), "{}", with_caret(&app));
    settle(&mut app);
    // 标题的 `#` 全部移除，成为段落。
    put_caret(&mut app, "题", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).ends_with("\n|题\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    // 光标前还有文字或位于代码块中时，按普通删除处理，删除一个字符。
    put_caret(&mut app, "引", true);
    assert!(!app.markdown_key(&backspace, false));
    put_caret(&mut app, "x", false);
    assert!(!app.markdown_key(&backspace, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tab_nests_list_items_in_the_preview() {
    use winit::keyboard::{Key, NamedKey};
    let tab = Key::Named(NamedKey::Tab);

    let (mut app, dir) = live_doc("md-tab", "- 一\n- 二\n\n段落\n");
    put_caret(&mut app, "二", true);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(with_caret(&app), "- 一\n  - 二|\n\n段落\n");
    settle(&mut app);
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&tab, false));
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(with_caret(&app), "- 一\n- 二|\n\n段落\n");
    settle(&mut app);
    // 第一项无法缩进，也不向文字中插入空格。
    put_caret(&mut app, "一", false);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(with_caret(&app), "- |一\n- 二\n\n段落\n");
    // 不是列表项时交给编辑区处理。
    put_caret(&mut app, "段落", true);
    assert!(!app.markdown_key(&tab, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enter_in_the_preview_continues_lists_and_splits_paragraphs() {
    use winit::keyboard::{Key, NamedKey};
    let enter = Key::Named(NamedKey::Enter);

    let (mut app, dir) = live_doc("md-enter", "- 一\n- 二\n\n段落一\n\n```\ncode\n```\n");
    put_caret(&mut app, "二", true);
    // 列表里回车续一项，空项再回车结束列表、另起一段。
    assert!(app.markdown_key(&enter, false));
    assert_eq!(
        with_caret(&app),
        "- 一\n- 二\n- |\n\n段落一\n\n```\ncode\n```\n"
    );
    settle(&mut app);
    assert!(app.markdown_key(&enter, false));
    app.run(winc_editor::Command::Insert("新".into()));
    assert_eq!(
        with_caret(&app),
        "- 一\n- 二\n\n新|\n\n段落一\n\n```\ncode\n```\n"
    );
    // 段落中回车另起一段，⇧ 回车段内换行。
    put_caret(&mut app, "段落", true);
    assert!(app.markdown_key(&enter, false));
    assert!(with_caret(&app).contains("段落\n\n|一"));
    settle(&mut app);
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&enter, false));
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert!(
        with_caret(&app).contains("段落\n\n\n|一"),
        "{}",
        with_caret(&app)
    );
    // 代码块里交给编辑区照常换行。
    put_caret(&mut app, "code", true);
    assert!(!app.markdown_key(&enter, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn arrows_skip_the_blank_lines_between_blocks() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = live_doc("md-gap", "甲\n\n\n乙\n");
    put_caret(&mut app, "甲", true);
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowRight), false));
    assert_eq!(with_caret(&app), "甲\n\n\n|乙\n");
    settle(&mut app);
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));
    assert_eq!(with_caret(&app), "甲|\n\n\n乙\n");
    settle(&mut app);
    // 块里面逐字移动交给编辑区。
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));
    // 文末之后没有块：停在空行上（交给编辑区）。
    put_caret(&mut app, "乙", true);
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowRight), false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pasting_a_url_over_selected_text_makes_a_link() {
    let (mut app, dir) = live_doc("md-paste-link", "见文档。\n\n`代码`\n\n```\n块\n```\n");
    let select = |app: &mut App, needle: &str| {
        let text = doc_text(app);
        let byte = text.find(needle).unwrap();
        let doc = app.editor.active_mut().unwrap();
        let rope = doc.text().rope().unwrap();
        let (a, b) = (
            rope.byte_to_char(byte),
            rope.byte_to_char(byte + needle.len()),
        );
        doc.selection = winc_core::Selection::new(a, b);
    };
    select(&mut app, "文档");
    assert_eq!(
        app.markdown_link_paste(" https://a.dev/x\n").as_deref(),
        Some("[文档](https://a.dev/x)")
    );
    assert_eq!(
        app.markdown_link_paste("https://a.dev/(1)").as_deref(),
        Some("[文档](<https://a.dev/(1)>)")
    );
    // 不是网址、没有选区、在行内代码或代码块里时照常粘贴。
    assert!(app.markdown_link_paste("文字").is_none());
    assert!(app.markdown_link_paste("https://a.dev x").is_none());
    select(&mut app, "代码");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    select(&mut app, "块");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    put_caret(&mut app, "见", true);
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    // 切换回源代码：预览已释放解析结果，按围栏判断代码块。
    app.set_preview_mode(PreviewMode::Source);
    settle(&mut app);
    assert!(app.markdown.parsed.is_none() && app.markdown.layout.is_none());
    select(&mut app, "块");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    select(&mut app, "文档");
    assert!(app.markdown_link_paste("https://a.dev").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_a_task_box_toggles_it() {
    let (mut app, dir) = live_doc("md-task", "开头\n\n- [ ] 甲\n- [x] 乙\n");
    let task = crate::view::preview::markdown::TASK;
    let boxes = |app: &App| -> Vec<winc_render::Rect> {
        app.markdown
            .links
            .iter()
            .filter(|(_, url)| url.starts_with(task))
            .map(|(r, _)| *r)
            .collect()
    };
    let caret = app.editor.active().unwrap().selection;
    let first = boxes(&app)[0];
    super::click(&mut app, (first.x + first.w / 2.0, first.y + first.h / 2.0));
    settle(&mut app);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [x] 乙\n");
    // 光标不移动，也不因此进入编辑状态：列表仍按排版显示，勾选框保留。
    assert_eq!(app.editor.active().unwrap().selection, caret);
    let second = boxes(&app)[1];
    super::click(&mut app, (second.x + 2.0, second.y + 2.0));
    settle(&mut app);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [ ] 乙\n");
    app.run(winc_editor::Command::Undo);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [x] 乙\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_modes_keeps_the_place() {
    let text: String = (0..60)
        .map(|i| format!("## 第 {i} 节\n\n第 {i} 节的正文。\n\n"))
        .collect();
    let (mut app, dir) = live_doc("md-switch", &text);
    // 预览滚到第 30 节：换成源代码后文字顶上是第 30 节的标题（第 120 行）。
    let y = crate::view::preview::markdown::anchor_top(&app, "第-30-节").unwrap();
    app.scroll_markdown_to(y);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    let line = app.editor.active().unwrap().scroll_line;
    assert!((line - 120.0).abs() < 2.0, "{line}");
    // 文字滚到第 10 节，换回只看预览：预览顶上是第 10 节，光标所在的块不显示原文。
    app.editor.active_mut().unwrap().scroll_line = 40.0;
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let y = crate::view::preview::markdown::anchor_top(&app, "第-10-节").unwrap();
    // 差值来自预览顶部的留白，分栏同步滚动的映射同样包含这段留白。
    let pad = app.markdown.scroll() - y;
    assert!((0.0..=40.0).contains(&pad), "{pad}");
    assert!(
        !crate::view::preview::markdown::full_text(&app)
            .unwrap()
            .contains("## ")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_across_blocks_keeps_the_layout_still() {
    let (mut app, dir) = live_doc("md-drag", "第一段文字\n\n第二段文字\n\n第三段文字\n");
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    let at = |s: &str| shown[..shown.find(s).unwrap()].chars().count();
    let (x, y) = point_of(&app, at("第一段") + 1);
    let (x2, y2) = point_of(&app, at("第三段") + 1);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    paint(&mut app);
    // 按住鼠标拖动期间排版不变（尚未开始编辑，全部按排版显示），鼠标下方的内容不移位。
    assert!(app.live_leaf().is_none());
    app.on_mouse_move(x2, y2 + 1.0);
    paint(&mut app);
    assert!(app.live_leaf().is_none());
    assert_eq!(
        crate::view::preview::markdown::full_text(&app).unwrap(),
        shown
    );
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let text = doc_text(&app);
    let rope = doc.text().rope().unwrap();
    let selected =
        &text[rope.char_to_byte(doc.selection.from())..rope.char_to_byte(doc.selection.to())];
    assert!(
        selected.starts_with("一段") && selected.contains("第二段文字"),
        "{selected}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tables_are_edited_cell_by_cell() {
    use winit::keyboard::{Key, NamedKey};
    let tab = Key::Named(NamedKey::Tab);

    let (mut app, dir) = live_doc("md-cells", "| a | b |\n|---|---|\n| **粗** | d |\n\n后文\n");
    put_caret(&mut app, "粗", false);
    app.run(winc_editor::Command::Insert("很".into()));
    paint(&mut app);
    // 只有光标所在的那一格显示原文，表格照常排。
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(shown.contains("**很粗**"), "{shown}");
    assert!(
        !shown.contains("|---|") && !shown.contains("| a |"),
        "{shown}"
    );
    let selected = |app: &App| {
        let doc = app.editor.active().unwrap();
        let rope = doc.text().rope().unwrap();
        rope.slice(doc.selection.from()..doc.selection.to())
            .to_string()
    };
    // Tab 选中下一格，最后一格再 Tab 添一行；⇧Tab 回去。
    assert!(app.markdown_key(&tab, false));
    assert_eq!(selected(&app), "d");
    settle(&mut app);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(
        with_caret(&app),
        "| a | b |\n|---|---|\n| **很粗** | d |\n| | |  |\n\n后文\n"
    );
    settle(&mut app);
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&tab, false));
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(selected(&app), "d");
    settle(&mut app);
    // 回车到下一行的同一列。
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    assert_eq!(
        with_caret(&app),
        "| a | b |\n|---|---|\n| **很粗** | d |\n|  | | |\n\n后文\n"
    );
    // 表头下面那一行上：整张表显示原文，Tab 交给编辑区。
    put_caret(&mut app, "|---", true);
    assert!(
        crate::view::preview::markdown::full_text(&app)
            .unwrap()
            .contains("|---|")
    );
    assert!(!app.markdown_key(&tab, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn code_fences_show_only_with_the_caret_on_them() {
    let (mut app, dir) = live_doc("md-fences", "前\n\n```rust\nfn x() {}\n```\n");
    put_caret(&mut app, "fn x", false);
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(shown.contains("fn x") && !shown.contains("```"), "{shown}");
    // 按 ← 移到围栏行：显示围栏，可以修改语言标识。
    app.run(winc_editor::Command::MoveLeft {
        extend: false,
        word: false,
    });
    settle(&mut app);
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    assert!(shown.contains("```rust\nfn x"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_table_toolbar_adds_aligns_and_deletes() {
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    let (mut app, dir) = live_doc(
        "md-table-tools",
        "前文\n\n| a | b |\n|---|---|\n| c | d |\n\n后文\n",
    );
    put_caret(&mut app, "c", true);
    // 光标在表格中时上方显示工具栏，单击按钮执行操作，不移动光标。
    let click = |app: &mut App, op: &str| {
        let url = format!("{}{op}", crate::view::preview::markdown::TABLE);
        let (rect, _) = app
            .markdown
            .links
            .iter()
            .find(|(_, u)| *u == url)
            .cloned()
            .unwrap_or_else(|| panic!("工具栏上有 {op}"));
        super::click(app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
        settle(app);
    };
    click(&mut app, "column");
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a |  | b |\n|---|---|---|\n| c | | | d |\n\n后文\n"
    );
    click(&mut app, "center");
    assert!(doc_text(&app).contains("|---|:---:|---|"));
    // 再次点击已选中的对齐方式：取消对齐。
    click(&mut app, "none");
    assert!(doc_text(&app).contains("|---|---|---|"));
    click(&mut app, "delete-column");
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | |d |\n\n后文\n"
    );
    // ⌘↩ 在下方插入一行，⌘⇧⌫ 删除当前行；一次撤销即可恢复。
    app.modifiers = command_modifiers();
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), true));
    app.modifiers = ModifiersState::empty();
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | d |\n|  | | |\n\n后文\n"
    );
    app.modifiers = command_modifiers() | ModifiersState::SHIFT;
    assert!(app.markdown_key(&Key::Named(NamedKey::Backspace), true));
    app.modifiers = ModifiersState::empty();
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | |d |\n\n后文\n"
    );
    click(&mut app, "delete");
    assert_eq!(doc_text(&app), "前文\n\n\n后文\n");
    app.run(winc_editor::Command::Undo);
    assert!(doc_text(&app).contains("| c | d |"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_edits_do_not_scroll_the_page() {
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    let filler = "前文\n\n".repeat(60);
    let (mut app, dir) = live_doc(
        "md-table-still",
        &format!("{filler}| a | b |\n|---|---|\n| c | d |\n\n{filler}"),
    );
    put_caret(&mut app, "c", true);
    // 将表格滚动到屏幕中部：光标不会移出可见区域，页面无需随之滚动。
    let view = app.markdown.area.h;
    app.markdown.scroll_to(app.markdown.scroll() + view / 2.0);
    settle(&mut app);
    let scroll = app.markdown.scroll();
    assert!(scroll > view, "表格在第一屏以下");
    // 插入行、删除行、删除表格都是文字修改：上方内容不移动，页面不滚动。
    let key = |app: &mut App, mods: ModifiersState, key: NamedKey| {
        app.modifiers = mods;
        assert!(app.markdown_key(&Key::Named(key), true));
        app.modifiers = ModifiersState::empty();
        // 先绘制一帧（解析结果尚未返回），再等待解析完成。
        paint(app);
        settle(app);
    };
    key(&mut app, command_modifiers(), NamedKey::Enter);
    assert!(doc_text(&app).contains("|  |  |"));
    assert_eq!(app.markdown.scroll(), scroll, "加行");
    let shift = command_modifiers() | ModifiersState::SHIFT;
    key(&mut app, shift, NamedKey::Backspace);
    assert_eq!(app.markdown.scroll(), scroll, "删行");
    assert!(app.live_table_tool("delete"));
    paint(&mut app);
    settle(&mut app);
    assert!(!doc_text(&app).contains("| c |"));
    assert_eq!(app.markdown.scroll(), scroll, "删表格");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edits_slide_the_blocks_below_and_fade_in_markers() {
    let half = crate::app::animation::MARKDOWN_MOTION / 2;
    let (mut app, dir) = live_doc(
        "md-motion",
        "正文 **粗** 字\n\n| a |\n|---|\n| b |\n| c |\n\n后文\n",
    );
    app.markdown.frozen = Some(half);
    // 光标移入粗体（未修改文字）：两侧的 `**` 淡入，块高度不变，不滑动。
    put_caret(&mut app, "字", false);
    put_caret(&mut app, "粗", false);
    let motion = app.markdown.motion.as_ref().expect("标记淡入");
    assert!(!motion.moving());
    assert_eq!(motion.fading.len(), 1);
    assert_eq!(motion.fading[0].1.len(), 2);
    // 输入文字时不淡入。
    app.run(winc_editor::Command::Insert("很".into()));
    settle(&mut app);
    assert!(
        app.markdown
            .motion
            .as_ref()
            .is_none_or(|m| m.fading.is_empty())
    );
    // 删除表格的一行：表格变矮，后文从原位置向上滑动，停在过渡的中点。
    put_caret(&mut app, "c", false);
    assert!(app.live_table_tool("delete-row"));
    settle(&mut app);
    let motion = app.markdown.motion.as_ref().expect("后文往上滑");
    let last = motion.from.len() - 1;
    assert!(motion.from[last] > 10.0, "{:?}", motion.from);
    let offset = motion.offset(last);
    assert!(offset > 0.0 && offset < motion.from[last], "{offset}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn math_and_diagrams_show_the_result_below_the_source() {
    let height = |text: &str| {
        let (mut app, dir) = live_doc("md-below", text);
        put_caret(&mut app, "A", false);
        app.run(winc_editor::Command::Insert(" ".into()));
        settle(&mut app);
        let shown = crate::view::preview::markdown::full_text(&app).unwrap();
        // 光标不在围栏行上时只显示围栏之间的原文，不显示围栏。
        assert!(shown.contains(" A") && !shown.contains("```"), "{shown}");
        assert!(!shown.contains("$$"), "{shown}");
        let height = app.markdown.height;
        let _ = std::fs::remove_dir_all(&dir);
        (height, shown)
    };
    // 原文下方绘制渲染后的图，因此比只显示原文的代码块更高。
    let (diagram, _) = height("```mermaid\ngraph TD\nA-->B\n```\n");
    let (plain, _) = height("```text\ngraph TD\nA-->B\n```\n");
    assert!(diagram > plain + 40.0, "{diagram} {plain}");
    // 公式的原文下面是排好的公式（复制的文字是公式的原文，跟在原文后面）。
    let (_, shown) = height("$$\nA^2 + b\n$$\n");
    assert_eq!(shown.matches("A^2").count(), 2, "{shown}");
}

#[test]
fn clicking_an_anchor_link_scrolls_to_the_heading() {
    let (mut app, dir) = readme("md-anchor", false, PreviewMode::Preview);
    let index = app
        .markdown
        .links
        .iter()
        .position(|(_, url)| url == "#安装")
        .expect("有转到「安装」的链接");
    let (rect, _) = app.markdown.links[index].clone();
    // 可编辑的预览中单击链接只放光标。
    super::click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    assert_eq!(app.markdown.scroll(), 0.0);
    command_click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    let top = crate::view::preview::markdown::anchor_top(&app, "安装").unwrap();
    assert!(top > 0.0);
    // 标题滚到顶上；文档剩下的不够一屏时滚到底为止。
    let max = app.markdown.height - app.markdown.area.h;
    assert_eq!(app.markdown.scroll(), top.min(max));
    // 脚注的编号和末尾的 ↩ 能找到彼此。
    let note = crate::view::preview::markdown::anchor_top(&app, "fn-1").expect("脚注有锚点");
    let back = crate::view::preview::markdown::anchor_top(&app, "fnref-1").expect("能跳回引用处");
    assert!(back > top && note > back);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn details_open_and_close_on_click() {
    let dir = sample_folder("md-details");
    let path = dir.join("README.md");
    std::fs::write(
        &path,
        "开头\n\n<details>\n<summary>点开看</summary>\n\n藏起来的话\n\n</details>\n\n结尾\n",
    )
    .unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let shown = |app: &App| {
        crate::view::preview::markdown::full_text(app)
            .unwrap()
            .contains("藏起来的话")
    };
    assert!(!shown(&app));
    let height = app.markdown.height;
    let toggle = |app: &mut App| {
        let (rect, _) = app
            .markdown
            .links
            .iter()
            .find(|(_, url)| url.starts_with(crate::view::preview::markdown::DETAILS))
            .cloned()
            .expect("折叠块的标题能点");
        super::click(app, (rect.x + 4.0, rect.y + 4.0));
        settle(app);
    };
    toggle(&mut app);
    assert!(shown(&app));
    assert!(app.markdown.height > height);
    toggle(&mut app);
    assert!(!shown(&app));
    assert_eq!(app.markdown.height, height);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn code_blocks_offer_a_copy_button_on_hover() {
    let (mut app, dir) = readme("md-copy-code", false, PreviewMode::Preview);
    let copy = crate::view::preview::markdown::COPY;
    assert!(
        !app.markdown
            .links
            .iter()
            .any(|(_, url)| url.starts_with(copy))
    );
    let code = *app.markdown.codes.first().expect("画出了代码块");
    app.on_mouse_move(code.x + 20.0, code.y + 20.0);
    paint(&mut app);
    let (rect, _) = app
        .markdown
        .links
        .iter()
        .find(|(_, url)| url.starts_with(copy))
        .cloned()
        .expect("鼠标在代码块上时有复制按钮");
    super::click(&mut app, (rect.x + 4.0, rect.y + 4.0));
    let copied = app.copied.clone().unwrap();
    assert!(copied.starts_with("fn main() {\n"), "{copied:?}");
    assert!(copied.trim_end().ends_with('}'));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn links_to_headings_in_other_files_open_there() {
    let dir = sample_folder("md-other-file");
    let path = dir.join("README.md");
    std::fs::write(&path, "见 [用法](docs/guide.md#用法)。\n").unwrap();
    let mut guide = String::from("# 指南\n\n");
    for i in 0..80 {
        guide.push_str(&format!("第 {i} 段。\n\n"));
    }
    guide.push_str("## 用法\n\n照着做。\n");
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/guide.md"), guide).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let (rect, _) = app.markdown.links[0].clone();
    command_click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.path.as_ref().unwrap().ends_with("docs/guide.md"));
    assert_eq!(doc.preview_mode(), PreviewMode::Preview);
    let top = crate::view::preview::markdown::anchor_top(&app, "用法").unwrap();
    assert!(top > 0.0);
    assert!(app.markdown.pending_anchor.is_none());
    let max = app.markdown.height - app.markdown.area.h;
    assert_eq!(app.markdown.scroll(), top.min(max));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_text_can_be_selected_and_copied() {
    use super::editor::navigation::press;
    use crate::keymap::KeyCode::Char;

    // 分栏时点了预览：选的是排出来的字。可编辑的预览中选的是原文，见
    // `the_full_preview_edits_the_document`。
    let (mut app, dir) = readme("md-select", false, PreviewMode::Split);
    let area = app.markdown.area;
    super::click(&mut app, (area.x + 5.0, area.bottom() - 5.0));
    assert!(app.markdown_focused());
    // ⌘A ⌘C：复制整份文字，段落之间换行，中文软换行处不插入空格，代码保持原样。
    assert!(press(&mut app, &["cmd"], Char('a')));
    assert!(press(&mut app, &["cmd"], Char('c')));
    let all = app.copied.clone().unwrap();
    assert!(
        all.starts_with("winc 编辑器\n一个用 Rust 写的轻量编辑器，启动快"),
        "{all}"
    );
    assert!(all.contains("混排时按字换行，链接比如 GitHub 和 安装。"));
    assert!(all.contains("fn main() {\n    // 打印一行\n"));
    assert!(all.contains("下载发行版\n解压到 /Applications\n"));

    // 拖着选一段（从「Rust」的 R 后面起）；双击选一个词。
    let at = all[..all.find("Rust").unwrap()].chars().count() + 1;
    let (x, y) = point_of(&app, at);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    // 拖到「混排」之后：按字的位置找，不按行数算（各系统的字体宽度不同，折行的位置也不同）。
    let end = all[..all.find("混排").unwrap()].chars().count() + 2;
    let (end_x, end_y) = point_of(&app, end);
    app.on_mouse_move(end_x, end_y);
    app.on_mouse_up(MouseButton::Left);
    let dragged = app.markdown_selected_text().unwrap();
    assert!(
        dragged.starts_with("ust 写的") && dragged.contains("混排"),
        "{dragged}"
    );
    super::click(&mut app, (x, y));
    assert_eq!(app.markdown_selected_text().as_deref(), Some("Rust"));

    // 点击预览外部（文件树）：清除选区。
    let tree = app.layout.tree;
    super::click(&mut app, (tree.x + 10.0, tree.y + 10.0));
    assert!(app.markdown.selection.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_and_source_point_at_each_other() {
    use super::editor::navigation::press;
    use crate::keymap::KeyCode::Char;

    let (mut app, dir) = readme("md-locate", false, PreviewMode::Split);
    assert!(press(&mut app, &["cmd"], Char('a')));
    let shown = crate::view::preview::markdown::full_text(&app).unwrap();
    // 样式标记、嵌套列表、代码块、表格、引用中的软换行和脚注都能对应到原文；点击链接是打开
    // 链接，不在此列。
    for target in [
        "启动快",
        "旧的写法",
        "第一次打开",
        "打印一行",
        "隔行铺底色",
        "可以有多行",
        "这是脚注",
    ] {
        let at = shown[..shown.find(target).unwrap()].chars().count();
        // 先滚到看得见。
        app.select_in_markdown(at, at + 1);
        paint(&mut app);
        let (x, y) = point_of(&app, at);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        paint(&mut app);
        let expected = SAMPLE[..SAMPLE.find(target).unwrap()].chars().count();
        let doc = app.editor.active().unwrap();
        assert_eq!(doc.selection, Selection::point(expected), "{target}");

        // 反过来：光标所在的字在预览上落在点中的那一行。
        let source = app.markdown_caret_source().unwrap();
        let laid = app.markdown.layout.as_ref().unwrap();
        let (_, top, _) = crate::view::preview::markdown::point_of(laid, source).expect(target);
        let line = crate::view::preview::markdown::offset_top(&app, at).expect(target);
        assert_eq!(top, line, "{target}");
    }

    // 只看预览时右键「在源代码中显示」：改为分栏，光标放到点中的字上。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let at = shown[..shown.find("打印一行").unwrap()].chars().count();
    app.select_in_markdown(at, at + 1);
    paint(&mut app);
    let (x, y) = point_of(&app, at);
    assert!(app.markdown_context_target(x, y));
    app.reveal_markdown_source();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    let expected = SAMPLE[..SAMPLE.find("打印一行").unwrap()].chars().count();
    assert_eq!(doc.selection, Selection::point(expected));
    assert!(!app.markdown.focused);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回预览上按下位置落在字符偏移 `offset` 之前的一点（逻辑像素）。
///
/// # Panics
///
/// 预览上找不到落在该字符上的一点时 panic。
fn point_of(app: &App, offset: usize) -> (f32, f32) {
    let area = app.markdown.area;
    // 在预览上逐行扫描，寻找落在该字符上的位置。
    let mut y = area.y;
    while y < area.bottom() {
        let mut x = area.x;
        while x < area.right() {
            if crate::view::preview::markdown::offset_at(app, x, y) == Some(offset)
                && crate::view::preview::markdown::offset_at(app, x + 4.0, y) == Some(offset + 1)
            {
                return (x, y);
            }
            x += 2.0;
        }
        y += 4.0;
    }
    panic!("预览上未找到第 {offset} 个字符");
}

#[test]
fn command_f_searches_the_rendered_preview() {
    // 分栏时点了预览：查找排出来的字。
    let (mut app, dir) = readme("md-find", false, PreviewMode::Split);
    let area = app.markdown.area;
    super::click(&mut app, (area.x + 5.0, area.bottom() - 5.0));
    assert!(app.markdown_focused());
    app.open_search(false);
    assert!(app.search_open);
    app.find_input.set_text("预览");
    app.search.query = "预览".into();
    app.search_changed();
    paint(&mut app);
    // 「预览」在渲染后的文字里出现两次（表格一格、提示块一句），源码中的写法不计入。
    assert_eq!(app.search.matches().len(), 2);
    assert_eq!(app.markdown_selected_text().as_deref(), Some("预览"));
    let first = app.markdown.selection;
    app.find_next(false);
    assert_ne!(app.markdown.selection, first);
    // 第二处在表格里，预览滚动到它的位置。
    assert!(app.markdown.scroll() > 0.0);
    app.find_next(false);
    assert_eq!(app.markdown.selection, first);
    // 预览没有「行」：「只显示匹配的行」按了没反应。
    app.press_search_button(crate::layout::SearchButton::Filter);
    assert!(!app.filter_lines);
    // 排出来的字不能改：不展开替换。
    app.close_search();
    app.open_search(true);
    assert!(!app.replace_open);
    app.close_search();

    // 点击左侧文字后查找文字。
    let text = app.layout.text;
    super::click(&mut app, (text.x + 40.0, text.y + 5.0));
    assert!(!app.markdown_focused());
    app.open_search(false);
    app.search_changed();
    assert_eq!(app.search.document(), app.editor.active().map(|d| d.id));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remote_images_come_from_the_cache_and_can_be_turned_off() {
    use crate::app::preview::markdown::{ImageSlot, image_path};
    use crate::layout::Setting;

    // 一个不会联网的地址：先把已下载的文件放进缓存，预览直接读取缓存。
    let url = format!("https://example.invalid/{}/badge", std::process::id());
    let cached = image_path(None, &url, true).unwrap();
    assert_eq!(image_path(None, &url, false), None);
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, LOGO).unwrap();

    let dir = sample_folder("md-remote");
    let path = dir.join("README.md");
    std::fs::write(&path, format!("![徽章]({url})\n")).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    assert!(matches!(
        app.markdown.images.get(&cached),
        Some(ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));
    assert_eq!(app.markdown.layout.as_ref().unwrap().chars(), 0);

    // 关掉联网：显示替代文字。
    app.settings_toggle(Setting::RemoteImages);
    assert_eq!(app.config.markdown_remote_images, Some(false));
    settle(&mut app);
    assert!(app.markdown.images.is_empty());
    let laid = app.markdown.layout.as_ref().unwrap();
    assert_eq!(laid.text(0, laid.chars()), "徽章\n");

    let _ = std::fs::remove_file(&cached);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_image_cache_drops_old_and_partial_files() {
    use crate::app::preview::markdown::{CACHE_AGE, clean_cache};
    use std::time::{Duration, SystemTime};

    let dir = sample_folder("md-cache");
    std::fs::write(dir.join("used.png"), b"x").unwrap();
    std::fs::write(dir.join("old.png"), b"x").unwrap();
    std::fs::write(dir.join("half.part123"), b"x").unwrap();
    // 把时间推后 2 小时：未下载完的临时文件视为过期，图片仍保留。
    let later = SystemTime::now() + Duration::from_secs(2 * 3600);
    clean_cache(&dir, later);
    assert!(!dir.join("half.part123").exists());
    assert!(dir.join("old.png").exists());
    // 刚用过的文件更新过修改时间，保留；超过 30 天未用的删除。
    let file = std::fs::File::options()
        .write(true)
        .open(dir.join("used.png"))
        .unwrap();
    file.set_modified(later + CACHE_AGE).unwrap();
    clean_cache(&dir, later + CACHE_AGE + Duration::from_secs(60));
    assert!(dir.join("used.png").exists());
    assert!(!dir.join("old.png").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headings_get_github_anchors() {
    use crate::view::preview::markdown::tests::slug;
    assert_eq!(slug("Getting Started!"), "getting-started");
    assert_eq!(slug("安装 winc"), "安装-winc");
    assert_eq!(slug("a_b-c"), "a_b-c");
}

/// 截图：分栏（左边源代码、右边预览）和只看预览，浅色、深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_preview() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        for (mode, name) in [
            (PreviewMode::Split, "split"),
            (PreviewMode::Preview, "preview"),
        ] {
            let name = format!("md-{name}-{theme}");
            let (mut app, dir) = readme(&name, dark, mode);
            shoot(&mut app, &name);
            if mode == PreviewMode::Preview {
                let area = app.markdown.area;
                app.mouse = (area.x + 50.0, area.y + 50.0);
                app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    0.0, -700.0,
                )));
                settle(&mut app);
                shoot(&mut app, &format!("{name}-scrolled"));
                // 选中一段、查找「预览」。
                app.markdown_select_all();
                app.markdown.selection = Some((10, 40));
                app.open_search(false);
                app.find_input.set_text("预览");
                app.search.query = "预览".into();
                app.search_changed();
                settle(&mut app);
                shoot(&mut app, &format!("{name}-find"));
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// GFM 补充写法的样本：front matter、短代码表情、上下标、`<details>` 折叠块、脚注，用于截图。
const EXTRAS: &str = r#"---
title: 发布说明
version: "1.2"
---

# 新东西 :tada: :rocket:

水的分子式是 H<sub>2</sub>O，面积 10 m<sup>2</sup>，再看脚注[^n]。

<details>
<summary>点开看 <b>细节</b></summary>

- 收起时只显示标题
- 点一下展开

</details>

<details open>
<summary>默认展开的</summary>

藏在里面的一段话。

</details>

```sh
cargo xtask test
```

[^n]: 脚注末尾可以跳回去。
"#;

/// 公式、内联 HTML 与 mermaid 图的样本，用于截图。
const RICH: &str = r#"# 公式

行内的 $E = mc^2$，还有 $\alpha_i^2 + \beta' \le \sqrt{x_1 + x_2}$ 和 $\frac{1}{2}$。

$$
\sum_{i=1}^{n} i = \frac{n(n+1)}{2} \qquad \int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}
$$

$$
\left( \frac{a}{b} \right)^2 = \lim_{x \to 0} \frac{\sin x}{x} \cdot \sqrt[3]{\frac{x^2+1}{y}} \quad \hat{x}\ \vec{v}\ \overline{AB}\ \mathbb{R}^n
$$

```math
A = \begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}, \quad f(x) = \begin{cases} x^2 & x \ge 0 \\ -x & \text{否则} \end{cases}
```

$$
\begin{aligned} a &= b + c \\ &= \underbrace{d + e}_{\text{两项}} \end{aligned}
$$

<table>
<tr><th>名字</th><th align="right">数量</th></tr>
<tr><td>苹果</td><td>3</td></tr>
<tr><td>梨</td><td><b>5</b></td></tr>
</table>

<ul><li>HTML 列表一</li><li>二</li></ul>
"#;

#[test]
#[ignore = "截图，手动运行"]
fn shoot_rich_markdown() {
    for dark in [false, true] {
        let dir = sample_folder("md-rich");
        let path = dir.join("RICH.md");
        std::fs::write(&path, RICH).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        shoot(
            &mut app,
            &format!("md-rich-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 能画的 mermaid 图悬停时显示全屏按钮；不支持的图按代码块显示，只有复制按钮。
#[test]
fn mermaid_blocks_draw_diagrams_and_fall_back_to_code() {
    let dir = sample_folder("md-mermaid-fallback");
    let path = dir.join("M.md");
    let drawn = "```mermaid\ngraph TD\n  A --> B\n```\n";
    let unknown = "```mermaid\ngantt\n  title 计划\n```\n";
    let expand = crate::view::preview::markdown::EXPAND;
    for (source, diagram) in [(drawn, true), (unknown, false)] {
        std::fs::write(&path, source).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let [block] = app.markdown.codes[..] else {
            panic!("只有一个代码块或图：{source}");
        };
        app.on_mouse_move(block.x + block.w / 2.0, block.y + block.h / 2.0);
        paint(&mut app);
        let has_expand = app.markdown.links.iter().any(|(_, url)| url == expand);
        assert_eq!(has_expand, diagram, "{source}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// README 顶上常见的 HTML：居中的标题和图标、徽章、按百分比并排的截图、表格里的图。
const HTML_README: &str = r#"<h1 align="center">
  <img src="logo.svg" width="64"><br>
  DEEIX Chat
</h1>

<p align="center">
  <a href="https://example.com"><img src="badge.svg" alt="build"></a>
  <a href="https://example.com"><img src="badge.svg" alt="license"></a>
</p>

<p align="center">
  <img src="wide.svg" alt="一" width="49.45%" />
  <img src="wide.svg" alt="二" width="49.45%" />
</p>

<p align="center">
  <img src="wide.svg" width="32.3%" />
  <img src="wide.svg" width="32.3%" />
  <img src="wide.svg" width="32.3%" />
</p>

按 <kbd>⌘</kbd> + <kbd>K</kbd>，<u>下划线</u>，<ins>插入</ins>，<mark>高亮</mark>，<small>小字</small>，H<sub>2</sub>O。

<div align="right">右对齐的一段</div>

<table>
  <thead><tr><th>功能</th><th>截图</th></tr></thead>
  <tbody>
    <tr><td>聊天</td><td><img src="wide.svg" width="160"></td></tr>
  </tbody>
</table>
"#;

const BADGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="20"><rect width="90" height="20" rx="3" fill="#4c1"/></svg>"##;

const WIDE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1000"><rect width="1600" height="1000" fill="#6a8fd8"/><circle cx="800" cy="500" r="300" fill="#f2c94c"/></svg>"##;

fn html_readme(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    std::fs::write(dir.join("badge.svg"), BADGE).unwrap();
    std::fs::write(dir.join("wide.svg"), WIDE).unwrap();
    let path = dir.join("README.md");
    std::fs::write(&path, HTML_README).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    (app, dir)
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_html_readme() {
    for dark in [false, true] {
        let (mut app, dir) = html_readme("md-html", dark);
        shoot(
            &mut app,
            &format!("md-html-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// mermaid 图的样本，每项为一个文档：
///
/// - 流程图与状态图；
/// - 时序图与饼图；
/// - 含多个子图的架构图，高度超过一屏；
/// - 节点文字较多、从左向右的架构图，以及含多条回边的状态图。
const DIAGRAMS: [&str; 4] = [
    r#"```mermaid
flowchart TD
    A[开始] --> B{登录了吗?}
    B -->|是| C[打开首页]
    B -->|否| D([登录页])
    D --> E[(用户库)]
    E -.-> B
    C ==> F((完成))
    subgraph 后台
        E
    end
```

```mermaid
graph LR
    a>不对称] --> b[[子程序]] --> c{{六边形}} --> d[/平行/]
    c --o e[\梯形/]
    c --x f(圆角)
```

```mermaid
stateDiagram-v2
    [*] --> 空闲
    空闲 --> 运行: 开始
    运行 --> 空闲: 停下
    运行 --> [*]
```
"#,
    r#"```mermaid
sequenceDiagram
    autonumber
    actor 用户
    participant W as 网页
    participant S as 服务器
    用户->>W: 点登录
    W->>+S: POST /login
    Note right of S: 查用户库
    alt 密码对
        S-->>W: 200 OK
    else 密码错
        S--xW: 401
    end
    S-->>-W: 结束
    W->>W: 刷新
    loop 每分钟
        W-)S: 心跳
    end
```

```mermaid
pie showData
    title 宠物
    "狗" : 386
    "猫" : 85
    "兔子" : 15
```
"#,
    r#"```mermaid
flowchart TB
    U[User / Admin Browser]
    subgraph FE[Frontend Build]
        N[Next.js 16 / React 19<br/>Chat UI / Admin Console]
    end
    subgraph GO[Go Single Runtime]
        S[Static Asset Serving]
        A[Gin HTTP API]
        APP[Application<br/>Auth / Routing / Files / Billing / Audit]
        I[Infra Adapters<br/>Protocols / Data / Cache / Storage]
    end
    subgraph EXT[External Capabilities]
        M[Model Providers<br/>OpenAI / Anthropic / Google / xAI / OpenRouter]
        T[Tool Services<br/>MCP / Provider Native Tools]
        F[Optional File Processing<br/>Tika / Docling / OCR]
    end
    subgraph DS[Data and Storage]
        P[PostgreSQL + pgvector<br/>or SQLite + sqlite-vec]
        R[Redis<br/>or In-Memory Cache]
        L[Local Filesystem<br/>or S3-Compatible Storage]
    end
    U --> S
    U --> A
    N --> S
    A --> APP --> I
    I --> M
    I --> T
    I --> F
    I --> P
    I --> R
    I --> L
```

```mermaid
flowchart LR
    subgraph 外层
        subgraph 内层
            a --> b
        end
        b --> c
    end
    x --> a
    c --> y
```
"#,
    r#"# qcAgent 架构图

## 1. 总体架构

```mermaid
flowchart LR
    caller[调用方平台]
    config[配置目录<br/>规则表 · 规则包 · 文档类型表 · 模型档案]
    subgraph svc[qcAgent 服务（一个代码库，两个入口）]
        api[qc-api<br/>鉴权 · 校验 · 数据等级闸门<br/>幂等 · 结果复用 · 规则预览]
        worker[qc-worker<br/>领取任务（租约）· 心跳 / 取消 · 清扫<br/>执行流水线 · 上传结果 · 回调]
        subgraph rt[运行环境（每个任务一套）]
            shard[分片 s01 … sNN<br/>容器模式：工具容器 + 每轮一个 pi 容器<br/>本地模式：pi 和执行服务是本机进程]
            net[容器模式：internal 网络 + 出口代理，只放行模型端点<br/>本地模式（QC_RUNTIME=local）：没有网络隔离]
        end
    end
    model[私有化模型]
    subgraph store[存储]
        db[(Postgres / SQLite<br/>任务 · 事件 · 幂等键 · 回调发件箱)]
        fs[(文件存储（本地目录）<br/>输入（按调用方、内容寻址）<br/>结果 · 审计包)]
    end
    caller -- 上传文件 / 提交任务 / 查询 / 取结果 --> api
    config --> worker
    api --> db
    api --> fs
    worker -- 领取 · 续租 · 写终态 --> db
    worker -- 读输入 · 写结果和审计包 --> fs
    worker --> shard
    shard --> net
    net --> model
    worker -. 签名回调（只含任务 ID 和状态） .-> caller
```

## 2. 任务生命周期

```mermaid
stateDiagram-v2
    [*] --> queued: 提交通过校验
    [*] --> succeeded: 输入与通过验收的旧任务完全相同（复用结果）
    queued --> running: worker 领取（设置租约）
    running --> queued: 租约过期（worker 失联），重试次数未用完
    queued --> failed: 排队超过截止时间（expired）
    queued --> canceled: 取消
    running --> canceled: 心跳发现取消，终止 pi 容器
    running --> succeeded: 有结果（passed 或 degraded）
    running --> failed: 拿不出结果（input_error / refused / system_error）
    succeeded --> [*]
    failed --> [*]
    canceled --> [*]
```
"#,
];

/// 宽度超过一行的流程图：多个子图并排，节点文字较长，结构来自实际的架构图。
const WIDE_DIAGRAM: &str = "```mermaid
flowchart TD
  subgraph FE[前端构建]
    Next[Next.js 16 / React 19<br/>Chat UI / Admin Console]
  end
  User[用户 / 管理员浏览器]
  subgraph Go[Go 单运行时]
    Static[静态资源托管]
    Gin[Gin HTTP API]
    App[Application<br/>权限 / 路由 / 文件 / 计费 / 审计]
    Infra[Infra Adapters<br/>协议 / 数据 / 缓存 / 存储]
  end
  subgraph Ext[外部能力]
    Models[模型服务商<br/>OpenAI / Anthropic / Google / xAI / OpenRouter]
    Tools[工具服务<br/>MCP / 官方原生工具]
    Extractors[可选文件处理<br/>Tika / Docling / OCR]
  end
  subgraph Data[数据与存储]
    DB[PostgreSQL + pgvector<br/>或 SQLite + sqlite-vec]
    Cache[Redis 或进程内缓存]
    Storage[本地文件 / S3 兼容存储]
  end
  Next --> Static
  User --> Static
  User --> Gin
  Gin --> App
  App --> Infra
  Infra --> Models
  Infra --> Tools
  Infra --> Extractors
  Infra --> DB
  Infra --> Cache
  Infra --> Storage
```
";

#[test]
fn wide_diagrams_shrink_to_fit_the_page() {
    let (mut app, dir) = live_doc(
        "md-mermaid-wide",
        &format!("前文\n\n{WIDE_DIAGRAM}\n后文\n"),
    );
    // 只显示预览和分栏（较窄）时都完整显示整张图：缩小字号重排，左侧的空白不计入宽度。
    for mode in [PreviewMode::Preview, PreviewMode::Split] {
        app.set_preview_mode(mode);
        settle(&mut app);
        let over = crate::view::preview::markdown::tests::overflow(&app);
        assert!(over <= 0.5, "{mode:?} 超出 {over}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_mermaid_diagrams() {
    for dark in [false, true] {
        for (k, source) in DIAGRAMS.iter().enumerate() {
            let dir = sample_folder("md-mermaid");
            let path = dir.join("MERMAID.md");
            std::fs::write(&path, source).unwrap();
            let mut app = app_with(&path, dark);
            paint(&mut app);
            app.set_preview_mode(PreviewMode::Preview);
            settle(&mut app);
            let theme = if dark { "dark" } else { "light" };
            shoot(&mut app, &format!("md-mermaid-{k}-{theme}"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// 打开内容为 `DIAGRAMS[k]` 的文档，切换到只看预览，鼠标移到第一张图上后点击全屏按钮。
fn diagram_view_app(name: &str, k: usize, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("MERMAID.md");
    std::fs::write(&path, DIAGRAMS[k]).unwrap();
    let mut app = app_with(&path, dark);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let diagram = *app.markdown.codes.first().expect("画出了图");
    app.on_mouse_move(diagram.x + diagram.w / 2.0, diagram.y + 20.0);
    paint(&mut app);
    let expand = crate::view::preview::markdown::EXPAND;
    let (button, _) = app
        .markdown
        .links
        .iter()
        .find(|(_, url)| url == expand)
        .cloned()
        .expect("鼠标在图上时有全屏按钮");
    click(
        &mut app,
        (button.x + button.w / 2.0, button.y + button.h / 2.0),
    );
    paint(&mut app);
    (app, dir)
}

/// 全屏查看：图覆盖编辑区并按适应查看区域的倍数显示；工具栏按钮、按键与按住 `⌘` 滚动可以
/// 缩放，拖动可以平移；`Esc`、关闭按钮或切回源代码时关闭，查看期间输入不写入文档。
#[test]
fn diagrams_open_full_screen_and_zoom() {
    use winit::keyboard::{Key, NamedKey};

    use crate::app::preview::markdown::DiagramTool;
    let (mut app, dir) = diagram_view_app("md-diagram-view", 0, false);
    let view = app.markdown.diagram.as_ref().expect("打开了全屏查看");
    assert_eq!(view.area, app.layout.editor);
    assert!(view.zoom.is_none());
    let fit = view.scale();
    assert!(fit > 1.0, "小图放大到适应窗口：{fit}");
    let button = |app: &App, tool| {
        let view = app.markdown.diagram.as_ref().unwrap();
        let (r, _) = view.buttons.iter().find(|(_, t)| *t == tool).unwrap();
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    let scale = |app: &App| app.markdown.diagram.as_ref().unwrap().scale();

    let at = button(&app, DiagramTool::ZoomIn);
    click(&mut app, at);
    paint(&mut app);
    assert!((scale(&app) - fit * 1.25).abs() < 1e-3);
    assert!(app.diagram_view_key(&Key::Character("-".into()), "-", false));
    assert!((scale(&app) - fit).abs() < 1e-3);

    // 按住 `⌘` 滚动：以鼠标处为中心放大，鼠标处的内容位置不变。
    let area = app.markdown.diagram.as_ref().unwrap().area;
    let at = (area.x + area.w / 4.0, area.y + area.h / 3.0);
    app.on_mouse_move(at.0, at.1);
    app.modifiers = command_modifiers();
    // 滚动量以物理像素给出；离屏窗口的缩放倍数为 2，换算为 100 逻辑像素。
    app.on_wheel(MouseScrollDelta::PixelDelta((0.0, 200.0).into()));
    app.modifiers = Default::default();
    paint(&mut app);
    let view = app.markdown.diagram.as_ref().unwrap();
    let k = view.scale() / fit;
    assert!(k > 1.5, "{k}");
    let (cx, cy) = (at.0 - area.x - area.w / 2.0, at.1 - area.y - area.h / 2.0);
    assert!((view.pan.0 - (cx - cx * k)).abs() < 0.5);
    assert!((view.pan.1 - (cy - cy * k)).abs() < 0.5);

    // 拖动平移；点击倍数后恢复适应查看区域的倍数，图居中。
    let before = view.pan;
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(at.0 + 30.0, at.1 - 10.0);
    app.on_mouse_up(MouseButton::Left);
    let view = app.markdown.diagram.as_ref().unwrap();
    assert!((view.pan.0 - (before.0 + 30.0)).abs() < 1e-3);
    assert!((view.pan.1 - (before.1 - 10.0)).abs() < 1e-3);
    assert!(view.grab.is_none());
    let at = button(&app, DiagramTool::Fit);
    click(&mut app, at);
    let view = app.markdown.diagram.as_ref().unwrap();
    assert_eq!((view.zoom, view.pan), (None, (0.0, 0.0)));

    // 全屏查看期间的输入不写入被遮住的文档。
    let revision = app.editor.active().unwrap().revision;
    assert!(app.diagram_view_key(&Key::Character("x".into()), "x", false));
    assert_eq!(app.editor.active().unwrap().revision, revision);

    assert!(app.diagram_view_key(&Key::Named(NamedKey::Escape), "", false));
    assert!(app.markdown.diagram.is_none());

    // 关闭按钮关闭全屏查看；切回源代码同样关闭。
    let (mut app, dir2) = diagram_view_app("md-diagram-view-close", 0, false);
    let at = button(&app, DiagramTool::Close);
    click(&mut app, at);
    assert!(app.markdown.diagram.is_none());
    let (mut app, dir3) = diagram_view_app("md-diagram-view-source", 0, false);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert!(app.markdown.diagram.is_none());
    for dir in [dir, dir2, dir3] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_diagram_view() {
    for dark in [false, true] {
        let (mut app, dir) = diagram_view_app("md-diagram-shot", 3, dark);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("md-diagram-view-{theme}"));
        // 关闭全屏查看后鼠标仍在图上，截取图右上角的全屏、复制按钮。
        app.close_diagram_view();
        let diagram = *app.markdown.codes.first().unwrap();
        app.on_mouse_move(diagram.right() - 40.0, diagram.y + 20.0);
        shoot(&mut app, &format!("md-diagram-buttons-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_github_extras() {
    for dark in [false, true] {
        let dir = sample_folder("md-extras");
        let path = dir.join("EXTRAS.md");
        std::fs::write(&path, EXTRAS).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let code = *app.markdown.codes.first().unwrap();
        app.on_mouse_move(code.x + 20.0, code.y + 10.0);
        shoot(
            &mut app,
            &format!("md-extras-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 计时：`DIAGRAMS` 重复 5 遍组成的文档首次排版的耗时，以及只看预览时滚动 50 帧的耗时；
/// 同一文档的图按普通代码块显示时的耗时作为对照。
#[test]
#[ignore = "计时，手动运行"]
fn time_mermaid() {
    for lang in ["mermaid", "text"] {
        let text = DIAGRAMS
            .join("\n\n")
            .repeat(5)
            .replace("```mermaid", &format!("```{lang}"));
        let dir = sample_folder("md-time-mermaid");
        let path = dir.join("MERMAID.md");
        std::fs::write(&path, &text).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!("mermaid 第一次排版 {:?}", started.elapsed());
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.markdown.scroll_to((i * 173) as f32);
            paint(&mut app);
        }
        eprintln!("{lang} 滚 50 次 {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 计时：只有一个很大的顶层块（长列表、长表格、长代码块）时，分栏滚动每帧要多久。
#[test]
#[ignore = "计时，手动运行"]
fn time_huge_blocks() {
    let list: String = (0..8000)
        .map(|i| format!("- 第 {i} 项，**加粗** 和 `代码`\n"))
        .collect();
    let table: String = "| a | b | c |\n|---|---|---|\n".to_owned()
        + &(0..8000)
            .map(|i| format!("| {i} | 第 {i} 行 | 说明 |\n"))
            .collect::<String>();
    let code = format!(
        "```rust\n{}```\n",
        (0..20000)
            .map(|i| format!("let x{i} = {i}; // 注释\n"))
            .collect::<String>()
    );
    for (name, text) in [("list", list), ("table", table), ("code", code)] {
        let dir = sample_folder(&format!("md-huge-{name}"));
        let path = dir.join("BIG.md");
        std::fs::write(&path, &text).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        // 对照：不分栏时滚同样的位置。
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.editor.active_mut().unwrap().scroll_line = (i * 97) as f32;
            paint(&mut app);
        }
        eprintln!("{name} 不分栏滚 50 次 {:?}", started.elapsed());
        app.set_preview_mode(PreviewMode::Split);
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!(
            "{name} {} KB 第一次排版 {:?}",
            text.len() / 1024,
            started.elapsed()
        );
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.editor.active_mut().unwrap().scroll_line = (i * 97) as f32;
            paint(&mut app);
        }
        eprintln!("{name} 滚左边 50 次 {:?}", started.elapsed());
        // 在中间某一项（行）里改一个字。
        app.go_to(4000, 3, 0);
        paint(&mut app);
        app.run(winc_editor::Command::Insert("字".into()));
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!("{name} 改一个字 {:?}", started.elapsed());
        // 只看预览时在同一处打字：光标所在的块显示原文，每打一个字重排这一块（不等解析）。
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        app.run(winc_editor::Command::Insert("字".into()));
        paint(&mut app);
        let started = std::time::Instant::now();
        for _ in 0..10 {
            app.run(winc_editor::Command::Insert("字".into()));
            paint(&mut app);
        }
        eprintln!("{name} 预览中打 10 个字 {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 测量约 1 MB 的文档在预览中执行各步骤后增加的堆内存（MB）和排版缓存的条目数。
#[test]
#[ignore = "测量，手动运行"]
// 按全局分配器（`large_alloc`，仅 macOS）记下的大块内存计算。
#[cfg(target_os = "macos")]
fn measure_markdown_memory() {
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    let section = SAMPLE.replace("logo.svg", "missing.svg");
    let text: String = (0..900)
        .map(|i| format!("## 第 {i} 节\n\n{section}\n"))
        .collect();
    let dir = sample_folder("md-memory");
    let path = dir.join("BIG.md");
    std::fs::write(&path, &text).unwrap();
    std::fs::write(dir.join("other.rs"), "fn main() {}\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let report = |app: &App, step: &str, base: isize| {
        let memo = &app.markdown.memo;
        eprintln!(
            "{step:<16} {:>7.1} MB  缓存 {} 条",
            mb(
                (crate::large_alloc::LIVE.load(std::sync::atomic::Ordering::Relaxed) - base).max(0)
                    as u64
            ),
            memo.len()
        );
    };
    let base = crate::large_alloc::LIVE.load(std::sync::atomic::Ordering::Relaxed);
    report(&app, "打开源代码", base);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    report(&app, "只看预览", base);
    if std::env::var_os("WINC_MD_BREAKDOWN").is_some() {
        let laid = app.markdown.layout.take();
        drop(laid);
        report(&app, "去掉排版结果", base);
        app.markdown.memo = Default::default();
        report(&app, "去掉排版缓存", base);
        heap_breakdown(app.markdown.parsed.as_ref().unwrap());
        app.markdown.parsed = None;
        report(&app, "去掉解析结果", base);
        return;
    }
    // 从开头滚动到末尾。
    let height = app.markdown.height;
    let mut y = 0.0;
    while y < height {
        app.markdown.scroll_to(y);
        paint(&mut app);
        y += app.markdown.area.h;
    }
    report(&app, "滚到底", base);
    // 在中间的段落中输入 300 个字符，每个字符绘制一帧并等待一次解析。
    let at = text.len() / 2;
    let at = text[at..].find("轻量").map_or(at, |i| at + i);
    let doc = app.editor.active_mut().unwrap();
    let pos = doc.text().rope().unwrap().byte_to_char(at);
    doc.selection = winc_core::Selection::point(pos);
    for _ in 0..300 {
        app.run(winc_editor::Command::Insert("字".into()));
        app.wait_for_markdown();
        paint(&mut app);
    }
    report(&app, "打 300 个字", base);
    app.set_preview_mode(PreviewMode::Source);
    settle(&mut app);
    report(&app, "换回源代码", base);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    app.open_path(&dir.join("other.rs"));
    settle(&mut app);
    report(&app, "换到别的标签", base);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 打印解析结果中各部分占用的堆内存（按容量计算）。
#[cfg(target_os = "macos")]
fn heap_breakdown(md: &winc_preview::markdown::Markdown) {
    use winc_preview::markdown::{Block, Inline};
    #[derive(Default, Debug)]
    struct Sum {
        blocks: usize,
        block_count: usize,
        inlines: usize,
        text: usize,
        text_len: usize,
        runs: usize,
        run_count: usize,
        map: usize,
        anchors: usize,
        links: usize,
        code: usize,
        other: usize,
    }
    fn inline(i: &Inline, s: &mut Sum) {
        s.inlines += 1;
        s.text += i.text.capacity();
        s.text_len += i.text.len();
        s.runs += i.runs.capacity() * std::mem::size_of::<winc_preview::markdown::Run>();
        s.run_count += i.runs.len();
        s.map += i.map.heap_bytes();
        s.anchors += i.map.anchors();
        s.links += i.links.iter().map(String::capacity).sum::<usize>() + i.links.capacity() * 24;
        s.other += i.images.capacity() * std::mem::size_of::<winc_preview::markdown::Image>();
    }
    fn blocks(v: &Vec<Block>, s: &mut Sum) {
        s.blocks += v.capacity() * std::mem::size_of::<Block>();
        for b in v {
            s.block_count += 1;
            match b {
                Block::Paragraph(i) | Block::Heading(_, i) => inline(i, s),
                Block::Code { lang, text, map } => {
                    s.code += lang.capacity() + text.capacity() + map.heap_bytes();
                }
                Block::Quote(_, b) | Block::Footnote(_, b) => blocks(b, s),
                Block::List(_, items) => {
                    s.other +=
                        items.capacity() * std::mem::size_of::<winc_preview::markdown::Item>();
                    for it in items {
                        blocks(&it.blocks, s);
                    }
                }
                Block::Table(a, rows, _) => {
                    s.other += a.capacity() + rows.capacity() * 24;
                    for r in rows {
                        s.other += r.capacity() * std::mem::size_of::<Inline>();
                        for c in r {
                            inline(c, s);
                        }
                    }
                }
                Block::Math(t) => s.code += t.capacity(),
                Block::Rule => {}
                Block::Details {
                    summary, blocks: b, ..
                } => {
                    inline(summary, s);
                    blocks(b, s);
                }
            }
        }
    }
    let mut s = Sum::default();
    blocks(&md.blocks, &mut s);
    eprintln!(
        "size_of Block {} Inline {}",
        std::mem::size_of::<Block>(),
        std::mem::size_of::<Inline>()
    );
    eprintln!("{s:#?}");
    eprintln!(
        "hashes {} lines {} ranges {} leaves {}",
        md.hashes.capacity() * 8,
        md.lines.capacity() * 8,
        md.ranges.capacity() * 16,
        md.leaves.capacity() * std::mem::size_of::<winc_preview::markdown::LeafRange>()
    );
}

/// 计时：用一个约 1 MB 的文档，量第一次排版和改一个字以后重排各要多久。
#[test]
#[ignore = "计时，手动运行"]
fn time_large_markdown() {
    let section = SAMPLE.replace("logo.svg", "missing.svg");
    let text: String = (0..900)
        .map(|i| format!("## 第 {i} 节\n\n{section}\n"))
        .collect();
    eprintln!("{} KB", text.len() / 1024);
    let dir = sample_folder("md-large");
    let path = dir.join("BIG.md");
    std::fs::write(&path, &text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    let started = std::time::Instant::now();
    app.wait_for_markdown();
    eprintln!("解析 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("第一次排版加两帧 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("再画两帧 {:?}", started.elapsed());
    settle(&mut app);
    app.run(winc_editor::Command::Insert("字".into()));
    app.wait_for_markdown();
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("改一个字后重排加两帧 {:?}", started.elapsed());
    // 有一张图片读好了：只重排用到它的块。
    app.markdown.loaded.push(dir.join("other.png"));
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("读好一张图片后重排加两帧 {:?}", started.elapsed());
    // 分栏时滚左边的文字（右边跟着滚）、滚右边的预览（左边跟着滚），每次一帧。
    let started = std::time::Instant::now();
    for i in 0..100 {
        app.editor.active_mut().unwrap().scroll_line = (i * 37) as f32;
        paint(&mut app);
    }
    eprintln!("滚左边 100 帧 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    for _ in 0..100 {
        app.scroll_markdown(-300.0);
        paint(&mut app);
    }
    eprintln!("滚右边 100 帧 {:?}", started.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：可编辑的预览中光标位于粗体和链接中，只显示光标接触到的标记。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_reveal() {
    let (mut app, dir) = readme("md-reveal-shot", false, PreviewMode::Preview);
    for (needle, name) in [
        ("Rust", "bold"),
        ("GitHub", "link"),
        ("winc 编辑器", "heading"),
        ("cargo xtask", "code"),
    ] {
        put_caret(&mut app, needle, false);
        app.run(winc_editor::Command::MoveRight {
            extend: false,
            word: false,
        });
        app.run(winc_editor::Command::MoveLeft {
            extend: false,
            word: false,
        });
        settle(&mut app);
        shoot(&mut app, &format!("md-reveal-{name}"));
    }
    // 标题中的输入法组字使用标题的字号和粗体。
    put_caret(&mut app, "编辑器", false);
    app.preedit = "中文".into();
    shoot(&mut app, "md-reveal-preedit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：可编辑的预览中光标分别位于代码、mermaid、公式、HTML、图片、表格中。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_blocks() {
    let text = "# 块\n\n```rust\nfn main() {}\n```\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n\
                $$\nx^2 + y^2\n$$\n\n<div align=\"center\">\n<b>粗</b> 和 <i>斜</i>\n</div>\n\n\
                ![标志](logo.svg)\n\n| 名 | 值 |\n|---|---|\n| **甲** | `1` |\n| 乙 | 2 |\n\n尾\n";
    let (mut app, dir) = live_doc("md-blocks-shot", text);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    settle(&mut app);
    for (needle, name) in [
        ("fn main", "code"),
        ("```rust", "code-fence"),
        ("A -->", "mermaid"),
        ("x^2", "math"),
        ("<b>", "html"),
        ("标志", "image"),
        ("甲", "table"),
    ] {
        put_caret(&mut app, needle, false);
        shoot(&mut app, &format!("md-block-{name}"));
    }
    // 删除表格的一行，过渡进行到一半：后文正在向上滑动。
    app.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 3);
    put_caret(&mut app, "乙", false);
    app.live_table_tool("delete-row");
    settle(&mut app);
    shoot(&mut app, "md-block-table-motion");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn markdown_images_render_one_size_at_a_time() {
    use crate::app::preview::markdown::ImageSlot;

    let mut app = blank_app(false);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
    let picture = winc_preview::svg::load(svg, None, None).unwrap();
    let path = PathBuf::from("/tmp/winc-shot.svg");
    let slot = ImageSlot::Ready {
        picture: std::sync::Arc::new(picture),
        pixels: Vec::new(),
        drawing: Vec::new(),
    };
    app.markdown.images.insert(path.clone(), slot);
    // 拖动改变窗口大小：每帧要一个新尺寸，正在绘制时不再提交，只有第一个在后台绘制。
    for width in 100..110 {
        app.request_markdown_pixels(&path, (width, 50));
    }
    let Some(ImageSlot::Ready { drawing, .. }) = app.markdown.images.get(&path) else {
        panic!("图片不在");
    };
    assert_eq!(drawing, &[(100, 50)]);
}

#[test]
fn the_chosen_preview_mode_applies_to_files_opened_later() {
    use winc_editor::PreviewMode;

    let dir = sample_folder("preview-mode-remembered");
    let (a, b, c) = (dir.join("a.md"), dir.join("b.md"), dir.join("c.md"));
    for path in [&a, &b, &c] {
        std::fs::write(path, "# 标题\n").unwrap();
    }
    let mut app = app_with(&a, false);
    paint(&mut app);
    let mode = |app: &App| app.editor.active().unwrap().preview_mode();
    assert_eq!(mode(&app), PreviewMode::Source);

    // 选择只看预览：记进状态，之后打开的 Markdown 文件同样只看预览。
    app.choose_preview_mode(PreviewMode::Preview);
    assert_eq!(app.state.preview_mode.as_deref(), Some("preview"));
    app.open_path(&b);
    paint(&mut app);
    assert_eq!(mode(&app), PreviewMode::Preview);

    // 改回源代码：状态清空，之后打开的文件显示源代码。
    app.choose_preview_mode(PreviewMode::Source);
    assert_eq!(app.state.preview_mode, None);
    app.open_path(&c);
    paint(&mut app);
    assert_eq!(mode(&app), PreviewMode::Source);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_the_split_divider_resizes_both_halves() {
    use winc_editor::PreviewMode;
    use winit::event::MouseButton;

    let dir = sample_folder("split-divider-drag");
    let path = dir.join("a.md");
    std::fs::write(&path, "# 标题\n\n正文\n").unwrap();
    let mut app = app_with(&path, false);
    app.choose_preview_mode(PreviewMode::Split);
    paint(&mut app);
    let (left, divider) = (app.layout.editor.w, app.layout.split_divider);
    assert!(divider.w > 0.0);
    let total = left + app.layout.split.w + 1.0;

    // 按住分隔线向右拖 100：源代码一侧变宽，预览一侧变窄，总宽度不变。
    let (x, y) = (divider.x + divider.w / 2.0, divider.y + divider.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::SplitDivider);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!((app.layout.editor.w - (left + 100.0)).abs() <= 1.0);
    assert_eq!(app.layout.editor.w + app.layout.split.w + 1.0, total);

    // 拖到最左边：源代码一侧停在最小宽度，不再变窄。
    let divider = app.layout.split_divider;
    let x = divider.x + divider.w / 2.0;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(0.0, y);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert_eq!(app.layout.editor.w, 240.0);

    // 双击分隔线：恢复为左右各占一半。
    let divider = app.layout.split_divider;
    let x = divider.x + divider.w / 2.0;
    app.on_mouse_move(x, y);
    for _ in 0..2 {
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
    }
    paint(&mut app);
    assert_eq!(app.split_share, None);
    assert_eq!(app.layout.editor.w, left);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_live_preview_hides_its_caret_until_clicked_and_ignores_the_margins() {
    let (mut app, dir) = readme("md-live-caret", false, PreviewMode::Preview);
    paint(&mut app);
    // 刚打开、只是阅读：不显示光标。
    assert!(!app.live_caret_shown());
    let before = app.editor.active().unwrap().selection;

    // 点在内容一栏左侧的空白：不放光标、不开始选字。
    let area = app.markdown.area;
    super::click(&mut app, (area.x + 2.0, area.y + area.h / 2.0));
    paint(&mut app);
    assert!(!app.live_caret_shown());
    assert_eq!(app.editor.active().unwrap().selection, before);

    // 点在文字上：显示光标。
    let at = point_of(&app, 2);
    super::click(&mut app, at);
    paint(&mut app);
    assert!(app.live_caret_shown());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 只看预览时左侧有标题指示条，每个标题一条；鼠标停在指示条上时标题列表在原处展开，点击一项
/// 滚到该标题。分栏时预览变窄，左侧留白放不下指示条，不画。
#[test]
fn the_preview_shows_a_heading_indicator() {
    use crate::app::preview::markdown::TocTarget;

    let (mut app, dir) = readme("md-toc", false, PreviewMode::Preview);
    assert_eq!(app.markdown.toc_tops.len(), 2);
    let find = |app: &App, target| {
        app.markdown
            .toc
            .iter()
            .find(|(_, t)| *t == target)
            .map(|h| h.0)
    };
    let bars = find(&app, TocTarget::Bars).unwrap();
    assert!(find(&app, TocTarget::Heading(0)).is_none());

    app.on_mouse_move(bars.x + 20.0, bars.y + 10.0);
    paint(&mut app);
    assert!(matches!(app.hover, Hover::MarkdownToc(_)));
    // 展开时第一行的横线落在指示条第一条的位置上（还没有滚到任何标题），横线不跳动。指示条的
    // 范围比第一条高出 8，第一条粗 2。
    let first = find(&app, TocTarget::Heading(0)).unwrap();
    assert_eq!(first.y + first.h / 2.0, bars.y + 8.0 + 1.0);
    // 列表盖住指示条展开；鼠标停在列表里两行之间的空白处时列表不收起。
    let row = find(&app, TocTarget::Heading(1)).unwrap();
    let card = app.markdown.toc.iter().rev().nth(1).unwrap().0;
    assert!(card.contains(bars.x + 20.0, bars.y + 10.0));
    app.on_mouse_move(card.x + 1.0, row.y);
    paint(&mut app);
    assert!(find(&app, TocTarget::Heading(1)).is_some());

    click(&mut app, (row.x + 20.0, row.y + row.h / 2.0));
    let max = (app.markdown.height - app.markdown.area.h).max(0.0);
    assert_eq!(app.markdown.scroll(), app.markdown.toc_tops[1].min(max));
    paint(&mut app);
    assert_eq!(app.markdown.toc_current, Some(1));

    // 指示条贴在正文左边，随正文居中的位置移动。
    let left = app.markdown.layout.as_ref().unwrap().left;
    // 列表展开着时卡片也记作 `Bars`，指示条本身总是最后一项。
    let bars = app.markdown.toc.last().unwrap().0;
    assert!(
        bars.right() <= left && bars.right() >= left - 24.0,
        "{bars:?} {left}"
    );

    // 点开头的标题后再滚到底：文末的标题滚不到顶上，当前所在的一节取屏幕里最后一个标题。
    app.press_toc(TocTarget::Heading(0));
    paint(&mut app);
    assert_eq!(app.markdown.toc_current, Some(0));
    app.scroll_markdown_to(max);
    paint(&mut app);
    assert_eq!(app.markdown.toc_current, Some(1));

    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert!(app.markdown.toc.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：本仓库的 README 只看预览，滚到第三个标题，鼠标停在标题指示条上；点最后一个标题后
/// 正文里的底色。深浅两套配色。
#[test]
#[ignore]
fn shoot_preview_heading_indicator() {
    use crate::app::preview::markdown::TocTarget;

    for dark in [false, true] {
        let dir = sample_folder(if dark { "md-toc-dark" } else { "md-toc-light" });
        let path = dir.join("README.md");
        std::fs::write(&path, include_str!("../../../../../README.md")).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let top = app.markdown.toc_tops[2];
        app.scroll_markdown_to(top);
        paint(&mut app);
        shoot(&mut app, if dark { "md-toc-dark" } else { "md-toc-light" });
        let bars = app
            .markdown
            .toc
            .iter()
            .find(|(_, t)| *t == TocTarget::Bars)
            .unwrap()
            .0;
        app.on_mouse_move(bars.x + 20.0, bars.y + 10.0);
        paint(&mut app);
        let open = if dark {
            "md-toc-open-dark"
        } else {
            "md-toc-open-light"
        };
        shoot(&mut app, open);
        // 展开到一半：横线正从指示条滑向各行，卡片和文字淡入到一半。
        let at = crate::app::animation::TOC_FADE;
        app.markdown.toc_fade = Some(crate::app::animation::Clock::frozen(at / 2, at));
        let half = if dark {
            "md-toc-opening-dark"
        } else {
            "md-toc-opening-light"
        };
        shoot(&mut app, half);
        app.markdown.toc_fade = None;
        // 点最后一个标题：滚到底，它成为当前所在的一节，正文里的底色淡出到三成。
        let last = app.markdown.toc_tops.len() - 1;
        app.press_toc(TocTarget::Heading(last));
        let at = crate::app::animation::TOC_FLASH;
        let clock = crate::app::animation::Clock::frozen(at * 3 / 10, at);
        app.markdown.toc_flash = Some((last, clock));
        app.on_mouse_move(900.0, 400.0);
        paint(&mut app);
        let picked = if dark {
            "md-toc-picked-dark"
        } else {
            "md-toc-picked-light"
        };
        shoot(&mut app, picked);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 把 `files`（包中路径、内容）原样存放（不压缩）写成 zip，按 ZIP 文件格式（APPNOTE.TXT）
/// 逐字段写出；读取时不核对 CRC，校验字段固定写 0。
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let sizes = [
            (data.len() as u32).to_le_bytes(),
            (data.len() as u32).to_le_bytes(),
        ];
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0]);
        out.extend([0; 8]);
        out.extend(sizes.concat());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0]);
        central.extend([0; 8]);
        central.extend(sizes.concat());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend([0, 0]);
    out
}

/// 一份 docx：两个标题、一段正文和包中的一张图（[`LOGO`]）。
fn sample_docx() -> Vec<u8> {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let root = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{rel}/officeDocument\" Target=\"word/document.xml\"/>\
         </Relationships>"
    );
    let rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rI\" Type=\"{rel}/image\" Target=\"media/logo.svg\"/>\
         </Relationships>"
    );
    let heading = |level: u8, text: &str| {
        format!(
            "<w:p><w:pPr><w:outlineLvl w:val=\"{level}\"/></w:pPr>\
             <w:r><w:t>{text}</w:t></w:r></w:p>"
        )
    };
    let document = format!(
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"{rel}\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
         xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\">\
         <w:body>{}{}<w:p><w:r><w:t>正文，</w:t></w:r><w:r><w:rPr><w:b/></w:rPr>\
         <w:t>加粗</w:t></w:r></w:p><w:p><w:r><w:drawing><wp:inline>\
         <wp:extent cx=\"914400\" cy=\"914400\"/><wp:docPr id=\"1\" name=\"标志\"/>\
         <a:graphic><a:graphicData><a:blip r:embed=\"rI\"/></a:graphicData></a:graphic>\
         </wp:inline></w:drawing></w:r></w:p></w:body></w:document>",
        heading(0, "季度报告"),
        heading(1, "概况"),
    );
    stored_zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/logo.svg", LOGO.as_bytes()),
    ])
}

/// docx 打开后显示阅读视图：只读、铺满编辑区，标题出现在标题指示条中，包中的图片从内存读取；
/// 右键菜单没有「在源代码中显示」。文件损坏时显示原因和重试、用默认应用打开的按钮。
#[test]
fn docx_files_open_as_a_reading_view() {
    let dir = sample_folder("md-docx");
    let path = dir.join("季度报告.docx");
    std::fs::write(&path, sample_docx()).unwrap();
    let mut app = app_with(&path, false);
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.unsupported, Some(winc_editor::Unsupported::Docx));
    assert!(doc.readonly);
    assert_eq!(app.markdown.area, app.layout.editor);
    assert!(app.markdown.failed.is_none());
    assert_eq!(app.markdown.toc_tops.len(), 2);
    // 图片按文档所在目录解析出路径，但不读磁盘：磁盘上没有这个文件。
    let opened = doc.path.clone().unwrap();
    let logo = opened.parent().unwrap().join("word/media/logo.svg");
    assert!(!logo.exists());
    assert!(matches!(
        app.markdown.images.get(&logo),
        Some(crate::app::preview::markdown::ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));
    let laid = app.markdown.layout.as_ref().unwrap();
    assert!(laid.chars() > 0);

    let area = app.markdown.area;
    right_click(&mut app, (area.x + area.w / 2.0, area.y + 60.0));
    assert!(
        !menu_commands(&app)
            .iter()
            .any(|(c, _)| *c == MenuCommand::RevealSource)
    );

    let broken = dir.join("损坏.docx");
    std::fs::write(&broken, b"not a docx").unwrap();
    app.open_path(&broken);
    settle(&mut app);
    assert_eq!(
        app.markdown.failed.as_deref(),
        Some(winc_i18n::t!("preview-docx-invalid"))
    );
    assert_eq!(app.placeholder_hits.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_docx() {
    for dark in [false, true] {
        let dir = sample_folder("md-docx-shot");
        let path = dir.join("季度报告.docx");
        std::fs::write(&path, sample_docx()).unwrap();
        let mut app = app_with(&path, dark);
        settle(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("md-docx-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 写一份 `count` 页的 PDF（每页 US Letter 大小，612 × 792 点）：每页顶部用 Helvetica 写页号，
/// 下面画一条长度随页号变化的红色横条。交叉引用表按各对象的实际偏移写出。
#[cfg(any(target_os = "macos", windows))]
fn sample_pdf(count: usize) -> Vec<u8> {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {count} >>",
            (0..count)
                .map(|k| format!("{} 0 R", 3 + k * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    for k in 0..count {
        let content = format!(
            "BT /F1 36 Tf 72 700 Td (Page {}) Tj ET 1 0 0 rg 72 620 {} 40 re f",
            k + 1,
            100 + k * 40
        );
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 \
             /BaseFont /Helvetica >> >> >> >>",
            4 + k * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", k + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// 打开 `path`，等待渲染线程完成打开和视口附近各页的渲染。
#[cfg(any(target_os = "macos", windows))]
fn open_pdf(path: &Path, dark: bool) -> App {
    let mut app = app_with(path, dark);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    paint(&mut app);
    app
}

/// PDF 打开后逐页显示：只渲染视口附近的页，缓存中只保留这几页；滚动到结尾后，前面的页从缓存中
/// 移除。文件损坏时显示原因，以及「重试」「用默认应用打开」两个按钮。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn pdf_files_render_only_the_pages_in_view() {
    use winit::keyboard::{Key, NamedKey};

    let dir = sample_folder("pdf-pages");
    let path = dir.join("手册.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.unsupported, Some(winc_editor::Unsupported::Pdf));
    assert!(doc.readonly);
    assert_eq!(app.pdf.area, app.layout.editor);
    assert_eq!(app.pdf.pages.as_ref().unwrap().as_ref().unwrap().len(), 30);
    let first: Vec<usize> = app.pdf.rendered.keys().copied().collect();
    assert!(first.contains(&0), "{first:?}");
    assert!(first.len() <= 4, "{first:?}");

    assert!(app.pdf_key(&Key::Named(NamedKey::End)));
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(app.pdf.scroll, app.pdf.max_scroll());
    let last: Vec<usize> = app.pdf.rendered.keys().copied().collect();
    assert!(last.contains(&29) && !last.contains(&0), "{last:?}");
    assert!(last.len() <= 4, "{last:?}");

    // 回归：页面滚出预览顶部时，超出预览区域的部分（包括边框）不绘制到标签栏上。
    let above = |app: &mut App| {
        paint(app);
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let scale = app.renderer.as_ref().unwrap().scale_factor();
        let rows = (app.pdf.area.y * scale).floor() as usize;
        image.pixels[..rows * image.width as usize].to_vec()
    };
    app.pdf.scroll = 0.0;
    let before = above(&mut app);
    app.pdf.scroll = 300.0;
    assert_eq!(above(&mut app), before);

    let broken = dir.join("损坏.pdf");
    std::fs::write(&broken, b"not a pdf").unwrap();
    app.open_path(&broken);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(
        app.pdf
            .pages
            .as_ref()
            .unwrap()
            .as_ref()
            .err()
            .map(String::as_str),
        Some(winc_i18n::t!("preview-pdf-invalid"))
    );
    assert_eq!(app.placeholder_hits.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(any(target_os = "macos", windows))]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_pdf() {
    for dark in [false, true] {
        let dir = sample_folder("pdf-shot");
        let path = dir.join("手册.pdf");
        std::fs::write(&path, sample_pdf(5)).unwrap();
        let mut app = open_pdf(&path, dark);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("pdf-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// PDF 第 `page` 页第 `i` 个字的中心（窗口坐标），按上一帧的排版换算。
#[cfg(windows)]
fn pdf_char_center(app: &App, page: usize, i: usize) -> (f32, f32) {
    let r = app.pdf.page_rects[page];
    let (x, y) = (app.pdf.area.x + r.x, app.pdf.area.y + r.y - app.pdf.scroll);
    let c = app.pdf.texts[&page].chars[i].rect.unwrap();
    (
        x + (c.left + c.right) / 2.0 * r.w,
        y + (c.top + c.bottom) / 2.0 * r.h,
    )
}

/// 反复绘制、收取渲染线程的结果，直到 `done` 成立（最多约 5 秒）。
#[cfg(windows)]
fn pdf_until(app: &mut App, done: impl Fn(&App) -> bool) {
    for _ in 0..250 {
        paint(app);
        app.wait_for_pdf();
        app.poll_pdf();
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("等待 PDF 超时");
}

/// 有 PDFium 时 PDF 能选字、复制，右键给选中的字加带注释的高亮并保存进文件；悬停显示注释，
/// 右键可以删除批注。没有 PDFium 时跳过（设置了 `WINC_PDFIUM` 却没有加载成功时失败）。
#[cfg(windows)]
#[test]
fn pdf_text_can_be_selected_annotated_and_saved() {
    use crate::app::input::context_menu::MenuCommand;
    use winc_preview::pdf::AnnotationKind;

    let dir = sample_folder("pdf-annotate");
    let path = dir.join("论文.pdf");
    std::fs::write(&path, sample_pdf(2)).unwrap();
    let mut app = open_pdf(&path, false);
    if !app.pdf.editable {
        assert!(
            std::env::var_os("WINC_PDFIUM").is_none(),
            "WINC_PDFIUM 指定的 PDFium 无法加载"
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    pdf_until(&mut app, |app| app.pdf.texts.contains_key(&0));
    let text: String = app.pdf.texts[&0].chars.iter().map(|c| c.ch).collect();
    let start = text.find("Page 1").unwrap();
    let (first, last) = (start, start + "Page 1".len() - 1);

    // 从 `P` 的左半边拖到 `1` 的右半边：选中「Page 1」。
    let (x0, y0) = pdf_char_center(&app, 0, first);
    let (x1, y1) = pdf_char_center(&app, 0, last);
    app.on_mouse_move(x0 - 4.0, y0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x1 + 4.0, y1);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.pdf_selected_text().as_deref(), Some("Page 1"));
    app.copy(false);
    assert_eq!(app.copied.as_deref(), Some("Page 1"));

    // 右键选中的字：可以加高亮、下划线、删除线和带注释的高亮。
    right_click(&mut app, ((x0 + x1) / 2.0, y0));
    let commands = menu_commands(&app);
    for command in [
        MenuCommand::Copy,
        MenuCommand::PdfHighlight,
        MenuCommand::PdfUnderline,
        MenuCommand::PdfStrikeOut,
        MenuCommand::PdfComment,
    ] {
        assert!(commands.contains(&(command, true)), "{commands:?}");
    }
    assert!(
        !commands
            .iter()
            .any(|(c, _)| *c == MenuCommand::PdfRemoveNote)
    );
    app.run_menu_command(MenuCommand::PdfComment);
    assert_eq!(app.prompt_kind, crate::app::PromptKind::PdfComment);
    // 长批注折行显示：输入条随行数增高，写过的字都看得见；最多显示 `NOTE_LINES` 行，再多在
    // 输入条里滚动。批注里可以换行。
    paint(&mut app);
    let one_line = app.prompt_bar().h;
    let field = app.goto_input.as_mut().unwrap();
    field.set_text(&"这一段的论证不够充分，".repeat(8));
    paint(&mut app);
    assert!(app.prompt_bar().h > one_line);
    let field = app.goto_input.as_mut().unwrap();
    field.set_text(&"很长的批注".repeat(200));
    paint(&mut app);
    let tallest = crate::layout::note_bar(app.layout.editor, crate::layout::NOTE_LINES);
    assert_eq!(app.prompt_bar(), tallest);
    let field = app.goto_input.as_mut().unwrap();
    field.set_text("");
    assert!(field.insert("第一行\n第二行"));
    assert_eq!(field.text(), "第一行\n第二行");
    app.goto_input.as_mut().unwrap().set_text("改成 Chapter 1");
    app.submit_prompt();
    pdf_until(&mut app, |app| {
        app.pdf.texts.get(&0).is_some_and(|t| !t.notes.is_empty())
    });
    let note = &app.pdf.texts[&0].notes[0];
    assert_eq!(note.kind, AnnotationKind::Highlight);
    assert_eq!(note.comment, "改成 Chapter 1");
    assert!(app.pdf.selection.is_none());

    // 批注已经写进文件：另外打开也能读到。
    let saved = winc_preview::pdf::Document::open(&path).unwrap();
    let notes = saved.annotations(0).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].comment, "改成 Chapter 1");
    drop(saved);
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    assert!(app.pdf.wrote(mtime));

    // 悬停在批注上显示注释；右键批注可以删除它。
    app.on_mouse_move(x0, y0);
    assert_eq!(app.hover, crate::app::Hover::PdfNote(0, 0));
    right_click(&mut app, (x0, y0));
    let commands = menu_commands(&app);
    assert!(
        commands.contains(&(MenuCommand::PdfRemoveNote, true)),
        "{commands:?}"
    );
    app.run_menu_command(MenuCommand::PdfRemoveNote);
    pdf_until(&mut app, |app| {
        app.pdf.texts.get(&0).is_some_and(|t| t.notes.is_empty())
    });
    let saved = winc_preview::pdf::Document::open(&path).unwrap();
    assert!(saved.annotations(0).unwrap().is_empty());
    drop(saved);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 没有 PDFium 时 PDF 只能预览：第一次点击提示下载 PDFium，右键菜单里只有下载；装好后重新
/// 打开，可以选字。安装包取自 `WINC_PDFIUM_ARCHIVE` 指定的本地文件（用 `file://` 地址「下载」，
/// 不联网），没有设置时只检查提示和菜单。已经能加载 PDFium（设置了 `WINC_PDFIUM`）时跳过。
#[cfg(windows)]
#[test]
fn without_pdfium_the_pdf_offers_to_download_it() {
    use crate::app::input::context_menu::MenuCommand;

    if winc_preview::pdf::engine_available() {
        return;
    }
    let dir = sample_folder("pdf-engine");
    winc_preview::pdf::set_engine_dir(dir.join("pdfium"));
    let path = dir.join("论文.pdf");
    std::fs::write(&path, sample_pdf(1)).unwrap();
    let mut app = open_pdf(&path, false);
    assert!(!app.pdf.editable);

    // 第一次点击：提示下载；之后的点击不再提示。
    let r = app.pdf.page_rects[0];
    let (x, y) = (
        app.pdf.area.x + r.x + r.w / 2.0,
        app.pdf.area.y + r.y + 40.0,
    );
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.notice_action, Some(NoticeAction::InstallPdfEngine));
    app.notice = None;
    app.notice_action = None;
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.notice_action, None);

    // 右键：菜单里只有下载 PDFium。
    right_click(&mut app, (x, y));
    assert_eq!(
        menu_commands(&app),
        vec![(MenuCommand::PdfInstallEngine, true)]
    );
    app.context_menu = None;

    let Some(archive) = std::env::var_os("WINC_PDFIUM_ARCHIVE") else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let url = format!(
        "file:///{}",
        std::path::Path::new(&archive)
            .display()
            .to_string()
            .replace('\\', "/")
    );
    app.install_pdf_engine_from(&url);
    for _ in 0..1500 {
        app.poll_pdf();
        if !app.pdf.is_installing() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!app.pdf.is_installing(), "安装 PDFium 超时");
    assert_eq!(app.notice, None, "安装失败");
    assert!(dir.join("pdfium").join("pdfium.dll").is_file());
    pdf_until(&mut app, |app| {
        app.pdf.editable && app.pdf.texts.contains_key(&0)
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// 写长批注时的输入条截图（需要 `WINC_PDFIUM`）。
#[cfg(windows)]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_pdf_comment() {
    let dir = sample_folder("pdf-comment-shot");
    let path = dir.join("论文.pdf");
    std::fs::write(&path, sample_pdf(2)).unwrap();
    let mut app = open_pdf(&path, false);
    assert!(app.pdf.editable, "需要 WINC_PDFIUM");
    pdf_until(&mut app, |app| app.pdf.texts.contains_key(&0));
    app.pdf_select_all();
    app.pdf_comment_selection();
    app.goto_input.as_mut().unwrap().set_text(
        "开篇就是这个呢？这一段的论证不够充分，建议先交代研究背景，再引出问题；\
         另外第二句和第三句的意思重复了，可以合并成一句。\n还有：图 1 的说明文字太短。",
    );
    shoot(&mut app, "pdf-comment");
    let _ = std::fs::remove_dir_all(&dir);
}
