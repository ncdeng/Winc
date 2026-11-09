//! 浮在编辑区上的几样东西的几何：查找条、跳转条、快速打开面板。

use super::*;
use winc_i18n::t;

/// 查找条中的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchButton {
    /// 展开或收起替换。
    Expand,
    /// 区分大小写。
    Case,
    /// 全字匹配。
    Word,
    /// 使用正则表达式。
    Regex,
    /// 跳到上一个匹配。
    Prev,
    /// 跳到下一个匹配。
    Next,
    /// 只显示有匹配的行。
    Filter,
    /// 关闭查找条。
    Close,
    /// 替换当前匹配。
    ReplaceOne,
    /// 替换全部匹配。
    ReplaceAll,
}

/// 悬停提示里名字和快捷键之间的分隔符：提示画成「名字 … 快捷键」两栏，快捷键是右边的淡色字，
/// 不加括号。
pub const TIP_KEY: char = '\t';

/// 返回悬停提示文字：名字，后面隔着 [`TIP_KEY`] 是快捷键（取自键位表）；没有快捷键时只返回
/// 名字。
pub fn with_key(name: &str, action: Option<Action>) -> String {
    match action.and_then(keymap::shortcut) {
        Some(key) => format!("{name}{TIP_KEY}{key}"),
        None => name.to_owned(),
    }
}

impl SearchButton {
    /// 返回悬停提示文字（含快捷键），查找条和全局搜索面板共用。
    ///
    /// 「上一个」「下一个」显示输入框中的 `⇧↩` 和 `↩`，而不是 `⌘G`：焦点在查找输入框时可
    /// 直接使用这两个键，用户更容易发现。
    pub fn label(self) -> String {
        let enter = |shift: bool| {
            let mods = if shift { Mods::SHIFT } else { Mods::NONE };
            Stroke {
                key: KeyCode::Enter,
                mods,
            }
            .label(Os::CURRENT)
        };
        match self {
            SearchButton::Expand => t!("workspace-find-toggle-replace").to_owned(),
            SearchButton::Case => {
                with_key(t!("workspace-find-match-case"), Some(Action::ToggleCase))
            }
            SearchButton::Word => {
                with_key(t!("workspace-find-whole-word"), Some(Action::ToggleWord))
            }
            SearchButton::Regex => {
                with_key(t!("workspace-find-use-regex"), Some(Action::ToggleRegex))
            }
            SearchButton::Prev => {
                format!("{}{TIP_KEY}{}", t!("workspace-find-previous"), enter(true))
            }
            SearchButton::Next => format!("{}{TIP_KEY}{}", t!("workspace-find-next"), enter(false)),
            SearchButton::Filter => {
                with_key(t!("workspace-find-filter-lines"), Some(Action::FilterLines))
            }
            SearchButton::Close => format!(
                "{}{TIP_KEY}{}",
                t!("workspace-find-close"),
                Stroke {
                    key: KeyCode::Escape,
                    mods: Mods::NONE
                }
                .label(Os::CURRENT)
            ),
            SearchButton::ReplaceOne => {
                with_key(t!("workspace-find-replace"), Some(Action::ReplaceOne))
            }
            SearchButton::ReplaceAll => {
                with_key(t!("workspace-find-replace-all"), Some(Action::ReplaceAll))
            }
        }
    }
}

/// 分支、位置选择器顶部一行的高度（逻辑像素）：`BUTTON_SMALL` 高的搜索框，上面留 6、下面留 4。
pub const MENU_PICKER_BAR: f32 = 34.0;
/// 分支、位置选择器列表下方的留白（逻辑像素）：和搜索框下面留的一样。
pub const MENU_PICKER_PAD: f32 = 4.0;

/// 快速打开面板（⌘P）的几何：上方是输入框所在的一行，下方是结果列表。
///
/// 绘制、点击和滚动都用这份几何，保证绘制位置与点击位置一致。列表上下各留 [`PICKER_PAD`] 的留白，
/// 滚动的行裁剪在留白以内。
#[derive(Debug, Clone, Copy)]
pub struct PickerParts {
    /// 输入框所在的一行；位置浮窗中是标题行。
    pub bar: Rect,
    /// 分支选择器输入框下面拉取、推送、获取的一行；其他面板没有，高度为 0。
    pub actions: Rect,
    /// 结果列表区域，不含上下留白。
    pub list: Rect,
    /// 位置浮窗底部显示完整路径的一栏；快速打开面板没有，高度为 0。
    pub footer: Rect,
}

impl PickerParts {
    /// 根据面板卡片的矩形 `card` 计算各部件的位置。
    pub fn new(card: Rect) -> PickerParts {
        Self::with_footer(card, 0.0)
    }

    /// 根据位置浮窗（定义、用法、书签）的矩形 `card` 计算各部件的位置：比快速打开面板多一条
    /// 高 [`PLACES_FOOTER`] 的底栏。
    pub fn places(card: Rect) -> PickerParts {
        Self::with_footer(card, PLACES_FOOTER)
    }

    /// 根据分支、位置选择器的矩形 `card` 计算各部件的位置：顶部是内嵌搜索框的一行，下面是高
    /// `actions` 的一行操作（没有时为 0），列表紧接在下面，再往下是高 `footer` 的底栏。
    pub fn menu(card: Rect, actions: f32, footer: f32) -> PickerParts {
        let bar = MENU_PICKER_BAR.min(card.h);
        let actions = actions.min((card.h - bar).max(0.0));
        let top = bar + actions;
        let footer = footer.min((card.h - top).max(0.0));
        let list = (card.h - top - MENU_PICKER_PAD - footer).max(0.0);
        PickerParts {
            bar: Rect::new(card.x, card.y, card.w, bar),
            actions: Rect::new(card.x, card.y + bar, card.w, actions),
            list: Rect::new(card.x, card.y + top, card.w, list),
            footer: Rect::new(card.x, card.bottom() - footer, card.w, footer),
        }
    }

    fn with_footer(card: Rect, footer: f32) -> PickerParts {
        let footer = footer.min((card.h - BAR_HEIGHT).max(0.0));
        PickerParts {
            bar: Rect::new(card.x, card.y, card.w, BAR_HEIGHT.min(card.h)),
            actions: Rect::new(card.x, card.y + BAR_HEIGHT.min(card.h), card.w, 0.0),
            list: Rect::new(
                card.x,
                card.y + BAR_HEIGHT + PICKER_PAD,
                card.w,
                (card.h - BAR_HEIGHT - 2.0 * PICKER_PAD - footer).max(0.0),
            ),
            footer: Rect::new(card.x, card.bottom() - footer, card.w, footer),
        }
    }

    /// 返回第 `index` 条结果（从 0 起）所在行的矩形，`scroll` 是列表的滚动量（像素）。
    ///
    /// 矩形可能只有一部分在列表内，或完全在列表之外；绘制时需要裁剪到 `list`。
    pub fn row(&self, index: usize, scroll: f32) -> Rect {
        let y = self.list.y + index as f32 * PICKER_ROW - scroll;
        Rect::new(self.list.x, y, self.list.w, PICKER_ROW)
    }

    /// 返回列表中纵坐标 `y` 处的结果序号（从 0 起），不检查实际是否有这条结果；`y` 不在列表内
    /// 时返回 `None`。
    pub fn row_at(&self, y: f32, scroll: f32) -> Option<usize> {
        let list = self.list;
        (y >= list.y && y < list.bottom())
            .then(|| ((y - list.y + scroll) / PICKER_ROW).floor() as usize)
    }

    /// 返回滚动量为 `scroll`、共 `count` 条结果时，列表中可见（包括只露出一部分）的结果序号范围，
    /// 不含末尾。
    pub fn visible(&self, scroll: f32, count: usize) -> std::ops::Range<usize> {
        let first = ((scroll / PICKER_ROW).floor().max(0.0) as usize).min(count);
        let last = (((scroll + self.list.h) / PICKER_ROW).ceil() as usize).min(count);
        first..last.max(first)
    }
}

/// 查找条的宽度（逻辑像素）。
///
/// 按 `SearchParts::new` 的排布，三个选项开关和四个按钮占用后，查找文字区域（含右端的匹配计数）
/// 宽 243 像素。
pub const SEARCH_WIDTH: f32 = 488.0;

/// 查找条中各部件的位置，绘制和点击判断共用。
///
/// 第一行依次是：展开按钮、查找文字（右端是匹配计数）、三个选项开关、竖线、上一个、下一个、
/// 「只显示匹配的行」、关闭。
///
/// 展开时有第二行：替换文字、「替换」「全部替换」两个按钮。收起时第二行的部件尺寸为 0。
#[derive(Debug, Clone, Copy)]
pub struct SearchParts {
    /// 展开或收起替换的按钮。
    pub expand: Rect,
    /// 查找文字和匹配计数所在的区域。
    pub field: Rect,
    /// 「区分大小写」开关。
    pub case_toggle: Rect,
    /// 「全字匹配」开关。
    pub word_toggle: Rect,
    /// 「使用正则表达式」开关。
    pub regex_toggle: Rect,
    /// 选项开关和翻页按钮之间的竖线。
    pub divider: Rect,
    /// 「上一个匹配」按钮。
    pub prev: Rect,
    /// 「下一个匹配」按钮。
    pub next: Rect,
    /// 「只显示匹配的行」按钮。
    pub filter: Rect,
    /// 关闭按钮。
    pub close: Rect,
    /// 替换文字所在的区域（第二行）。
    pub replace_field: Rect,
    /// 「替换」按钮（第二行）。
    pub replace_one: Rect,
    /// 「全部替换」按钮（第二行）。
    pub replace_all: Rect,
}

impl SearchParts {
    /// 根据查找条的矩形 `bar` 计算各部件的位置；`bar` 高度超过一行时才有第二行的部件。
    pub fn new(bar: Rect) -> SearchParts {
        const B: f32 = BUTTON_SMALL;

        let y = bar.y + (BAR_HEIGHT - B) / 2.0;
        let expand = Rect::new(bar.x + 6.0, y, B, B);
        let close = Rect::new(bar.right() - 6.0 - B, y, B, B);
        let filter = Rect::new(close.x - BUTTON_GAP - B, y, B, B);
        let next = Rect::new(filter.x - BUTTON_GAP - B, y, B, B);
        let prev = Rect::new(next.x - BUTTON_GAP - B, y, B, B);
        let divider = Rect::new(prev.x - 6.0, bar.y + 10.0, 1.0, BAR_HEIGHT - 20.0);
        let regex_toggle = Rect::new(divider.x - 5.0 - TOGGLE_W, y, TOGGLE_W, B);
        let word_toggle = Rect::new(regex_toggle.x - BUTTON_GAP - TOGGLE_W, y, TOGGLE_W, B);
        let case_toggle = Rect::new(word_toggle.x - BUTTON_GAP - TOGGLE_W, y, TOGGLE_W, B);
        let field_x = expand.right() + 6.0;
        let field = Rect::new(
            field_x,
            bar.y,
            (case_toggle.x - 8.0 - field_x).max(0.0),
            BAR_HEIGHT,
        );

        // 第二行的两个图标按钮在「只显示匹配的行」「关闭」正下方。
        let (replace_field, replace_one, replace_all) = if bar.h > BAR_HEIGHT + 1.0 {
            let y = bar.y + BAR_HEIGHT + (BAR_HEIGHT - B) / 2.0;
            let all = Rect::new(close.x, y, B, B);
            let one = Rect::new(filter.x, y, B, B);
            let y = bar.y + BAR_HEIGHT;
            let field = Rect::new(field_x, y, (one.x - 8.0 - field_x).max(0.0), BAR_HEIGHT);
            (field, one, all)
        } else {
            Default::default()
        };

        SearchParts {
            expand,
            field,
            case_toggle,
            word_toggle,
            regex_toggle,
            divider,
            prev,
            next,
            filter,
            close,
            replace_field,
            replace_one,
            replace_all,
        }
    }

    /// 返回 `button` 的位置；收起替换时第二行按钮的尺寸为 0。
    pub fn rect(&self, button: SearchButton) -> Rect {
        match button {
            SearchButton::Expand => self.expand,
            SearchButton::Case => self.case_toggle,
            SearchButton::Word => self.word_toggle,
            SearchButton::Regex => self.regex_toggle,
            SearchButton::Prev => self.prev,
            SearchButton::Next => self.next,
            SearchButton::Filter => self.filter,
            SearchButton::Close => self.close,
            SearchButton::ReplaceOne => self.replace_one,
            SearchButton::ReplaceAll => self.replace_all,
        }
    }

    /// 返回 `(x, y)` 处的按钮；没有按钮时返回 `None`。
    pub fn button_at(&self, x: f32, y: f32) -> Option<SearchButton> {
        [
            (self.expand, SearchButton::Expand),
            (self.case_toggle, SearchButton::Case),
            (self.word_toggle, SearchButton::Word),
            (self.regex_toggle, SearchButton::Regex),
            (self.prev, SearchButton::Prev),
            (self.next, SearchButton::Next),
            (self.filter, SearchButton::Filter),
            (self.close, SearchButton::Close),
            (self.replace_one, SearchButton::ReplaceOne),
            (self.replace_all, SearchButton::ReplaceAll),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(x, y))
        .map(|(_, button)| button)
    }
}

/// PDF 批注输入条最多显示的行数：文字更多时在输入条里纵向滚动。
pub const NOTE_LINES: u32 = 6;

/// 返回 PDF 批注输入条的位置：和跳转条一样位于编辑区顶部居中，但更宽（最多 640 像素），文字按
/// 宽度折行，输入条随行数 `lines` 增高（最多 [`NOTE_LINES`] 行）。长批注不必横向滚动，写过的
/// 字都看得见。绘制和点击判断共用。
pub fn note_bar(editor: Rect, lines: u32) -> Rect {
    let width = 640.0_f32.min(editor.w - 40.0).max(0.0);
    let x = (editor.x + (editor.w - width) / 2.0).round();
    let extra = (lines.clamp(1, NOTE_LINES) - 1) as f32 * AREA_LINE;
    Rect::new(x, editor.y + 8.0, width, BAR_HEIGHT + extra)
}

/// 返回「跳转到行」输入条（⌃G）的位置：位于编辑区顶部居中，绘制和点击判断共用。
///
/// 宽度最多 320 像素，编辑区较窄时随它收窄；顶边距编辑区 8 像素。
pub fn goto_bar(editor: Rect) -> Rect {
    let width = 320.0_f32.min(editor.w - 40.0).max(0.0);
    let x = (editor.x + (editor.w - width) / 2.0).round();
    Rect::new(x, editor.y + 8.0, width, BAR_HEIGHT)
}
