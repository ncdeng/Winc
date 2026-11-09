//! 界面主题：深色、浅色两套配色，以及统一的字号、区域尺寸和圆角半径。
//!
//! 尺寸常量的单位均为逻辑像素。[`Palette`] 保存一套配色，[`ViewFonts`] 保存运行期的字号和行高；
//! 文件类型图标的记号和颜色由 [`file_kind`] 按文件名给出。

mod kinds;
mod palettes;
mod winc;

pub use kinds::*;
pub use palettes::*;
pub use winc::*;

use std::borrow::Cow;
use std::time::Duration;

use winc_config::BADGE_COLORS;
use winc_render::Color;
use winc_syntax::TokenKind;

use crate::icons::FileGlyph;

/// 一套配色，深色（[`DARK`]）和浅色（[`LIGHT`]）各有一个实例。
///
/// 颜色字段按用途分组：底色、线条与阴影、文字、交互状态、编辑区、语法高亮。
pub struct Palette {
    /// 是否为深色配色。
    pub dark: bool,

    // 底色
    /// 窗口底色：侧栏、编辑区两张卡片四周和之间的留白，以及状态栏。
    pub frame_bg: Color,
    /// 侧栏卡片的底色；所有配色里都与编辑区相同。
    pub sidebar_bg: Color,
    /// 编辑区卡片的底色，代码和差异都画在此颜色上。
    pub editor_bg: Color,
    /// 浮层底色：快速打开、菜单和设置卡片。
    pub popup_bg: Color,
    /// 当前标签的底色和外圈细边，带一点强调色；未选中的标签是凸起的一块灰。
    pub tab_fill: Color,
    /// 标签的描边：当前标签带强调色，未选中的标签是同色系的一圈细线，把标签与卡片分开。
    pub tab_edge: Color,
    /// 设置页中一组设置项的底色：叠在编辑区底色之上，浅色配色下比编辑区底色略深，深色配色下略浅。
    pub group_bg: Color,

    // 线条与阴影
    /// 卡片内部的分隔线、设置项的描边。
    pub border: Color,
    /// 浮层的描边；比 `border` 淡，浮层叠在卡片上时不显重。
    pub popup_border: Color,
    /// 浮层的投影颜色（半透明的黑）。
    pub shadow: Color,
    /// 相邻两个未选中标签之间的竖线。
    pub tab_divider: Color,
    /// 设置等模态浮层后方覆盖整个窗口的半透明遮罩。
    pub scrim: Color,

    // 文字
    /// 界面正文：文件树、标签、按钮。
    pub text: Color,
    /// 次要文字：路径、说明、未选中的标签。
    pub text_muted: Color,
    /// 更淡的文字：占位符、快捷键提示。
    pub text_dim: Color,
    /// 铺满强调色的底上的文字颜色。
    pub text_on_accent: Color,
    /// 细线、图标和链接字用的强调色；整块铺满时改用 `accent_fill`。
    pub accent: Color,
    /// 整块铺满强调色的底（主按钮、勾上的勾选框、当前分支的标签），上面的字用
    /// `text_on_accent`。
    ///
    /// 深色配色里比 `accent` 暗、灰一些：`accent` 是给细线、图标和字用的
    /// 亮蓝，整块铺开太刺眼。
    pub accent_fill: Color,
    /// 出错时的文字、图标和描边颜色。
    pub error: Color,
    /// 不算出错但需要留意的提醒（例如工具未安装），比 `error` 温和。
    pub warning: Color,
    /// 状态栏中提示文字的颜色；与 `warning` 同色，不算出错。
    pub status_message: Color,

    // 交互状态
    /// 鼠标悬停的列表行和按钮的底色，半透明，叠在下面的卡片上。
    pub hover: Color,
    /// 列表和侧栏中选中项的底色。
    pub selected: Color,
    /// 窗口没有焦点时选中项的底色，比 `selected` 淡。
    pub selected_inactive: Color,
    /// 列表（文件树、搜索结果、改动列表）没有焦点时选中行的底色。
    ///
    /// 带少量强调色，深度为 `selected` 的一半。若使用中性灰，与悬停的浅灰差别过小，
    /// 焦点在编辑区时无法分辨选中的是哪一行。
    pub selected_unfocused: Color,
    /// 关闭状态的开关底色；开启状态使用 `accent`。
    pub switch_off: Color,
    /// 开关滑块的底色。
    pub switch_knob: Color,
    /// 设置页一组内，行与行之间的细线颜色。
    pub separator: Color,
    /// 文件树中目录名的颜色，与次要文字同色，和文件名区分。
    pub folder: Color,

    // 编辑区
    /// 行号栏中普通行号的颜色。
    pub gutter_text: Color,
    /// 行号栏中光标所在行的行号颜色，比 `gutter_text` 亮。
    pub gutter_text_current: Color,
    /// 光标所在行的底色。
    pub current_line: Color,
    /// 选区中文字的底色。
    pub selection: Color,
    /// 编辑区没有键盘焦点时选区的底色。
    ///
    /// 焦点在查找条、文件树、终端，或窗口在后台时使用：由三成 `selection` 与中性灰混合而成，
    /// 颜色淡且偏灰，使用户能够直接分辨键入的文字不会进入编辑区。
    pub selection_inactive: Color,
    /// 光标竖线的颜色。
    pub caret: Color,
    /// 缩进参考线的颜色。
    pub indent_guide: Color,
    /// 光标所在代码块的缩进参考线颜色，比 `indent_guide` 深。
    pub indent_guide_active: Color,
    /// 查找结果中非当前匹配的底色。
    pub search_match: Color,
    /// 查找结果中当前匹配的底色，比 `search_match` 浓。
    pub search_current: Color,
    /// 配对括号的底色。
    pub match_brace: Color,
    /// 光标所在的词在其他位置的出现；比选区淡得多，并与括号匹配的灰色区分开。
    pub occurrence: Color,
    /// 滚动条滑块的颜色，半透明，叠在卡片上。
    pub scrollbar: Color,
    /// 滚动条上标出光标所在词的其他出现所在行的颜色：不带色相的灰，和 Git 改动的蓝、绿分开。
    pub ruler_occurrence: Color,
    /// 滚动条上标出光标所在行的颜色。
    pub ruler_caret: Color,
    /// 鼠标悬停在滚动条上或正在拖动滚动条时，滑块的颜色。
    pub scrollbar_active: Color,
    /// 在文件树中拖动时，放置目标目录及其子行的底色。
    pub drop_target: Color,
    /// 标签上表示有未保存修改的圆点颜色。
    pub modified_dot: Color,
    /// Git 状态：修改、改名的文件名和行号栏标记。
    pub git_modified: Color,
    /// Git 状态：新增、未跟踪的文件名和行号栏标记。
    pub git_added: Color,
    /// Git 状态：删除的文件名和行号栏标记。
    pub git_deleted: Color,
    /// Git 状态：有冲突的文件名。
    pub git_conflict: Color,
    /// 差异里修改的一段（两侧都有行）。
    pub diff_modified: DiffShade,
    /// 差异里新增的一段（只有右侧有行）。
    pub diff_added: DiffShade,
    /// 差异里删掉的一段（只有左侧有行）。
    pub diff_deleted: DiffShade,
    /// 项目徽章的（底色，字色）；下标是最近列表为每个目录选定的颜色序号（`State::badge_color`）。
    ///
    /// 各色的色相在色环上相隔，十种颜色并列时仍可区分。
    pub badges: [(Color, Color); BADGE_COLORS],

    // 语法高亮
    /// 代码正文；与界面正文 `text` 同色，使文件树、标签与代码在相同字号下的笔画粗细一致。
    pub syntax_text: Color,
    /// 关键字。
    pub syntax_keyword: Color,
    /// 内建的类型与函数名。
    pub syntax_builtin: Color,
    /// 字符串字面量。
    pub syntax_string: Color,
    /// 数字字面量。
    pub syntax_number: Color,
    /// 注释。
    pub syntax_comment: Color,
    /// 函数名与函数定义。
    pub syntax_function: Color,
    /// 装饰器、注解。
    pub syntax_decorator: Color,
    /// 配置文件中的键、样式表中的属性名。
    pub syntax_property: Color,

    /// 终端的 16 色（黑、红、绿、黄、蓝、品红、青、白，再是各自的亮色）。
    pub ansi: [Color; 16],
}

/// 差异里一种改动的三层颜色，都是不透明的柔和色，长时间看也不刺眼。
#[derive(Debug, Clone, Copy)]
pub struct DiffShade {
    /// 改动的行和中间带子的底色。
    pub line: Color,
    /// 行里改了的字的底色。
    pub word: Color,
    /// 带子的边和一侧没有行时的插入线。
    pub edge: Color,
    /// 行号栏里的改动标记：比边线深一些，细细一条也看得出，但不像 Git 状态色那样刺眼。
    pub mark: Color,
}

/// 颜色的相对亮度（0–1，按 WCAG 2.1 的定义）。
pub(crate) fn luminance(c: Color) -> f32 {
    let lin = |x: f32| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// 两个颜色的对比度（1–21，按 WCAG 2.1 的定义）。
pub(crate) fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// 返回调整后的 `fg`：在 `bg` 上的对比度至少为 `min`。
///
/// 方向跟随 `fg` 本身：比底色亮的往白调，暗的往黑调，色相大致不变；朝该方向调到极端仍不够时
/// 才翻到另一方向。判断方向不看底色的明暗：终端程序常在浅色底上写黑字，这类底色按明度算偏
/// 深，按底色方向调会把黑字推成白字。
///
/// `min` 不大于 1，或 `fg` 与 `bg` 相同时原样返回，终端程序用相同的字色与底色隐去文字。
pub(crate) fn readable(fg: Color, bg: Color, min: f32) -> Color {
    if min <= 1.0 || fg == bg || contrast(fg, bg) >= min {
        return fg;
    }
    let lighter = luminance(fg) > luminance(bg);
    let (near, far) = if lighter {
        (WHITE, BLACK)
    } else {
        (BLACK, WHITE)
    };
    let toward = if contrast(near, bg) >= min || contrast(near, bg) >= contrast(far, bg) {
        near
    } else {
        far
    };
    // 反向调整时直接取纯黑或纯白：原有颜色已无法保留，取刚好达标的中间值时文字呈灰色。
    if toward == far {
        return far;
    }
    // 翻到另一头时要先越过底色，对比度先变小再变大：只算越过以后的。
    let past = |c: Color| (luminance(c) > luminance(bg)) == (toward == WHITE);
    // 二分找最小的调和比例：越过底色以后，对比度随比例单调变大。
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..12 {
        let mid = (lo + hi) / 2.0;
        let c = mix(toward, fg, mid);
        if past(c) && contrast(c, bg) >= min {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    mix(toward, fg, hi)
}

const WHITE: Color = Color::hex(0xFFFFFF);
const BLACK: Color = Color::hex(0x000000);

// 每个主题只写基本颜色（`Tokens`），其余按同一套规则算（`Palette::build`）：各主题的层次和
// 对比度一致，换主题只换色相。两张卡片（侧栏、编辑区）在所有主题里是同一个颜色；窗框（四周的
// 留白、状态栏）由各主题的 `frame` 决定，比卡片深一档或浅一档。

/// 项目徽章：深色主题用浅色底、深色字，各色的色相在色环上相隔。
const BADGES_DARK: [(Color, Color); BADGE_COLORS] = {
    let ink = Color::hex(0x1A1B1F);
    [
        (Color::hex(0xF2949F), ink),
        (Color::hex(0xF5B07E), ink),
        (Color::hex(0xEBCB72), ink),
        (Color::hex(0x9AD3A2), ink),
        (Color::hex(0x7DD3CB), ink),
        (Color::hex(0x86C5EE), ink),
        (Color::hex(0x9FB4F7), ink),
        (Color::hex(0xC4A8F2), ink),
        (Color::hex(0xEBA0D2), ink),
        (Color::hex(0xB6BFCC), ink),
    ]
};

/// 项目徽章的配色：浅色主题用饱和度较高的底色、白字。
const BADGES_LIGHT: [(Color, Color); BADGE_COLORS] = [
    (Color::hex(0xD9475D), WHITE),
    (Color::hex(0xDE7333), WHITE),
    (Color::hex(0xB98618), WHITE),
    (Color::hex(0x3E9C58), WHITE),
    (Color::hex(0x1F948C), WHITE),
    (Color::hex(0x2E86C7), WHITE),
    (Color::hex(0x4767DB), WHITE),
    (Color::hex(0x8454D4), WHITE),
    (Color::hex(0xC14596), WHITE),
    (Color::hex(0x637189), WHITE),
];

/// 一套主题的基本颜色。
struct Tokens {
    /// 是否为深色主题；决定后续层次是叠白还是叠黑。
    dark: bool,
    /// 窗框：两张卡片四周的留白、顶栏和状态栏。
    frame: Color,
    /// 两张卡片（侧栏、编辑区）共用的底色。
    card: Color,
    /// 浮层（快速打开、菜单、设置）的底色。
    popup: Color,
    /// 界面文字：文件树、标签、按钮。
    text: Color,
    /// 次要文字：路径、说明、没选中的标签。
    text_muted: Color,
    /// 更淡的文字：占位、提示。
    text_dim: Color,
    /// 细线、图标和链接字用的强调色。
    accent: Color,
    /// 整块铺满的强调色（主按钮、勾选框），上面是白字。
    accent_fill: Color,
    /// 编辑区选中文字的底色。
    selection: Color,
    /// 行号：普通的、光标所在行的。
    gutter: [Color; 2],
    error: Color,
    warning: Color,
    /// Git 状态：修改、新增、删除、冲突。
    ///
    /// 文件名、行号栏标记、差异的底色都从这里来，同一种改动在哪里都是同一个颜色；差异里的删除
    /// 例外，见 [`DELETED_DARK`]。
    git: [Color; 4],
    /// 当前标签是否带强调色；为 `false` 时是凸起的一块灰。
    tinted_tab: bool,
    /// 语法色：正文、关键字、类型和内建名、字符串、数字、注释、函数、装饰器、属性名。
    ///
    /// 浅色主题的语法色按色相压暗：在卡片底色上对比度至少 6:1，注释至少 4.5:1，否则细笔画
    /// 的彩色字在白底上显得发虚。
    syntax: [Color; 9],
    /// 终端的 16 色（黑、红、绿、黄、蓝、品红、青、白，再是各自的亮色），与所在主题的语法色
    /// 同一色系。
    ///
    /// 除了深色主题的黑、浅色主题的亮白（这两色本来就接近底色），其余 15 色在卡片底色上的
    /// 对比度至少 3:1。
    ansi: [Color; 16],
}

/// 把 `fg` 按 `t`（0–1）叠在不透明的 `bg` 上，得到不透明的颜色。
const fn mix(fg: Color, bg: Color, t: f32) -> Color {
    Color {
        r: bg.r + (fg.r - bg.r) * t,
        g: bg.g + (fg.g - bg.g) * t,
        b: bg.b + (fg.b - bg.b) * t,
        a: 1.0,
    }
}

/// 差异里一种改动的颜色：改动色 `mark` 按四档比例叠在卡片底色上。
///
/// 四档比例依次用于行、行内改动的字、边线和行号栏标记；深色主题叠得浓一些，否则在深色底上
/// 看不出来。比例取自调参，按两种配色下改动可见程度相当校准。
/// 深色主题下差异里删除的颜色：比编辑区底色亮、不带色相的灰。
const DELETED_DARK: Color = palettes::hex(0x9DA1A8);
/// 浅色主题下差异里删除的颜色：比编辑区底色暗、不带色相的灰。
const DELETED_LIGHT: Color = palettes::hex(0x70737D);

const fn shade(mark: Color, card: Color, dark: bool) -> DiffShade {
    let t = if dark {
        [0.21, 0.36, 0.46, 0.75]
    } else {
        [0.16, 0.30, 0.42, 0.62]
    };
    DiffShade {
        line: mix(mark, card, t[0]),
        word: mix(mark, card, t[1]),
        edge: mix(mark, card, t[2]),
        mark: mix(mark, card, t[3]),
    }
}

impl Palette {
    /// 按基本颜色 `t` 算出整套配色。
    ///
    /// 深色主题在卡片上叠白、浅色主题叠黑，得到悬停、选中、边线、当前行这些层次；两边的浓度
    /// 各自调过，看上去一样深。
    const fn build(t: Tokens) -> Palette {
        let dark = t.dark;
        let ink = if dark { WHITE } else { BLACK };
        // 当前标签：叠在卡片上的颜色和底、边各叠多少。
        let (tab, fill, edge) = match (t.tinted_tab, dark) {
            (true, true) => (t.accent_fill, 0.24, 0.6),
            (true, false) => (t.accent_fill, 0.1, 0.45),
            (false, true) => (ink, 0.085, 0.14),
            (false, false) => (ink, 0.04, 0.11),
        };
        let (tab_fill, tab_edge) = (mix(tab, t.card, fill), mix(tab, t.card, edge));
        Palette {
            dark,
            frame_bg: t.frame,
            sidebar_bg: t.card,
            editor_bg: t.card,
            popup_bg: t.popup,
            tab_fill,
            tab_edge,
            group_bg: mix(ink, t.card, if dark { 0.035 } else { 0.03 }),
            border: mix(ink, t.card, if dark { 0.07 } else { 0.09 }),
            popup_border: ink.alpha(if dark { 0.09 } else { 0.08 }),
            shadow: BLACK.alpha(if dark { 0.5 } else { 0.14 }),
            tab_divider: ink.alpha(if dark { 0.08 } else { 0.1 }),
            scrim: BLACK.alpha(if dark { 0.4 } else { 0.08 }),
            text: t.text,
            text_muted: t.text_muted,
            text_dim: t.text_dim,
            text_on_accent: WHITE,
            accent: t.accent,
            accent_fill: t.accent_fill,
            error: t.error,
            warning: t.warning,
            status_message: t.warning,
            hover: ink.alpha(if dark { 0.05 } else { 0.045 }),
            selected: if dark {
                mix(t.accent_fill, t.card, 0.32)
            } else {
                mix(t.accent_fill, t.card, 0.16)
            },
            selected_inactive: mix(ink, t.card, if dark { 0.12 } else { 0.07 }),
            selected_unfocused: if dark {
                mix(t.accent_fill, t.card, 0.16)
            } else {
                mix(t.accent_fill, t.card, 0.08)
            },
            switch_off: ink.alpha(if dark { 0.16 } else { 0.14 }),
            switch_knob: WHITE,
            separator: ink.alpha(if dark { 0.06 } else { 0.07 }),
            folder: t.text_muted,
            gutter_text: t.gutter[0],
            gutter_text_current: t.gutter[1],
            current_line: ink.alpha(if dark { 0.03 } else { 0.025 }),
            selection: t.selection,
            selection_inactive: mix(
                t.selection,
                mix(ink, t.card, if dark { 0.12 } else { 0.08 }),
                0.3,
            ),
            caret: if dark { mix(WHITE, t.text, 0.4) } else { BLACK },
            indent_guide: ink.alpha(if dark { 0.07 } else { 0.08 }),
            indent_guide_active: ink.alpha(if dark { 0.2 } else { 0.24 }),
            // 查找结果取主题的 Git 新增色（绿）：黄色压低浓度叠在深色卡片上偏褐，蓝色又与选区、
            // 选中行撞色，只有绿色既能看清又不与已有的标记混淆。
            search_match: t.git[1].alpha(if dark { 0.34 } else { 0.24 }),
            search_current: t.git[1].alpha(if dark { 0.62 } else { 0.46 }),
            match_brace: ink.alpha(if dark { 0.12 } else { 0.1 }),
            occurrence: t.accent.alpha(if dark { 0.14 } else { 0.1 }),
            scrollbar: ink.alpha(if dark { 0.16 } else { 0.18 }),
            ruler_occurrence: ink.alpha(if dark { 0.34 } else { 0.24 }),
            ruler_caret: ink.alpha(if dark { 0.55 } else { 0.45 }),
            scrollbar_active: ink.alpha(if dark { 0.32 } else { 0.36 }),
            drop_target: t.accent.alpha(if dark { 0.14 } else { 0.1 }),
            badges: if dark { BADGES_DARK } else { BADGES_LIGHT },
            modified_dot: t.text,
            git_modified: t.git[0],
            git_added: t.git[1],
            git_deleted: t.git[2],
            git_conflict: t.git[3],
            diff_modified: shade(t.git[0], t.card, dark),
            diff_added: shade(t.git[1], t.card, dark),
            // 删除用中性灰，不随主题的 Git 状态色变红：红色留给合并冲突。
            diff_deleted: shade(
                if dark { DELETED_DARK } else { DELETED_LIGHT },
                t.card,
                dark,
            ),
            syntax_text: t.syntax[0],
            syntax_keyword: t.syntax[1],
            syntax_builtin: t.syntax[2],
            syntax_string: t.syntax[3],
            syntax_number: t.syntax[4],
            syntax_comment: t.syntax[5],
            syntax_function: t.syntax[6],
            syntax_decorator: t.syntax[7],
            syntax_property: t.syntax[8],
            ansi: t.ansi,
        }
    }
}

/// 项目徽章：项目名中的前两个字母或数字，以及最近列表为该目录选定的颜色。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectBadge {
    /// 徽章上显示的字符（字母转为大写）；项目名中没有字母或数字时为「W」。
    pub initials: String,
    /// 徽章底色。
    pub background: Color,
    /// 徽章文字颜色。
    pub text: Color,
}

impl Palette {
    /// 返回名为 `name`、颜色序号为 `color`（`State::badge_color`）的项目徽章。
    ///
    /// 颜色序号对 `BADGE_COLORS` 取余后选色。
    pub fn project_badge(&self, name: &str, color: usize) -> ProjectBadge {
        let initials: String = name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect();
        let (background, text) = self.badges[color % BADGE_COLORS];

        ProjectBadge {
            initials: if initials.is_empty() {
                "W".to_owned()
            } else {
                initials
            },
            background,
            text,
        }
    }

    /// 返回语法标记 `kind` 对应的颜色。
    pub fn token(&self, kind: TokenKind) -> Color {
        match kind {
            TokenKind::Text => self.syntax_text,
            TokenKind::Keyword => self.syntax_keyword,
            TokenKind::Builtin => self.syntax_builtin,
            TokenKind::String => self.syntax_string,
            TokenKind::Number => self.syntax_number,
            TokenKind::Comment => self.syntax_comment,
            TokenKind::Function => self.syntax_function,
            TokenKind::Decorator => self.syntax_decorator,
            TokenKind::Property => self.syntax_property,
            // 以下类别还没有单独的主题色，借用词法上最接近的一类。
            TokenKind::Variable => self.syntax_property,
            TokenKind::Type => self.syntax_builtin,
            TokenKind::Escape => self.syntax_number,
            TokenKind::Tag => self.syntax_keyword,
            TokenKind::Attribute => self.syntax_property,
            TokenKind::Heading => self.syntax_decorator,
            TokenKind::Emphasis => self.syntax_builtin,
            TokenKind::Strong => self.syntax_keyword,
            TokenKind::Link => self.syntax_function,
            TokenKind::Inserted => self.syntax_string,
            TokenKind::Deleted => self.error,
        }
    }

    /// 返回文件类型图标：纸面上的记号表示大类，颜色表示语言；浅色配色使用较深的颜色。
    pub fn file_icon(&self, name: &str) -> (FileGlyph, Color) {
        let (glyph, [dark, light]) = file_kind(name);
        (glyph, Color::hex(if self.dark { dark } else { light }))
    }
}

// 字号与行高（逻辑像素，下同）。上方的常量是默认值；运行时以 `ViewFonts` 为准，
// 字号可以在配置中修改。
/// 界面字体的默认字号。
pub const UI_FONT: f32 = 13.0;
/// 界面次要文字的默认字号。
pub const UI_FONT_SMALL: f32 = 12.0;
/// 项目名首字母徽章的默认字号。
pub const INITIALS_FONT: f32 = 10.0;
/// 编辑区代码的默认字号。
///
/// 比界面字号小 1 像素：代码的行距由 [`LINE_HEIGHT_RATIO`] 决定，字号放小后一屏多出几行，
/// 代码也不显得拥挤。
pub const CODE_FONT: f32 = 12.0;

/// 行高与字号的比值：13 号字配 20 的行高。
///
/// 编辑区的行高按字号等比缩放，使字号变化后行距的比例不变；默认 12 号字的行高是 18。
pub const LINE_HEIGHT_RATIO: f32 = 20.0 / 13.0;

/// 运行期的字号和行高（逻辑像素）。
///
/// 界面上每处文字都用这里的值，不直接使用上方的默认常量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFonts {
    /// 界面字体（文件树、标签、状态栏）。
    pub ui: f32,
    /// 界面的次要文字（路径、说明、快捷键提示）。
    pub ui_small: f32,
    /// 项目名首字母徽章。
    pub initials: f32,
    /// 编辑区代码。
    pub code: f32,
    /// 编辑区行高；行号与坐标的换算以该值为准。
    pub line_height: f32,
    /// 终端的字号。
    pub terminal: f32,
    /// 终端的行高，算法与编辑区行高相同。
    pub terminal_line_height: f32,
}

impl ViewFonts {
    /// 按编辑区字号 `code`、界面字号 `ui` 和终端字号 `terminal` 算出整套度量。
    ///
    /// 行高取整到整像素：否则基线会落在半像素位置，相邻两行字形的浓淡不一致。
    ///
    /// 次要文字和徽章与界面字号保持固定的差值（与默认值相同），下限分别为 8 像素和 7 像素。
    /// 不按比例缩放：界面字号为 14 时按比例算出 12.92，字形落不到整像素上，笔画发虚，
    /// 看起来和其他文字粗细不一。
    pub fn new(code: f32, ui: f32, terminal: f32) -> ViewFonts {
        let line_height = |size: f32| (size * LINE_HEIGHT_RATIO).round().max(8.0);
        ViewFonts {
            ui,
            ui_small: (ui - (UI_FONT - UI_FONT_SMALL)).max(8.0),
            initials: (ui - (UI_FONT - INITIALS_FONT)).max(7.0),
            code,
            line_height: line_height(code),
            terminal,
            terminal_line_height: line_height(terminal),
        }
    }

    /// 按 `config` 中的字号算出整套度量，未设置的用默认字号。
    ///
    /// 终端字号与编辑区字号互不影响：终端未设置时用编辑区的默认字号 [`CODE_FONT`]，不随编辑区
    /// 字号变化。
    pub fn of(config: &winc_config::Config) -> ViewFonts {
        let code = config.editor_size.unwrap_or(CODE_FONT);
        let ui = config.ui_size.unwrap_or(UI_FONT);
        ViewFonts::new(code, ui, config.terminal_size.unwrap_or(CODE_FONT))
    }
}

impl Default for ViewFonts {
    fn default() -> Self {
        ViewFonts::new(CODE_FONT, UI_FONT, CODE_FONT)
    }
}

// 区域尺寸（逻辑像素）：按三档字号下的界面比例调参得出。
/// 顶栏高度：红绿灯、侧栏开关和标签都在这一行，位于两张卡片的顶部。
pub const TOP_BAR: f32 = 40.0;
/// 窗口边缘到侧栏、编辑区两张卡片的留白；卡片底边贴着状态栏，下方不留。
pub const ISLAND_MARGIN: f32 = 6.0;
/// 侧栏卡片与编辑区卡片之间的间距，拖动调整侧栏宽度的分隔条就在这里。
pub const ISLAND_GAP: f32 = 6.0;
/// 顶栏下边缘的纵坐标：卡片上方的留白加顶栏高度。
pub const TOP_BAR_END: f32 = ISLAND_MARGIN + TOP_BAR;
/// 状态栏高度。
pub const STATUS_HEIGHT: f32 = 26.0;
/// 侧栏第二行（项目、分支和当前视图的工具）的高度。
pub const SIDEBAR_HEADER: f32 = 32.0;
/// 侧栏里上下相邻两块（第二行的按钮、输入框、「提交」按钮）之间看得见的间距，各个视图统一。
pub const SIDEBAR_GAP: f32 = 8.0;
/// 第二行下面第一块的纵向偏移（相对第二行下沿）：第二行里的按钮都是 `BUTTON_SMALL` 高、上下
/// 居中，下面已经留了 4，再加 4 正好是 `SIDEBAR_GAP`。
pub const SIDEBAR_FIRST: f32 = SIDEBAR_GAP - (SIDEBAR_HEADER - BUTTON_SMALL) / 2.0;
/// 输入框、按钮到下方列表的间距：列表行自带上下留白（24 高的行里内容约 16），
/// 因此只留一半，视觉上与 `SIDEBAR_GAP` 相等。
///
/// 文件树紧接在第二行下方，间距也是该值。
pub const SIDEBAR_LIST_GAP: f32 = SIDEBAR_GAP / 2.0;
/// 侧栏的默认宽度。
pub const DEFAULT_PANEL_WIDTH: f32 = 300.0;
/// 侧栏的最小宽度。
///
/// macOS 上要恰好放下顶部的红绿灯（右端 80）、侧栏开关和靠右的视图切换（文件、搜索、提交三段
/// 的分段控件，由 `layout::view_tabs` 计算）：开关右端 118，隔 `SEGMENT_GAP` 6，底槽
/// 3 × `SEGMENT_W` + 2 × `SEGMENT_SPACING` + 4 = 92，离侧栏右边 8，再加 `ISLAND_GAP` / 2，
/// 合计 227；取 230 留出余量。
pub const MIN_PANEL_WIDTH: f32 = 230.0;
/// 文件树一行的高度。
pub const TREE_ROW: f32 = 24.0;
/// 文件树每一级缩进的宽度。
pub const TREE_INDENT: f32 = 16.0;
/// 标签的高度（编辑区和终端面板的标签同高），与侧栏「文件 / 搜索 / 提交」分段控件的底槽
/// 同高（每段 22，上下各留 2）。
pub const TAB_HEIGHT: f32 = 26.0;
/// 标签之间的间距。
pub const TAB_GAP: f32 = 4.0;
/// 第一个文件标签离标签栏左边的距离（逻辑像素），与标签上下的留白相同。
pub const TAB_INSET: f32 = (TOP_BAR - TAB_HEIGHT) / 2.0;
/// 标签的最大宽度。
///
/// 标签宽度贴合文件名，文件名更长时截短；所有标签放不下时横向滚动。
pub const TAB_WIDTH_MAX: f32 = 180.0;
/// 标签的最小宽度：文件名很短时也不窄于该宽度，留出足够的点击范围。
pub const TAB_WIDTH_MIN: f32 = 80.0;
/// 查找条、跳转条（单行）的高度。
pub const BAR_HEIGHT: f32 = 36.0;
/// 多行输入框（提交信息、PDF 批注）一行的高度。
pub const AREA_LINE: f32 = 20.0;

// 按钮尺寸：只有 `BUTTON`、`BUTTON_SMALL` 和 `TOGGLE_W`（宽度）三种，圆角均为 `RADIUS_ROW`。
/// 顶栏上的按钮（侧栏开关、「文件 / 搜索」、新建标签）和设置卡片的关闭按钮。
pub const BUTTON: f32 = 28.0;
/// 工具栏、查找条、列表行上的图标按钮。
pub const BUTTON_SMALL: f32 = 24.0;
/// 查找选项开关（Aa、ab、.*）和「过滤」按钮的宽度；图标有两个字符宽，因此比 `BUTTON_SMALL` 宽，
/// 高度与 `BUTTON_SMALL` 相同。
pub const TOGGLE_W: f32 = 26.0;
/// 并排的按钮之间的间距。
pub const BUTTON_GAP: f32 = 2.0;

// 圆角半径（逻辑像素）
/// 列表行和按钮的圆角半径。
pub const RADIUS_ROW: f32 = 6.0;
/// 标签的圆角半径，与侧栏分段控件的底槽相同。
pub const RADIUS_TAB: f32 = RADIUS_ROW + 2.0;
/// 菜单、悬停提示、改动卡片这类贴着内容弹出的小浮层的圆角半径。
pub const RADIUS_POPUP: f32 = RADIUS_ROW + 2.0;
/// 查找条、跳转条、右上角的提醒这类浮动单行条的圆角半径。
pub const RADIUS_BAR: f32 = 10.0;
/// 设置等较大的浮动卡片的圆角半径（逻辑像素）。
pub const RADIUS_CARD: f32 = 12.0;
/// 侧栏、编辑区两张卡片的圆角半径。
pub const RADIUS_ISLAND: f32 = 10.0;
/// 文本选区的圆角半径。
pub const RADIUS_SELECTION: f32 = 4.0;

// 编辑区内部
/// 行号栏左右两侧的留白（逻辑像素）。
pub const GUTTER_PADDING: f32 = 18.0;
/// 文本区左侧的留白（逻辑像素）。
pub const TEXT_PAD: f32 = 8.0;
/// 滚动条可点击、可拖动区域的宽度（逻辑像素），从区域的右边缘（水平滚动条为下边缘）向内计算。
pub const SCROLLBAR_HIT: f32 = 12.0;
/// 缩略图中一行的高度（逻辑像素）。
pub const MINIMAP_ROW: f32 = 2.0;
/// 滚动条滑块的活动范围在轨道两端各让出的距离（逻辑像素）：滑块贴着卡片的边，到头时不能
/// 伸出卡片的圆角（圆角半径 10，离边 1 像素处要往里收约 6 像素）。
pub const SCROLLBAR_END: f32 = 6.0;
/// 滚动条滑块的粗细（逻辑像素）。
pub const SCROLLBAR_THUMB: f32 = 6.0;
/// 滑块离卡片边缘的距离（逻辑像素）：紧贴着卡片的右边（水平滚动条是下边），只留一线。
pub const SCROLLBAR_INSET: f32 = 1.0;
/// 加载图标每秒旋转的圈数（圈）。
///
/// 1.25 圈每秒即 0.8 秒一圈：同步、提交、刷新的图标都按它转，转速一致。
pub const SPIN_SPEED: f32 = 1.25;
/// 鼠标悬停在按钮上多久后显示提示（毫秒）。
///
/// 与 macOS 的悬停提示一致：指针停留约 0.5 s 后出现，扫过按钮时不弹出。
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

// 设置浮层（⌘,）的尺寸（逻辑像素），取自调参，按各页内容在默认字号下无滚动条放下校准。浮层
// 分左右两栏：左侧是分类栏，
// 右侧是一页，分几组，每行一项：名称在左，控件靠右。
/// 设置卡片的宽度；换页时不变，窗口放不下时缩小，内容滚动。
pub const SETTINGS_W: f32 = 720.0;
/// 设置卡片的高度；换页时不变，窗口放不下时缩小，内容滚动。
pub const SETTINGS_H: f32 = 520.0;
/// 设置卡片与窗口四周至少保留的距离。
pub const SETTINGS_MARGIN: f32 = 32.0;
/// 左侧分类栏的宽度。
pub const SETTINGS_NAV_W: f32 = 180.0;
/// 分类栏中每一项的高度。
pub const SETTINGS_NAV_ROW: f32 = 30.0;
/// 右侧顶部标题行（「设置 › 外观」和关闭按钮）的高度。
pub const SETTINGS_HEADER: f32 = 52.0;
/// 组与右栏左右边缘的距离；行内的名称和控件再向内缩进 `SETTINGS_INSET`。
pub const SETTINGS_PAD: f32 = 24.0;
/// 行内名称和控件相对组边缘的内缩距离。
pub const SETTINGS_INSET: f32 = 14.0;
/// 两组之间的间距。
pub const SETTINGS_GROUP_GAP: f32 = 16.0;
/// 最后一组下方的留白。
pub const SETTINGS_BOTTOM: f32 = 24.0;
/// 一行的高度（名称下方没有说明文字时）。
pub const SETTINGS_ROW: f32 = 40.0;
/// 名称下方还有一行说明文字时的行高。
pub const SETTINGS_ROW_TALL: f32 = 54.0;
/// 文字、图标类控件（菜单的值、字体名、数值步进、版本号）的底色在可见内容右侧留出的边距。
///
/// 可见内容的右缘与组内分隔线的右端对齐，开关、分段控件、按钮这类有底色的控件右缘也对齐到
/// 该处；悬停底色因此比可见内容向右多出这段边距。
pub const SETTINGS_GLYPH_PAD: f32 = 8.0;
/// 「关于」页顶部产品信息一行的高度：上方留白 20、图标、间隔 10、名称一行 28、间隔 2、版本号
/// 一行 24，下方留白 16。
pub const SETTINGS_PRODUCT_H: f32 = 164.0;
/// 「关于」页顶部产品图标的边长（逻辑像素）。
pub const SETTINGS_LOGO: f32 = 64.0;
/// 说明文字折成多行时，每多一行增加的行高：说明文字的行距。
pub const SETTINGS_DESC_LINE: f32 = 18.0;
/// 说明文字最多折成的行数；更窄时最后一行在末尾截短。
pub const SETTINGS_DESC_LINES: usize = 3;

/// 组上方标题行的高度（含与组之间的间距）。
pub const SETTINGS_TITLE: f32 = 26.0;
/// 组下方文字按钮（如「添加语言…」）的宽度。
pub const SETTINGS_LINK_W: f32 = 120.0;
/// 组下方文字按钮（如「添加语言…」）的高度。
pub const SETTINGS_LINK_H: f32 = 30.0;
/// 控件的高度。
pub const SETTINGS_CONTROL_H: f32 = 28.0;
/// 点击后弹出菜单的选择控件（工具名、排除的语言）的宽度；比字体名输入框窄，为左侧名称下方的
/// 说明文字留出更多空间。
pub const SETTINGS_CHOICE_W: f32 = 150.0;
/// 设置行右侧按钮（「检查更新」）的宽度。
pub const SETTINGS_BUTTON_W: f32 = 96.0;
/// 自定义格式化命令输入框的宽度；该设置平时是点击后弹出菜单的选项，选自定义命令时才换成输入框。
pub const SETTINGS_COMMAND_W: f32 = 290.0;
/// 加减控件中单个按钮的宽度。
pub const SETTINGS_STEP_W: f32 = 28.0;
/// 加减控件中数值区域的宽度。
pub const SETTINGS_VALUE_W: f32 = 36.0;
/// 开关的宽度。
pub const SETTINGS_SWITCH_W: f32 = 32.0;
/// 开关的高度。
pub const SETTINGS_SWITCH_H: f32 = 18.0;
/// 「模式」分段控件（跟随系统、浅色、深色）每一段的宽度：按字数定，名字左右各留 10 左右，
/// 不是三段一样宽（一样宽时两个字的那两段左右空得太多）。
pub const SETTINGS_SEGMENT_W: [f32; 3] = [68.0, 44.0, 44.0];

// 功能对话框（新建工作树）的尺寸（逻辑像素）：标题行、边距、按钮高度沿用设置浮层的值，
// 下面几项是对话框自己的表单：左边是名称，右边是框。
/// 对话框的宽度：目录、分支名都放得下，又明显窄于设置浮层，与只做一件事的用途相称。
pub const DIALOG_W: f32 = 460.0;
/// 输入框、分支菜单、位置的高度，与提交信息框、侧栏搜索框相同
/// （一行文字 20，上下各留 2）。
pub const DIALOG_FIELD_H: f32 = 24.0;
/// 左边名称列的宽度：名称都是两个字（分支、基于、位置），右对齐。
pub const DIALOG_LABEL_W: f32 = 40.0;
/// 名称列和框之间的距离。
pub const DIALOG_LABEL_GAP: f32 = 12.0;
/// 框内文字到框左右边缘的距离，与提交信息框相同。
pub const DIALOG_FIELD_INSET: f32 = 10.0;
/// 上下两个框之间的距离。
pub const DIALOG_GAP: f32 = 10.0;
/// 底部按钮（「取消」「创建」）的宽度。
pub const DIALOG_BUTTON_W: f32 = 72.0;
/// 「位置」右端选目录按钮（一个目录图标）的宽度。
pub const DIALOG_CHANGE_W: f32 = 28.0;

/// 「快捷键」页顶部搜索框的高度（逻辑像素），与分类栏顶部的搜索框相同。
pub const KEYS_SEARCH_H: f32 = SETTINGS_CONTROL_H;
/// 搜索框右端「录制按键」按钮的边长（逻辑像素）：上下各离搜索框边缘 3。
pub const KEYS_RECORD: f32 = KEYS_SEARCH_H - 6.0;
/// 「快捷键」页列表一行的高度（逻辑像素）。
pub const KEYS_ROW: f32 = 34.0;

/// 快速打开面板的宽度（逻辑像素）；列表中要放下「路径 + 文件名」，比查找条宽。
pub const PICKER_WIDTH: f32 = 560.0;

/// 红绿灯第一个按钮距窗口左边缘的距离（逻辑像素）：卡片左侧的留白加 14，按钮到卡片左边缘的
/// 距离与到顶栏上下边缘的距离相近。
pub const TRAFFIC_LIGHT_LEFT: f32 = ISLAND_MARGIN + 14.0;
/// 红绿灯右端（最后一个按钮的右边缘）距窗口左边缘的距离（逻辑像素）的默认值：macOS 26 的
/// 按钮边长 14、间距 23，即 `TRAFFIC_LIGHT_LEFT + 23 × 2 + 14`。
///
/// 窗口创建后按系统实际摆放的
/// 位置更新（[`crate::layout::set_traffic_lights_end`]），macOS 15 及更早的间距是 20。非 macOS
/// 没有红绿灯，为 0。
pub const TRAFFIC_LIGHTS_END: f32 = if cfg!(target_os = "macos") {
    TRAFFIC_LIGHT_LEFT + 23.0 * 2.0 + 14.0
} else {
    0.0
};
/// 红绿灯与右侧第一个图标之间的距离（逻辑像素，按两者可见的边缘计算）。
///
/// 与顶栏上并排的两个图标之间的距离相同：14 宽的图标在 `BUTTON` 宽的按钮里两侧各留 7，
/// 加上按钮间的 `BUTTON_GAP`，共 16。
pub const LIGHTS_GAP: f32 = (BUTTON - 14.0) + BUTTON_GAP;

#[cfg(test)]
mod tests;
