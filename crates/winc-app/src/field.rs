//! 输入框：文字、光标、选区、撤销和输入历史。
//!
//! 查找、替换、跳转、快速打开、全局搜索和设置中的单行输入都使用 [`TextField`]。
//! 提交信息这类多行输入由 [`TextField::multiline`] 创建，文字中可以包含换行符，
//! 绘制时按可用宽度折行。文字中的位置（光标、选区两端）均为 UTF-8 字节偏移，并且总落在字符边界上；
//! 光标按字素（grapheme）移动。接受位置的接口先把 `pos` 限制在文字长度以内，再退到最近的有效边界，
//! 越界时不会 panic。

use unicode_segmentation::UnicodeSegmentation;
use winc_render::Rect;

/// 撤销栈最多保留的步数（步）。
///
/// 超过该步数时丢弃最旧的一步，最近的一步总是保留。该值与 [`UNDO_BYTES`] 共同约束撤销栈的
/// 内存占用，取值按一次编辑会话中需要回退的修改次数取自调参：连续键入超过 100 个字素后，
/// 更早的步骤被回退的情况很少出现。
const UNDO_LIMIT: usize = 100;
/// 撤销栈中各步文字合计允许占用的最大字节数（1 MiB）。
///
/// 超过该字节数时丢弃最旧的步，最近的一步总是保留。每一步保存的是修改前的整段文字，撤销栈的
/// 占用因此是文字长度的 [`UNDO_LIMIT`] 倍，需要单独的字节上限；该上限由 [`UNDO_LIMIT`] 与
/// 单次粘贴的文字长度共同约束，取值取自调参：文字为百字节量级时，撤销栈的占用不影响帧时间。
const UNDO_BYTES: usize = 1 << 20;
/// 输入历史最多保留的条数（条）。
///
/// 超过该条数时丢弃最旧的一条。该值取自调参：查找、替换等单行输入中，连续翻阅超过 50 条的
/// 情况很少出现。
const HISTORY_LIMIT: usize = 50;

/// 光标移动和删除的单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// 一个字素。
    Char,
    /// 一个词：连续的同类字素算一个词。
    Word,
    /// 到行首或行尾；单行输入框中即到文字开头或末尾，多行输入框中到当前行（按换行符划分）的两端。
    Line,
}

/// 输入框的状态：文字、光标、选区、撤销栈和输入历史。
#[derive(Debug, Clone)]
pub struct TextField {
    text: String,
    /// 光标位置（字节偏移）。
    caret: usize,
    /// 选区的另一端（字节偏移）；与 `caret` 相等时没有选区。
    anchor: usize,
    /// 允许输入的字符的过滤函数；键入、粘贴和输入法提交的文字都经过该函数过滤。
    accept: fn(char) -> bool,
    /// 是否允许多行（文字中可以包含 `\n`）。
    multiline: bool,
    /// 是否遮盖文字：绘制为圆点，不允许复制和剪切。
    masked: bool,
    /// 撤销栈，最近的一步在末尾。
    undo: Vec<Snapshot>,
    /// 重做栈，最近撤销的一步在末尾。
    redo: Vec<Snapshot>,
    /// 上一次修改是否为键入单个字素；为 `true` 时，下一次键入的字符合并进同一步撤销。
    typing: bool,
    history: History,
    /// 上一次绘制时记录的几何，供鼠标点击和拖选使用。
    pub view: FieldView,
}

/// 输入历史，最旧的在前，不重复。
#[derive(Debug, Clone, Default)]
struct History {
    /// 历史条目。
    entries: Vec<String>,
    /// 正在查看的条目下标（从 0 起）；没有翻阅历史时为 `None`。
    cursor: Option<usize>,
}

impl History {
    /// 记录一条历史：先移除相同条目，再追加为最新一条，并结束翻阅历史的状态。
    ///
    /// 空文字不记录。
    fn record(&mut self, text: &str) {
        self.cursor = None;

        if text.is_empty() {
            return;
        }

        self.entries.retain(|e| e != text);

        if self.entries.len() >= HISTORY_LIMIT {
            self.entries.remove(0);
        }

        self.entries.push(text.to_owned());
    }

    /// 当前文字不在历史中时先记录，以便翻阅时能找到该条目。
    fn keep(&mut self, current: &str) {
        if !self.entries.iter().any(|e| e == current) {
            self.record(current);
        }
    }

    /// 返回前一条（更旧的）历史，跳过与当前文字相同的条目；已经是最旧的一条时返回 `None`。
    fn previous(&mut self, current: &str) -> Option<String> {
        self.keep(current);
        let mut i = self.cursor.unwrap_or(self.entries.len()).checked_sub(1)?;

        if self.entries[i] == current {
            i = i.checked_sub(1)?;
        }

        self.cursor = Some(i);
        Some(self.entries[i].clone())
    }

    /// 返回后一条（更新的）历史。
    ///
    /// 超过最新的一条（或没有在翻阅历史）时返回空文字，即清空输入。
    fn next(&mut self, current: &str) -> String {
        self.keep(current);
        let mut i = self.cursor.map_or(self.entries.len(), |i| i + 1);

        if self.entries.get(i).is_some_and(|e| e == current) {
            i += 1;
        }

        self.cursor = (i < self.entries.len()).then_some(i);
        self.entries.get(i).cloned().unwrap_or_default()
    }
}

/// 撤销栈中的一个快照：某一时刻的文字、光标和选区另一端。
#[derive(Debug, Clone)]
struct Snapshot {
    text: String,
    caret: usize,
    anchor: usize,
}

/// 绘制时记录的几何，供鼠标点击和拖选使用。
#[derive(Debug, Clone, Default)]
pub struct FieldView {
    /// 输入框的绘制区域。
    pub area: Rect,
    /// 文字起点的 x 坐标（逻辑像素），已减去横向滚动量。
    pub origin: f32,
    /// 横向滚动量（逻辑像素）；文字比输入框宽时，滚动使光标始终可见。
    pub scroll: f32,
    /// 每个字素边界的（字节偏移，相对文字起点的 x 坐标），从 0 到文字末尾。
    pub stops: Vec<(usize, f32)>,
    /// 多行输入框：每个边界在第几个显示行（与 `stops` 一一对应）；单行输入框为空。
    pub rows: Vec<u32>,
    /// 多行输入框：第 0 个显示行的顶边（已减去纵向滚动量）。
    pub top: f32,
    /// 多行输入框：行高。
    pub line_height: f32,
    /// 单行输入框的测量缓存：`stops` 是按哪段文字、多大的字号量出来的。
    ///
    /// 两者都没变时下一帧直接用 `stops`，不再逐个字素测量；多行输入框不使用，值为 `None`。
    pub measured: Option<(String, f32)>,
    /// 多行输入框的滚动跟随状态：上次绘制时的（光标位置，显示文字的长度）。
    ///
    /// 两者都没变时纵向滚动量保持不变，用户可以用滚轮把光标滚出可见区域；任何一个变了，
    /// 滚动量回到光标所在的行。
    pub follow: Option<(usize, usize)>,
    /// 单行输入框被滚轮滚动时的（光标位置，文字的长度）。
    ///
    /// 两者都没变时横向滚动量不跟随光标，光标可以滚出可见区域；任何一个变了，回到跟随光标。
    pub wheeled: Option<(usize, usize)>,
}

impl FieldView {
    /// 返回输入框内横坐标 `x`（逻辑像素）处最近的字素边界的字节偏移。
    pub fn position_at(&self, x: f32) -> usize {
        let x = x - self.origin;
        self.stops
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |&(pos, _)| pos)
    }

    /// 返回距输入框内点 `(x, y)`（逻辑像素坐标）最近的字素边界的字节偏移。
    ///
    /// 多行输入框先按 `y` 确定显示行，再在该行内按 `x` 查找；单行输入框只按 `x` 查找。
    pub fn position_at_point(&self, x: f32, y: f32) -> usize {
        if self.rows.is_empty() || self.line_height <= 0.0 {
            return self.position_at(x);
        }
        let last = self.rows.last().copied().unwrap_or(0);
        let row = (((y - self.top) / self.line_height).floor().max(0.0) as u32).min(last);
        self.nearest_in_row(row, x - self.origin).unwrap_or(0)
    }

    /// 返回多行输入框中第 `row` 个显示行内距 `x`（相对文字起点，逻辑像素）最近的边界。
    fn nearest_in_row(&self, row: u32, x: f32) -> Option<usize> {
        self.stops
            .iter()
            .zip(&self.rows)
            .filter(|(_, r)| **r == row)
            .min_by(|a, b| (a.0.1 - x).abs().total_cmp(&(b.0.1 - x).abs()))
            .map(|(&(pos, _), _)| pos)
    }

    /// 返回多行输入框中 `pos` 所在的显示行号和该处的 x 坐标（逻辑像素，相对文字起点）。
    ///
    /// `pos` 不在字素边界上时取其后第一个边界。
    pub fn row_and_x(&self, pos: usize) -> Option<(u32, f32)> {
        let i = self.stops.iter().position(|&(p, _)| p >= pos)?;
        Some((*self.rows.get(i)?, self.stops[i].1))
    }

    /// 返回字节偏移 `pos` 相对文字起点的 x 坐标（逻辑像素）。
    ///
    /// `pos` 没有恰好对应的字素边界时取其后第一个边界。
    pub fn x_of(&self, pos: usize) -> f32 {
        let i = self.stops.partition_point(|&(p, _)| p < pos);
        self.stops.get(i).or(self.stops.last()).map_or(0.0, |s| s.1)
    }
}

impl Default for TextField {
    fn default() -> Self {
        Self::with_filter(|_| true)
    }
}

/// 字素的类别；按词移动时，连续的同类字素算一个词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Punct,
}

fn class_of(grapheme: &str) -> Class {
    match grapheme.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        Some(c) if c.is_whitespace() => Class::Space,
        _ => Class::Punct,
    }
}

impl TextField {
    /// 滚轮横向滚动单行输入框 `delta`（逻辑像素，正数露出后面的文字）。
    ///
    /// 下一帧绘制时限制在文字范围内。
    pub fn wheel_scroll(&mut self, delta: f32) {
        self.view.scroll += delta;
        self.view.wheeled = Some((self.caret(), self.text().len()));
    }

    /// 创建只接受 `accept` 返回 `true` 的字符的输入框；控制字符和换行符始终不接受。
    pub fn with_filter(accept: fn(char) -> bool) -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: 0,
            accept,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            history: History::default(),
            view: FieldView::default(),
            multiline: false,
            masked: false,
        }
    }

    /// 创建遮盖文字的输入框，用于密码和口令。
    pub fn masked() -> Self {
        Self {
            masked: true,
            ..Self::default()
        }
    }

    /// 是否遮盖文字。
    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// 创建多行输入框：文字中可以换行，粘贴的多行文字按原样保留（换行符统一为 `\n`）。
    pub fn multiline() -> Self {
        Self {
            multiline: true,
            ..Self::default()
        }
    }

    /// 是否是多行输入框（[`TextField::multiline`]）。
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// 在多行输入框中把光标上移或下移一个显示行。
    ///
    /// `extend` 为 `true` 时扩展选区。光标已经在第一行（或最后一行）时位置不变，返回 `false`。
    /// 折行位置按上一次绘制时记录的几何计算。
    pub fn move_vertical(&mut self, down: bool, extend: bool) -> bool {
        let Some((row, x)) = self.view.row_and_x(self.caret) else {
            return false;
        };
        let target = if down {
            row + 1
        } else {
            match row.checked_sub(1) {
                Some(r) => r,
                None => return false,
            }
        };
        let Some(pos) = self.view.nearest_in_row(target, x) else {
            return false;
        };
        self.set_caret(pos, extend);
        true
    }

    /// 返回输入框的文字。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 返回光标位置（字节偏移）。
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// 返回选区的（起点，终点）字节偏移，起点在前。
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// 返回选区的文字；没有选区时返回空串。
    pub fn selected_text(&self) -> &str {
        let (from, to) = self.selection();
        &self.text[from..to]
    }

    /// 替换整个文字并全选，此后键入的文字替换选区；该操作可以撤销。
    ///
    /// 文字与现有的相同时只全选，不记撤销步骤，也不清掉重做记录。
    pub fn set_text(&mut self, text: &str) {
        self.select_all();
        if text == self.text {
            return;
        }
        self.replace_selection(text, false);
        self.anchor = 0;
    }

    /// 选中全部文字。
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.typing = false;
    }

    /// 选中 `pos` 所在的词（或一段连续的空白、标点）。
    pub fn select_word_at(&mut self, pos: usize) {
        let graphemes: Vec<_> = self.text.grapheme_indices(true).collect();
        // `pos` 在文字末尾时选中最后一个词。
        let Some(at) = graphemes
            .iter()
            .position(|&(start, g)| pos < start + g.len())
            .or(graphemes.len().checked_sub(1))
        else {
            return;
        };
        let class = class_of(graphemes[at].1);
        let same = |&&(_, g): &&(usize, &str)| class_of(g) == class;
        let first = at - graphemes[..at].iter().rev().take_while(same).count();
        let last = at + graphemes[at + 1..].iter().take_while(same).count();

        self.anchor = graphemes[first].0;
        self.caret = graphemes[last].0 + graphemes[last].1.len();
        self.typing = false;
    }

    /// 把光标移到 `pos`（字节偏移，来自鼠标点击）；`extend` 为 `true` 时保留选区的另一端。
    ///
    /// `pos` 来自上一次绘制时记录的字素边界。文字可能在该次绘制之后已经变化，因为键入和鼠标
    /// 事件在同一批事件中处理，期间没有重绘；`pos` 落在字符中间时退到该字符的开头，保证后续
    /// 按字节切分文字时不会 panic。
    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        let mut pos = pos.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        self.caret = pos;

        if !extend {
            self.anchor = self.caret;
        }

        self.typing = false;
    }

    /// 按 `unit` 移动光标：`forward` 为 `true` 时朝文字末尾方向，否则朝开头方向。
    ///
    /// `extend` 为 `true` 时扩展选区。存在选区且不扩展选区时，按字符移动先把光标放到选区的
    /// 相应一端。
    pub fn move_caret(&mut self, unit: Unit, forward: bool, extend: bool) {
        let (from, to) = self.selection();

        let target = if !extend && from != to && unit == Unit::Char {
            if forward { to } else { from }
        } else {
            self.boundary(self.caret, unit, forward)
        };

        self.set_caret(target, extend);
    }

    /// 键入或粘贴 `text`：按 `accept` 过滤后替换选区。
    ///
    /// 单行输入框只取第一个非空行；多行输入框保留换行符，`\r\n` 和单独的 `\r` 都转换为 `\n`。
    ///
    /// 返回文字是否发生变化。
    pub fn insert(&mut self, text: &str) -> bool {
        let accept = self.accept;
        let filtered: String = if self.multiline {
            text.replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .filter(|&c| c == '\n' || (!c.is_control() && accept(c)))
                .collect()
        } else {
            // 含换行时取第一个非空行；不含换行时原样使用，键入的空格本身就是内容。
            let line = if text.contains(['\n', '\r']) {
                let mut lines = text.split(['\n', '\r']);
                lines.find(|l| !l.trim().is_empty()).unwrap_or_default()
            } else {
                text
            };
            line.chars()
                .filter(|&c| !c.is_control() && accept(c))
                .collect()
        };

        if filtered.is_empty() {
            return false;
        }

        let typing = self.caret == self.anchor && filtered.graphemes(true).count() == 1;
        self.replace_selection(&filtered, typing)
    }

    /// 删除选区；没有选区时按 `unit` 删除一个单位（`forward` 为 `true` 时朝文字末尾方向，
    /// 否则朝开头方向）。
    ///
    /// 返回文字是否发生变化。
    pub fn delete(&mut self, unit: Unit, forward: bool) -> bool {
        if self.caret == self.anchor {
            self.anchor = self.boundary(self.caret, unit, forward);
        }

        self.replace_selection("", false)
    }

    /// 把当前文字记入输入历史；查找、替换和关闭查找条时调用。
    pub fn remember(&mut self) {
        self.history.record(&self.text);
    }

    /// 是否正在翻阅输入历史（上一次修改文字来自 [`TextField::recall`]）。
    pub fn recalling(&self) -> bool {
        self.history.cursor.is_some()
    }

    /// 翻阅输入历史：`back` 为 `true` 时取更旧的一条，否则取更新的一条。
    ///
    /// 超过最新的一条时清空文字，光标放在末尾。该操作可以撤销。返回文字是否发生变化。
    pub fn recall(&mut self, back: bool) -> bool {
        let text = if back {
            self.history.previous(&self.text)
        } else {
            Some(self.history.next(&self.text))
        };

        match text {
            Some(text) if text != self.text => {
                // `set_text` 会清除翻阅历史的下标，因此在调用前保存，调用后恢复。
                let cursor = self.history.cursor;
                self.set_text(&text);
                self.history.cursor = cursor;
                self.set_caret(self.text.len(), false);
                true
            }
            _ => false,
        }
    }

    /// 撤销一步；没有可撤销的内容时返回 `false`。
    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    /// 重做一步；没有可重做的内容时返回 `false`。
    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    /// 撤销或重做一步：从 `from` 栈取出快照，把当前状态压入 `to` 栈。
    fn step(&mut self, undo: bool) -> bool {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };

        let Some(snapshot) = from.pop() else {
            return false;
        };

        to.push(Snapshot {
            text: std::mem::replace(&mut self.text, snapshot.text),
            caret: self.caret,
            anchor: self.anchor,
        });
        self.caret = snapshot.caret;
        self.anchor = snapshot.anchor;
        self.typing = false;
        true
    }

    /// 用 `text` 替换选区，光标停在新文字之后。
    ///
    /// `typing` 表示本次修改是键入单个字素；连续键入的内容合并为一步撤销。
    fn replace_selection(&mut self, text: &str, typing: bool) -> bool {
        let (from, to) = self.selection();

        if from == to && text.is_empty() {
            return false;
        }

        if !(typing && self.typing) {
            self.undo.push(Snapshot {
                text: self.text.clone(),
                caret: self.caret,
                anchor: self.anchor,
            });
            let mut bytes: usize = self.undo.iter().map(|s| s.text.len()).sum();
            while self.undo.len() > 1 && (self.undo.len() > UNDO_LIMIT || bytes > UNDO_BYTES) {
                bytes -= self.undo.remove(0).text.len();
            }
        }

        self.redo.clear();
        // 文字由该函数修改后，输入框不再处于翻阅历史的状态。
        self.history.cursor = None;
        self.text.replace_range(from..to, text);
        self.caret = from + text.len();
        self.anchor = self.caret;
        self.typing = typing;
        true
    }

    /// 返回从 `pos` 起按 `unit` 移动一个单位后的字节偏移：`forward` 为 `true` 时朝文字末尾方向，
    /// 否则朝开头方向。
    ///
    /// `pos` 总在字符边界上（由 [`Self::set_caret`] 保证），按字节切片不会 panic。
    fn boundary(&self, pos: usize, unit: Unit, forward: bool) -> usize {
        let graphemes = self.text.grapheme_indices(true);

        match (unit, forward) {
            (Unit::Line, true) if self.multiline => self.text[pos..]
                .find('\n')
                .map_or(self.text.len(), |i| pos + i),
            (Unit::Line, false) if self.multiline => {
                self.text[..pos].rfind('\n').map_or(0, |i| i + 1)
            }
            (Unit::Line, true) => self.text.len(),
            (Unit::Line, false) => 0,
            (Unit::Char, true) => graphemes
                .map(|(start, g)| start + g.len())
                .find(|&end| end > pos)
                .unwrap_or(self.text.len()),
            (Unit::Char, false) => graphemes
                .map(|(start, _)| start)
                .take_while(|&start| start < pos)
                .last()
                .unwrap_or(0),
            // 按词移动：先跳过空白类字素，再取同一类字素的末尾。
            (Unit::Word, true) => {
                let mut rest = graphemes
                    .filter(|&(start, _)| start >= pos)
                    .skip_while(|&(_, g)| class_of(g) == Class::Space)
                    .peekable();
                let Some(&(_, first)) = rest.peek() else {
                    return self.text.len();
                };
                let class = class_of(first);
                rest.take_while(|&(_, g)| class_of(g) == class)
                    .last()
                    .map_or(self.text.len(), |(start, g)| start + g.len())
            }
            (Unit::Word, false) => {
                let before: Vec<_> = graphemes.take_while(|&(start, _)| start < pos).collect();
                let mut rest = before
                    .iter()
                    .rev()
                    .skip_while(|&&(_, g)| class_of(g) == Class::Space)
                    .peekable();
                let Some(&&(_, first)) = rest.peek() else {
                    return 0;
                };
                let class = class_of(first);
                rest.take_while(|&&(_, g)| class_of(g) == class)
                    .last()
                    .map_or(0, |&(start, _)| start)
            }
        }
    }
}

#[cfg(test)]
mod tests;
