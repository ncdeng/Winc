//! 文档：文本、选区、撤销历史、高亮状态，以及对应的磁盘文件。
//!
//! 主要类型是 [`Document`]。文本存放在 [`Content`] 中：普通文件是 `rope`，大文件按需读取磁盘。
//! 列对齐、折叠、粘滞滚动等派生数据按 `revision` 判断缓存是否失效。本模块还负责读写磁盘文件（编码、
//! 换行风格、BOM）和检查外部修改。

use std::borrow::Cow;
use std::fs;
use std::io::{self, BufReader, Read, Seek, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use ropey::Rope;
use winc_core::edit::{EditKind, Transaction};
use winc_core::selection::merge;
use winc_core::text::{Aligned, CellLayout, ColumnWidths, Columns};
use winc_core::{History, Indent, LineEnding, Lines, Selection, diff, indent, transform};
use winc_i18n::t;
use winc_syntax::{Highlighter, LineState};

use crate::Command;
use crate::diff_view::DiffView;
use crate::editing::cursors::others;
use crate::editing::folding::{Folds, Tracked};
use crate::editing::scopes::{self, Ancestry, FoldRange, Scope};
use crate::navigation::bookmarks::Bookmarks;
use content::Content;
use editorconfig::FileConfig;
use encoding::{self as charset, Encoding, UTF8_BOM, utf16_bom};
use large::LargeText;
use winc_preview::image::Picture;
use winc_preview::table::Table;

pub(crate) mod content;
pub(crate) mod disk;
pub mod editorconfig;
pub mod encoding;
mod indentation;
pub(crate) mod large;
mod preview;
#[cfg(test)]
mod tests;

use disk::load;
pub(crate) use winc_config::files::sync_dir;

/// 文档的标识，在同一个 [`crate::Editor`] 内唯一。
///
/// 界面按（文档、版本）缓存绘制结果，因此标识在进程内不会复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

/// 打开文件失败的原因。
#[derive(Debug)]
pub enum OpenError {
    /// 读取文件时的 I/O 错误，例如文件不存在或没有读权限。
    Io(io::Error),
    /// 内容是二进制文件（开头 `BINARY_SNIFF` 字节内含 NUL），不能作为文本打开。
    ///
    /// 值为文件大小（字节）。
    Binary(u64),
    /// 文件超过 `OPEN_LIMIT`，值为文件大小（字节）。
    TooLarge(u64),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "{e}"),
            OpenError::Binary(_) => f.write_str(t!("editor-open-binary")),
            OpenError::TooLarge(size) => f.write_str(&t!(
                "editor-open-too-large",
                size = size >> 20,
                limit = OPEN_LIMIT >> 30
            )),
        }
    }
}

impl std::error::Error for OpenError {}

/// 编辑区里显示的图：解码或绘制中、已就绪，或失败原因。
#[derive(Debug, Clone)]
pub enum PictureState {
    /// 已交给后台线程解码（图片）或绘制（SVG）。
    Loading,
    /// 已生成可直接上屏的位图。
    Ready(Arc<Picture>),
    /// 生成失败的原因（例如 SVG 语法错误）。
    Failed(String),
}

/// 表格读取的状态，见 [`TableView`]。
#[derive(Debug, Clone)]
pub enum TableState {
    /// 已交给后台线程读取。
    Loading,
    /// 读好的表格。
    Ready(Arc<Table>),
    /// 读取失败的原因（例如文件损坏）。
    Failed(String),
}

/// 编辑区中只读的表格：xlsx 工作簿，或切换到预览、分栏的 csv、tsv。
#[derive(Debug, Clone)]
pub struct TableView {
    /// 读取的状态。
    pub state: TableState,
    /// 显示的表格依据的文档版本号（`revision`）：落后于文档时重新读取表格。
    pub revision: u64,
    /// 显示的是第几张工作表（从 0 起）。
    pub sheet: usize,
    /// 横向、纵向的滚动位置（逻辑像素）。
    pub scroll: (f32, f32),
    /// 选中的单元格：起点（按下处，或按 `⇧` 之前的那一格）和当前格（行号，列号），选区是以
    /// 两者为对角的区域，打开时选中 A1。
    pub selection: ((usize, usize), (usize, usize)),
    /// 当前格卡片（显示没有显示全的文字）的滚动位置（逻辑像素）。
    pub card_scroll: f32,
    /// 当前格的卡片按 Esc 收起了；换到另一格时重新显示。
    pub card_closed: bool,
}

impl TableView {
    /// 待读取的表格，依据文档版本 `revision`。
    fn loading(revision: u64) -> Box<Self> {
        Box::new(Self {
            state: TableState::Loading,
            revision,
            sheet: 0,
            scroll: (0.0, 0.0),
            selection: ((0, 0), (0, 0)),
            card_scroll: 0.0,
            card_closed: false,
        })
    }

    /// 读好的表格；尚未读完或读取失败时返回 `None`。
    pub fn table(&self) -> Option<&Arc<Table>> {
        match &self.state {
            TableState::Ready(table) => Some(table),
            _ => None,
        }
    }
}

/// 能预览的文件的种类，见 `Document::preview_kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// SVG：预览是画出来的图（`Document::picture`）。
    Svg,
    /// Markdown：预览是排好版的文档，由界面解析、排版。
    Markdown,
    /// csv、tsv：预览是表格（`Document::table`）。
    Table,
}

/// 能预览的文件（SVG、Markdown）怎么显示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// 只显示源代码。
    Source,
    /// 左边源代码（可以继续编辑），右边跟着文字实时预览。
    Split,
    /// 只显示预览，文档暂时只读。
    Preview,
}

/// 没有文本内容的标签：图片，或无法作为文本显示的文件。
///
/// 图片在编辑区显示预览；其余文件在编辑区说明原因，并给出可执行的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// 图片：编辑区显示预览图（见 `Document::picture`）。
    Image,
    /// xlsx 工作簿：编辑区显示表格（见 `Document::table`）。
    Workbook,
    /// docx 文字文档：编辑区显示转换成 Markdown 后排版的阅读视图，由界面读取和转换。
    Docx,
    /// PDF（只在能预览 PDF 的平台上，见 `winc_preview::pdf::supported`）：编辑区逐页显示，由界面
    /// 读取和渲染。
    Pdf,
    /// 二进制文件或不支持的文本编码，值为文件大小（字节）。
    Binary(u64),
    /// 文件超过 `OPEN_LIMIT`，值为文件大小（字节）。
    TooLarge(u64),
    /// 无法读取，例如没有权限或文件不存在；内容是显示给用户的原因。
    Unreadable(String),
}

impl From<OpenError> for Unsupported {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Binary(size) => Unsupported::Binary(size),
            OpenError::TooLarge(size) => Unsupported::TooLarge(size),
            OpenError::Io(e) => Unsupported::Unreadable(match e.kind() {
                io::ErrorKind::NotFound => t!("editor-unreadable-not-found").to_owned(),
                io::ErrorKind::PermissionDenied => t!("editor-unreadable-permission").to_owned(),
                _ => t!("editor-unreadable-failed", error = e.to_string()),
            }),
        }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported::Image => f.write_str(t!("editor-unsupported-image")),
            Unsupported::Workbook => f.write_str(t!("editor-unsupported-workbook")),
            Unsupported::Docx => f.write_str(t!("editor-unsupported-docx")),
            Unsupported::Pdf => f.write_str(t!("editor-unsupported-pdf")),
            Unsupported::Binary(size) => {
                f.write_str(&t!("editor-unsupported-binary", size = size_text(*size)))
            }
            Unsupported::TooLarge(size) => f.write_str(&t!(
                "editor-unsupported-too-large",
                size = format!("{:.1}", *size as f64 / (1u64 << 30) as f64),
                limit = OPEN_LIMIT >> 30
            )),
            Unsupported::Unreadable(reason) => {
                f.write_str(&t!("editor-unsupported-unreadable", reason = reason))
            }
        }
    }
}

/// 把文件大小写成「2.4 MB」这样的文字：不到 1 KB 写字节数，再往上按 1000 进位并保留一位小数。
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return t!("editor-size-bytes", count = bytes);
    }
    let mut size = bytes as f64 / 1000.0;
    let mut unit = 0;
    while size >= 999.95 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

/// 检查外部修改后的处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskChange {
    /// 磁盘文件没有变化，或者变化与本事件无关。
    Unchanged,
    /// 文档没有未保存的修改，已经按磁盘内容重新加载。
    Reloaded,
    /// 文件仅在末尾追加了内容（例如日志）：只把追加的部分接到文档末尾，撤销记录、光标、折叠保持
    /// 不变。
    Appended,
    /// 磁盘文件和文档都有修改，保留文档中的内容。
    Conflict,
}

/// 按（版本、Tab 宽度）缓存的列对齐结果；`None` 表示不对齐。
type ColumnsCache = ((u64, usize), Option<Arc<Columns>>);

/// 按（版本、Tab 宽度）缓存的整个文件可折叠的块，共享给每次折叠命令，不必复制。
type FoldCache = ((u64, usize), Rc<[FoldRange]>);

/// 按列对齐 CSV、TSV 的文件大小上限（字节，2 MiB）。
///
/// 打字时只重新测量改动的几行（[`ColumnWidths`]），但删掉某一列最宽的字段、换了文件时要测量
/// 整个文件，耗时随文件大小线性增长。超过上限的文件只着色、不对齐。2 MiB 的文件约需数毫秒。
const ALIGN_MAX: usize = 2 << 20;

/// 把 `old` 改成 `new` 时改动涉及的行：`old` 的 `head..old_end` 行换成了 `new` 的
/// `head..new_end` 行，前后其余的行没变。
///
/// 逐块比较两头相同的字节，不逐行比。
fn changed_lines(old: &ropey::Rope, new: &ropey::Rope) -> (usize, usize, usize) {
    let same = |a: ropey::iter::Chunks<'_>, b: ropey::iter::Chunks<'_>, back: bool| {
        let (mut a, mut b) = (a.map(str::as_bytes), b.map(str::as_bytes));
        let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
        let mut same = 0;
        loop {
            if x.is_empty() {
                match a.next() {
                    Some(c) => x = c,
                    None => return same,
                }
            }
            if y.is_empty() {
                match b.next() {
                    Some(c) => y = c,
                    None => return same,
                }
            }
            let n = x.len().min(y.len());
            let (p, q) = if back {
                (&x[x.len() - n..], &y[y.len() - n..])
            } else {
                (&x[..n], &y[..n])
            };
            if p != q {
                let pairs = p.iter().zip(q);
                return same
                    + if back {
                        pairs.rev().take_while(|(u, v)| u == v).count()
                    } else {
                        pairs.take_while(|(u, v)| u == v).count()
                    };
            }
            same += n;
            (x, y) = if back {
                (&x[..x.len() - n], &y[..y.len() - n])
            } else {
                (&x[n..], &y[n..])
            };
        }
    };
    let prefix = same(old.chunks(), new.chunks(), false);
    let head = old.byte_to_line(prefix).min(new.byte_to_line(prefix));
    fn tail_of(text: &ropey::Rope) -> ropey::iter::Chunks<'_> {
        text.chunks_at_byte(text.len_bytes()).0.reversed()
    }
    let suffix = same(tail_of(old), tail_of(new), true);
    // 整行都落在相同的结尾里的行数。
    let full = |text: &ropey::Rope| {
        let at = text.len_bytes() - suffix.min(text.len_bytes());
        let line = text.byte_to_line(at);
        let first = if text.line_to_byte(line) == at {
            line
        } else {
            line + 1
        };
        text.len_lines() - first.min(text.len_lines())
    };
    let (old_n, new_n) = (old.len_lines(), new.len_lines());
    let tail = full(old).min(full(new)).min(old_n - head).min(new_n - head);
    (head, old_n - tail, new_n - tail)
}

/// 新增改动记下多少次（次，64），供 `Document::unchanged_lines_since` 向前追溯。
///
/// 一次连续输入与随后的滚动查询通常只涉及几十次改动，64 条足以覆盖；记录更多只会增加内存开销。
const EDIT_LOG: usize = 64;

/// 可打开的文件大小上限（字节，4 GiB）。
///
/// 超过该大小的文件不打开。上限取自调参：`rope` 把全文放在内存中并额外维护树的节点，内存占用
/// 随文件大小线性增长，超过该值后占用达到可用内存的量级。该上限同时约束打开文件与全局搜索的扫描，
/// 按需读取的方式见 [`LARGE_FILE`]。
pub(crate) const OPEN_LIMIT: u64 = 4 << 30;
/// 大文件的大小下限（字节，16 MiB）。
///
/// 超过该大小的文件按大文件打开：原文留在磁盘上按需读取，内存里只记录改动。下限取自调参：按大文件
/// 打开后每次滚动都要读磁盘，只有当整个文件读入内存的占用和耗时不可忽略时，这一代价才值得承担。
pub const LARGE_FILE: u64 = 16 << 20;
/// 一次复制的最大字节数（字节，64 MiB）。
///
/// 大文件全选后复制会把整个文件读进内存，因此需要限制。
pub const COPY_LIMIT: u64 = 64 << 20;
/// 按 `#!` 行识别语言时读取首行的最大长度（字符，256）。
///
/// 识别只匹配行首的解释器名与参数，常见的 `#!/usr/bin/env -S python3 -u` 一类行长度为几十个字符，
/// 该上限在此基础上留出余量，同时限制超长首行复制到字符串的开销。超过该长度的部分不参与识别。
const SHEBANG_CHARS: usize = 256;
/// 一次编辑涉及的选区的最大字节数（字节，16 MiB）。
///
/// 删除的内容要保存在撤销记录里，选区过大会使内存占用随之增长。
pub const EDIT_LIMIT: u64 = 16 << 20;
/// 大文件高亮的回看行数（行，256）。
///
/// 高亮状态从可见区域上方 256 行处开始推算，推算的代价因此与文件长度无关；高亮状态的传播不能
/// 跨过这一距离，起始位置以上 256 行内没有未闭合的块注释或字符串时结果才与从头推算相同。
const HIGHLIGHT_LOOKBACK: usize = 256;

/// 粘滞滚动每帧最多向后推算的高亮行数（行，4000）。
///
/// 与编辑区绘制时的每帧预算取同一个值：推算不完时本次结果不缓存，下一帧重新计算。该值取自调参，
/// 上限越大，跳到文件靠后位置时粘滞块能更早算出，单帧耗时也随之增长。
const STICKY_HIGHLIGHT_BUDGET: usize = 4_000;
/// 二进制检测读取的文件前缀长度（字节，8 KiB）。
///
/// 这段前缀中出现 NUL 即视为二进制文件；带 UTF-16 BOM 的文件除外。
pub(crate) const BINARY_SNIFF: usize = 8 << 10;

// 打开文件和全局搜索（project_search.rs）使用同一套判断：是否为二进制、如何去掉 BOM、
// 非 UTF-8 时如何解码。两处一致，搜索结果的匹配位置才与打开后的文本、行号对应。

/// 是否为二进制文件：开头 `BINARY_SNIFF` 字节内含 NUL。
///
/// UTF-16 文件含大量 NUL，因此开头有 BOM 时不算二进制。`bytes` 应从文件开头起，超出
/// `BINARY_SNIFF` 的部分不检查。
pub fn looks_binary(bytes: &[u8]) -> bool {
    !utf16_bom(bytes) && memchr::memchr(0, &bytes[..bytes.len().min(BINARY_SNIFF)]).is_some()
}

/// 一个打开的文档：文本、选区、撤销历史、高亮状态，以及对应的磁盘文件。
///
/// 文本可能来自 `rope`（普通文件）或按需读取的磁盘文件（大文件），见 [`Content`]。文本每变化
/// 一次 `revision` 加 1，所有按内容缓存的派生数据（列对齐、折叠、粘滞滚动）都以它为准。
pub struct Document {
    /// 文档标识，在同一个 `Editor` 内唯一。
    pub id: DocumentId,
    /// 对应的磁盘文件；未命名文档和输出标签没有路径。
    pub path: Option<PathBuf>,
    /// 当前选区；`anchor` 与 `head` 相同时表示光标。
    ///
    /// 有多个光标时是其中的主选区，其余的见 `Document::extra`。
    pub selection: Selection,
    /// 连续上下移动光标时保持的显示列（从 0 起）；其他命令会将其重置。
    pub goal_cell: Option<usize>,
    /// 撤销与重做栈，同时记录内容是否已保存。
    pub history: History,
    /// 逐行的词法高亮状态。
    pub highlighter: Highlighter,
    /// 文件的换行风格；新插入的换行使用同一风格。
    pub line_ending: LineEnding,
    /// 文件的编码；保存时按它写回。
    pub encoding: Encoding,
    /// 从文件内容检测出的缩进；无法检测（新文件、没有缩进）时为 `None`，此时使用语言的默认值。
    pub detected_indent: Option<Indent>,
    /// `.editorconfig` 对该文件的设置，优先于检测出的缩进。
    pub editorconfig: FileConfig,
    /// 在状态栏为该文件选择的缩进，优先于 `.editorconfig` 和检测出的缩进。
    ///
    /// 仅在本次打开期间有效，不写入任何文件。
    pub indent_override: Option<Indent>,
    /// 在状态栏为该文件选择的 Tab 显示宽度（显示列）。
    pub tab_width_override: Option<usize>,
    /// 是否只读：预览图、二进制文件、含无法解码字节的文件都是只读。
    pub readonly: bool,
    /// 无法显示的文件：标签中只显示原因。
    ///
    /// 此时文本为空，文档只读。
    pub unsupported: Option<Unsupported>,
    /// 编辑区显示图而不是文本：图片文件，或切换到预览的 SVG。
    pub picture: Option<PictureState>,
    /// 表格：xlsx 工作簿、只看预览的 csv 和 tsv 在编辑区显示它，分栏时显示在右边。
    pub table: Option<Box<TableView>>,
    /// 编辑区显示并排差异而不是文本，差异由 `DiffView` 管理。
    ///
    /// 这样的文档只读、没有对应的文件。
    pub diff: Option<Box<DiffView>>,
    /// 编辑区显示 Git 日志而不是文本，日志由 `LogView` 管理。
    ///
    /// 只读、没有对应的文件。
    pub log: Option<Box<crate::git::log::LogView>>,
    /// 切换到预览之前的 `readonly`：SVG 的预览期间强制只读，切回源代码时恢复该值。
    readonly_before_preview: bool,
    /// 分栏预览（`PreviewMode::Split`）：文本仍然显示，右边的预览由界面管理。
    split: bool,
    /// Markdown 只看预览：编辑区画排好版的文档，光标所在的块显示原文，可以直接编辑（SVG 的
    /// 预览用 `picture`）。
    rendered: bool,
    /// 不对应文件的只读标签（格式化工具的输出、安装说明）的标题。
    pub title: Option<String>,
    /// 文本每变化一次加 1，搜索结果等缓存据此判断是否失效。
    pub revision: u64,
    /// 滚动位置的首行（行，从 0 起，可以是小数，用于平滑滚动）。
    pub scroll_line: f32,
    /// 横向滚动的偏移（像素）。
    pub scroll_x: f32,
    /// 主选区以外的其余选区（多光标）。
    pub(crate) extra: crate::editing::cursors::Extra,
    /// 撤销多光标操作的历史，以及键盘列选择正在框选的区域。
    pub(crate) cursor_memory: crate::editing::cursors::CursorMemory,
    /// 扩大选区时逐层记录的（扩大前，扩大后）选区；缩小选区时据此回退。
    pub(crate) grown: Vec<(Selection, Selection)>,
    /// 已折叠的块。
    pub(crate) folds: Folds,
    /// 书签，按行号从小到大排列；文本变化时随之移动。
    pub(crate) bookmarks: Bookmarks,
    /// CSV、TSV 按列对齐时每一列的起始位置，按（版本、Tab 宽度）缓存。
    columns: Option<ColumnsCache>,
    /// 上一次测量列宽时的文字和各列宽度：文字变了只重新测量改动的行。
    column_widths: Option<(ropey::Rope, ColumnWidths)>,
    /// 整个文件可折叠的块，按（版本、Tab 宽度）缓存。
    pub(crate) fold_cache: Option<FoldCache>,
    /// 粘滞滚动上一次向上查找的结果，键为（版本、Tab 宽度）；键不变时下一次直接复用。
    sticky_memo: ((u64, usize), Vec<Ancestry>),
    /// 最近几次改动：（改动后的版本号，第一处改动所在的行），供
    /// `Document::unchanged_lines_since` 判断哪些行没有变。
    edits: std::collections::VecDeque<(u64, usize)>,
    /// 文档的文本：`rope`，或按需读取磁盘的大文件。
    pub(crate) text: Content,
    /// 最近一次读写时磁盘文件的修改时间。
    disk_mtime: Option<SystemTime>,
    /// 最近一次读写时磁盘文件的大小（字节，含 BOM），用于判断文件是否只在末尾追加了内容。
    disk_len: u64,
    /// 磁盘文件已被其他程序修改，而文档也有未保存的修改：任何一边的内容都不能被静默覆盖。
    ///
    /// 保存或重新加载后清除。
    conflict: bool,
    /// 恢复现场时还没有读取的标签（见 `Document::pending`）：文本为空、只读，切换到该标签时
    /// 由 `crate::Editor::load` 换成读好的文档。
    pending: bool,
    /// 尚未读取的文档在读取时是否把二进制内容也按文本读入（「以文本方式打开」）。
    pending_as_text: bool,
}

/// 从磁盘读出的内容。
struct Loaded {
    text: Content,
    /// 检测到的编码。
    encoding: Encoding,
    /// 检测到的换行风格。
    line_ending: LineEnding,
    /// 读取时文件的修改时间。
    mtime: Option<SystemTime>,
    /// 读取时文件的大小（字节）。
    len: u64,
}

impl Document {
    /// 创建内容为空、没有路径的文档。
    pub fn empty(id: DocumentId) -> Self {
        Self::from_text(id, "")
    }

    /// 用给定文本创建没有路径的文档，换行风格取默认值。
    pub fn from_text(id: DocumentId, text: &str) -> Self {
        Self::from_content(id, Content::Rope(Rope::from_str(text)))
    }

    /// 用已有的内容 `text` 创建没有路径的文档，换行风格取默认值。
    fn from_content(id: DocumentId, text: Content) -> Self {
        Self {
            id,
            path: None,
            selection: Selection::default(),
            goal_cell: None,
            history: History::default(),
            highlighter: Highlighter::new(winc_syntax::detect("")),
            line_ending: LineEnding::default(),
            encoding: Encoding::Utf8,
            detected_indent: None,
            editorconfig: FileConfig::default(),
            indent_override: None,
            tab_width_override: None,
            readonly: false,
            unsupported: None,
            picture: None,
            table: None,
            diff: None,
            log: None,
            readonly_before_preview: false,
            split: false,
            rendered: false,
            title: None,
            revision: 0,
            scroll_line: 0.0,
            scroll_x: 0.0,
            extra: Default::default(),
            cursor_memory: Default::default(),
            grown: Vec::new(),
            folds: Folds::default(),
            bookmarks: Bookmarks::default(),
            columns: None,
            column_widths: None,
            fold_cache: None,
            sticky_memo: ((0, 0), Vec::new()),
            edits: std::collections::VecDeque::new(),
            text,
            disk_mtime: None,
            disk_len: 0,
            conflict: false,
            pending: false,
            pending_as_text: false,
        }
    }

    /// 读取 `path` 并创建文档。
    ///
    /// # Errors
    ///
    /// 文件不存在或没有读权限时返回 [`OpenError::Io`]；内容是二进制时返回
    /// [`OpenError::Binary`]；超过 `OPEN_LIMIT` 时返回 [`OpenError::TooLarge`]。
    pub fn open(id: DocumentId, path: PathBuf) -> Result<Self, OpenError> {
        Self::open_with(id, path, false)
    }

    /// 把二进制文件当作文本打开（「以文本方式打开」），只读。
    ///
    /// # Errors
    ///
    /// 与 [`Document::open`] 相同，但不会因为内容是二进制而返回 [`OpenError::Binary`]；
    /// 文件大小上限仍然生效。
    pub fn open_binary(id: DocumentId, path: PathBuf) -> Result<Self, OpenError> {
        Self::open_with(id, path, true)
    }

    fn open_with(id: DocumentId, path: PathBuf, allow_binary: bool) -> Result<Self, OpenError> {
        let loaded = load(&path, allow_binary)?;
        let mut document = Document::empty(id);
        document.set_path(path);
        document.set_content(loaded);
        Ok(document)
    }

    /// 创建只读、不对应文件的输出标签，标签上显示 `title`，内容为 `text`。
    pub fn output(id: DocumentId, title: &str, text: &str) -> Self {
        let mut document = Document::from_text(id, text);
        document.title = Some(title.to_owned());
        document.readonly = true;
        document
    }

    /// 创建并排差异的标签，标签上显示 `title`。
    ///
    /// 文本是右侧的内容，与差异视图共用同一份 `rope`（克隆只增加引用计数，不复制文本）。文档
    /// 只读，没有对应的文件。
    pub fn diff(id: DocumentId, title: &str, view: DiffView) -> Self {
        let mut document = Document::from_content(id, Content::Rope(view.new.text.clone()));
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.diff = Some(Box::new(view));
        document
    }

    /// 创建 Git 日志的标签，标签上显示 `title`。
    ///
    /// 没有文本，只读。
    pub fn log(id: DocumentId, title: &str, view: crate::git::log::LogView) -> Self {
        let mut document = Document::from_text(id, "");
        document.title = Some(title.to_owned());
        document.readonly = true;
        document.log = Some(Box::new(view));
        document
    }

    /// 返回 CSV、TSV 按列对齐时每一列的起始显示列；列宽由该列所有行中最宽的一格决定。
    ///
    /// 不是表格，或文件超过 `ALIGN_MAX` 时返回 `None`，此时按普通文本显示。结果按（版本、Tab 宽度）
    /// 缓存。
    pub fn columns(&mut self, tab_width: usize) -> Option<Arc<Columns>> {
        let key = (self.revision, tab_width);
        if let Some((cached, columns)) = &self.columns
            && *cached == key
        {
            return columns.clone();
        }
        let columns = self.column_widths(tab_width).map(|w| Arc::new(w.columns()));
        self.columns = Some((key, columns.clone()));
        columns
    }

    /// 当前文字各列的宽度：与上一次测量时比较，只重新测量改动的行；不是表格时为 `None`。
    fn column_widths(&mut self, tab_width: usize) -> Option<&ColumnWidths> {
        let Some(separator) = self.column_separator() else {
            self.column_widths = None;
            return None;
        };
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        let widths = match self.column_widths.take() {
            Some((old, mut widths))
                if widths.separator == separator && widths.tab_width == tab_width =>
            {
                let (head, old_end, new_end) = changed_lines(&old, rope);
                let removed = old.slice(old.line_to_char(head)..old.line_to_char(old_end));
                let inserted = rope.slice(rope.line_to_char(head)..rope.line_to_char(new_end));
                if widths.update(removed.chunks(), inserted.chunks()) {
                    widths
                } else {
                    ColumnWidths::measure(separator, rope.chunks(), tab_width)
                }
            }
            _ => ColumnWidths::measure(separator, rope.chunks(), tab_width),
        };
        let (_, widths) = self.column_widths.insert((rope.clone(), widths));
        Some(widths)
    }

    /// 与 [`Document::columns`] 相同，但只借用 `&self`。
    ///
    /// 缓存对应当前版本时直接使用，否则重新测量（整个文件扫一遍），且不写入缓存；要换算很多
    /// 位置时先用 [`Document::columns`] 取一次，再交给 `*_with` 系列的换算函数。
    pub fn current_columns(&self, tab_width: usize) -> Option<Arc<Columns>> {
        match &self.columns {
            Some((key, columns)) if *key == (self.revision, tab_width) => columns.clone(),
            _ => self.measure_columns(tab_width).map(Arc::new),
        }
    }

    /// 把第 `line` 行的行内列 `col`（字符偏移，从 0 起）换算为显示列；按列对齐的 CSV 返回
    /// 对齐后的位置。
    ///
    /// # Panics
    ///
    /// 第 `line` 行不存在，或 `tab_width` 为 0 且该行含制表符时 panic。
    pub fn col_to_cell(&self, line: usize, col: usize, tab_width: usize) -> usize {
        let columns = self.current_columns(tab_width);
        self.col_to_cell_with(columns.as_deref(), line, col, tab_width)
    }

    /// 同 [`Document::col_to_cell`]，按列对齐时的各列位置由调用方给出（[`Document::columns`]
    /// 的结果）。连续换算多处时不必每次都查缓存或重新测量。
    pub(crate) fn col_to_cell_with(
        &self,
        columns: Option<&Columns>,
        line: usize,
        col: usize,
        tab_width: usize,
    ) -> usize {
        let content = self.text.line_content(line);
        match columns {
            Some(columns) => Aligned { columns, tab_width }.col_to_cell(content.slice(), col),
            None => winc_core::text::col_to_cell(content.slice(), col, tab_width),
        }
    }

    /// 把第 `line` 行的显示列 `cell` 换算为行内列；按列对齐的 CSV 按列起点查找，其余按 Tab 宽度。
    ///
    /// # Panics
    ///
    /// 第 `line` 行不存在时 panic；在到达目标列之前遇到制表符且 `tab_width` 为 0 时同样 panic。
    pub fn cell_to_col(&self, line: usize, cell: usize, tab_width: usize) -> usize {
        let columns = self.current_columns(tab_width);
        self.cell_to_col_with(columns.as_deref(), line, cell, tab_width)
    }

    /// 同 [`Document::cell_to_col`]，各列位置由调用方给出，见 [`Document::col_to_cell_with`]。
    pub(crate) fn cell_to_col_with(
        &self,
        columns: Option<&Columns>,
        line: usize,
        cell: usize,
        tab_width: usize,
    ) -> usize {
        let content = self.text.line_content(line);
        match columns {
            Some(columns) => Aligned { columns, tab_width }.cell_to_col(content.slice(), cell),
            None => winc_core::text::cell_to_col(content.slice(), cell, tab_width),
        }
    }

    fn measure_columns(&self, tab_width: usize) -> Option<Columns> {
        let separator = self.column_separator()?;
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        Some(Columns::measure(separator, rope.chunks(), tab_width))
    }

    /// 按列对齐时的分隔符；不是表格、不在内存里或超过 `ALIGN_MAX` 时为 `None`。
    fn column_separator(&self) -> Option<char> {
        let separator = match self.highlighter.language().id {
            "csv" => ',',
            "tsv" => '\t',
            _ => return None,
        };
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        if rope.len_bytes() > ALIGN_MAX {
            return None;
        }
        // 第一行中分号多于逗号时，CSV 按分号分列。
        let first = rope.line(0);
        let count = |c: char| first.chars().filter(|&x| x == c).count();
        let separator = if separator == ',' && count(';') > count(',') {
            ';'
        } else {
            separator
        };
        Some(separator)
    }

    /// 创建对应文件已不在磁盘上的文档：内容为空，未保存的内容只能从备份恢复
    /// （见 [`Document::restore_unsaved`]）。
    ///
    /// `mtime` 是文件存在时的修改时间（记录在备份中）。有值时文件按已删除处理
    /// （[`Document::deleted_on_disk`]），自动保存不重建该文件，用户手动保存时才重新创建。
    pub fn missing(id: DocumentId, path: PathBuf, mtime: Option<SystemTime>) -> Self {
        let mut document = Document::empty(id);
        document.set_path(path);
        document.disk_mtime = mtime;
        document
    }

    /// 向只读的输出标签（标准输入、格式化工具的输出）末尾追加文本。
    ///
    /// 追加不进入撤销记录，也不算修改。内容不是 `rope`（大文件）时不做任何事。
    pub fn append_output(&mut self, text: &str) {
        let Content::Rope(rope) = &mut self.text else {
            return;
        };
        let line = rope.len_lines() - 1;
        let end = rope.len_chars();
        rope.insert(end, text);
        self.highlighter.invalidate_from(line);
        self.revision += 1;
        self.note_edit(line);
        self.appended(line);
    }

    /// 创建无法打开的文件对应的文档：只记录路径和原因 `reason`，文本为空，文档只读。
    pub fn unsupported(id: DocumentId, path: PathBuf, reason: Unsupported) -> Self {
        let mut document = Document::empty(id);
        document.set_path(path);
        document.readonly = true;
        document.unsupported = Some(reason);
        document
    }

    /// 返回文档的文本。
    pub fn text(&self) -> &Content {
        &self.text
    }

    /// 是否按大文件方式打开（原文留在磁盘上）。
    pub fn is_large(&self) -> bool {
        self.text.is_large()
    }

    /// 返回文档只读的原因（显示给用户）；可以编辑时返回 `None`。
    pub fn readonly_reason(&self) -> Option<&'static str> {
        match &self.text {
            _ if !self.readonly => None,
            _ if self.pending => Some(t!("editor-readonly-pending")),
            _ if self.unsupported.is_some() => Some(t!("editor-readonly-unsupported")),
            _ if self.previewing() => Some(t!("editor-readonly-previewing")),
            _ if self.title.is_some() => Some(t!("editor-readonly-tab")),
            _ if self.encoding == Encoding::Binary => Some(t!("editor-readonly-binary")),
            Content::Large(large) if large.has_overlong_lines() => {
                Some(t!("editor-readonly-long-lines"))
            }
            _ => Some(t!("editor-readonly-not-utf8")),
        }
    }

    /// 返回不能执行编辑命令 `command` 的原因（显示给用户）；可以执行时返回 `None`。
    pub fn edit_blocked(&self, command: &Command) -> Option<&'static str> {
        if !command.is_edit() {
            return None;
        }

        if let Some(reason) = self.readonly_reason() {
            return Some(reason);
        }

        // 作用于整个文件的命令需要读取并改写整个文件，大文件不支持。
        let whole_file = match command {
            Command::TrimTrailingWhitespace => true,
            Command::SortLines { .. } => {
                winc_core::transform::sorts_whole_file(&self.text, self.selection)
            }
            _ => false,
        };
        if whole_file && self.is_large() {
            return Some(t!("editor-large-whole-file"));
        }

        if matches!(command, Command::Undo | Command::Redo) {
            return None;
        }
        // 多光标时每个选区都会被改写，被删掉的文字都要记进撤销历史，按全部选区的总和计算。
        // 光标（空选区）不占字节，不必逐个计算。
        let mut total = 0;
        for selection in self.all_selections().filter(|s| !s.is_empty()) {
            total += self.text.byte_len(selection.from(), selection.to());
            if total > EDIT_LIMIT {
                return Some(t!("editor-selection-too-large-to-edit"));
            }
        }
        None
    }

    /// 返回第 `line` 行（从 0 起）开头的高亮状态。
    ///
    /// 普通文档从头推算并缓存，每次最多向后推算 `budget` 行；尚未推算到 `line` 时返回 `None`。
    ///
    /// 大文件不从头推算，只从 `line` 上方 `HIGHLIGHT_LOOKBACK` 行处开始推算，因此跳到文件末尾时
    /// 无需扫描全文；推算出的一段状态保留下来，滚动时继续使用
    /// （见 [`winc_syntax::Highlighter::state_near`]）。
    pub fn highlight_state(&mut self, line: usize, budget: usize) -> Option<LineState> {
        match &self.text {
            Content::Rope(rope) => self.highlighter.state_at(rope, line, budget),
            Content::Large(large) => Some(self.highlighter.state_near(
                &**large,
                line,
                HIGHLIGHT_LOOKBACK,
            )),
        }
    }

    /// 返回编辑区显示 `rows` 各行时固定在顶部的块（粘滞滚动，从外层到内层，最多 `max` 个），
    /// 以及高亮是否已推算到位。
    ///
    /// `rows` 是编辑区从上到下显示的行号（从 0 起）；`partial` 表示滚动停在某一行的中间。
    /// 第二个返回值为 `false` 时，高亮尚未推算到需要的位置，字符串中的行可能被当作代码，
    /// 需要在下一帧重新计算。
    pub fn sticky_scopes(
        &mut self,
        rows: &[usize],
        partial: bool,
        max: usize,
        tab_width: usize,
    ) -> (Vec<Scope>, bool) {
        // 判断块是否在字符串中，需要 `rows` 及其下方 `SCAN_DOWN` 行（查找块末尾的范围）的高亮状态，
        // 因此先把高亮推算到那里；编辑区绘制时只推算到第一行。预算内推算不完时
        // （例如刚跳到长文件靠后的位置）`settled` 为 `false`，下一帧重新计算。
        let last = rows.last().copied().unwrap_or(0) + scopes::SCAN_DOWN;
        let settled = self
            .highlight_state(last, STICKY_HIGHLIGHT_BUDGET)
            .is_some();
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| starts_quoted(highlighter, text, line);
        let key = (self.revision, tab_width);
        if self.sticky_memo.0 != key {
            self.sticky_memo = (key, Vec::new());
        }
        let memo = &mut self.sticky_memo.1;
        let found = scopes::sticky(
            text,
            language,
            rows,
            partial,
            max,
            tab_width,
            &mut quoted,
            memo,
        );
        // 高亮尚未推算到位时，字符串中的行可能被当作代码，这次的结果不缓存给下一帧。
        if !settled {
            memo.clear();
        }
        (found, settled)
    }

    /// 返回包住第 `line` 行的块的层数，即粘滞滚动时顶部需要固定的行数。
    pub fn scope_depth(&mut self, line: usize, tab_width: usize) -> usize {
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| starts_quoted(highlighter, text, line);
        let memo = match &self.sticky_memo {
            (key, memo) if *key == (self.revision, tab_width) => &memo[..],
            _ => &[],
        };
        scopes::depth(text, language, line, tab_width, &mut quoted, memo)
    }

    /// 返回位置 `from` 到 `to` 之间的文本（字符偏移），用于复制；超过 [`COPY_LIMIT`] 字节时
    /// 返回 `None`。
    pub fn text_range(&self, from: usize, to: usize) -> Option<String> {
        self.text.text_range(from, to, COPY_LIMIT)
    }

    /// 按文件名确定语言并重建高亮状态；文件名认不出时按首行的 `#!` 识别。语言变化会影响列对齐和
    /// 折叠，因此同时推进 `revision` 让按内容缓存的结果失效。
    pub fn set_language(&mut self, file_name: &str) {
        // `#!` 行很短，只取开头一段，超长的首行不必整行复制。
        let first_line: String = self
            .text
            .line_content(0)
            .slice()
            .chars()
            .take(SHEBANG_CHARS)
            .collect();
        self.choose_language(winc_syntax::detect_with_first_line(file_name, &first_line));
    }

    /// 改用语言 `language`（状态栏上选的）：重建高亮状态，按内容缓存的结果一起失效，见
    /// [`Document::set_language`]。
    ///
    /// 文件之后改名、另存为时按新的文件名重新识别。
    pub fn choose_language(&mut self, language: &'static winc_syntax::Language) {
        self.highlighter = Highlighter::new(language);
        // 列对齐、折叠、粘滞滚动的结果取决于语言，而缓存的键只有（版本、Tab 宽度）。改名换
        // 扩展名（`a.txt` → `a.csv`）时文本没有变化，因此递增 `revision` 使缓存重新计算。
        self.revision += 1;
        self.columns = None;
    }

    /// 标签上显示的名字：输出标签用 `title`，否则用文件名，没有文件名时是 `untitled`。
    pub fn name(&self) -> Cow<'_, str> {
        if let Some(title) = &self.title {
            return Cow::Borrowed(title);
        }
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or(Cow::Borrowed("untitled"), |n| n.to_string_lossy())
    }

    /// 是否有未保存的修改。
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// 最近一次读写时磁盘文件的修改时间。
    pub fn disk_mtime(&self) -> Option<SystemTime> {
        self.disk_mtime
    }

    /// 文件由本程序以别的方式写出后（PDF 预览把批注保存进文件），记下写出后的修改时间：之后检查
    /// 磁盘时认得这次写入，不当作被其他程序修改。
    pub fn adopt_disk_mtime(&mut self, mtime: SystemTime) {
        self.disk_mtime = Some(mtime);
    }

    /// 恢复备份中上次未保存的内容，返回备份写入后磁盘文件是否被外部修改；无法恢复时返回 `None`。
    ///
    /// 调用前文档应已按磁盘上当前的内容打开。恢复时把内容换成 `text`，算一次修改，⌘Z 回到磁盘上
    /// 的内容。`mtime` 与当前修改时间不同且恢复后有未保存的修改时，说明备份之后文件被其他程序改
    /// 过，标记为冲突并返回 `Some(true)`；`conflict` 表示备份时已处于冲突中。只读或大文件时不做
    /// 改动并返回 `None`。
    pub fn restore_unsaved(
        &mut self,
        text: &str,
        mtime: Option<SystemTime>,
        selection: Selection,
        conflict: bool,
    ) -> Option<bool> {
        let Content::Rope(rope) = &self.text else {
            return None;
        };
        if self.readonly {
            return None;
        }

        let old = rope.to_string();
        if old != text {
            let changes = diff::line_changes(&old, text);
            let before = self.selection;
            self.apply(Transaction::new(changes, before, before, EditKind::Other));
        }
        self.selection = selection.clamped(self.text.end_pos());

        let changed = conflict || (self.disk_mtime.is_some() && mtime != self.disk_mtime);
        if changed && self.is_dirty() {
            self.conflict = true;
        }
        Some(changed && self.is_dirty())
    }

    /// 应用一个事务并记入撤销栈。
    ///
    /// 只读文档忽略。
    pub fn apply(&mut self, transaction: Transaction) {
        if self.readonly {
            return;
        }

        transaction.apply(&mut Tracked {
            text: &mut self.text,
            folds: &mut self.folds,
            bookmarks: &mut self.bookmarks,
        });
        self.selection = transaction.after.clamped(self.text.end_pos());
        let extra = transaction.extra_after.clone();

        if !transaction.changes.is_empty() {
            let first = transaction.first_offset();
            self.history.push(transaction);
            self.after_change(first);
        }
        // 不带其余选区的事务改动文本后，其余选区随版本号变化自动作废；带着的按它恢复。
        if !extra.is_empty() {
            self.restore_extra(extra);
        }
    }

    /// 记下刚才那次改动（已递增 `revision`）从第 `line` 行开始。
    fn note_edit(&mut self, line: usize) {
        self.edits.push_back((self.revision, line));
        if self.edits.len() > EDIT_LOG {
            self.edits.pop_front();
        }
    }

    /// 从版本 `revision` 到现在，开头有多少行一直没变：之后的每次改动都在这些行之下。
    ///
    /// 只记得最近 `EDIT_LOG` 次改动；更早的版本，或者中间经过了不是编辑的变化（从磁盘重新
    /// 读取、换了语言）时返回 0。版本没变时返回 `usize::MAX`。
    pub fn unchanged_lines_since(&self, revision: u64) -> usize {
        let mut unchanged = usize::MAX;
        let mut count = 0;
        for &(at, line) in self.edits.iter().rev().take_while(|(at, _)| *at > revision) {
            debug_assert!(at <= self.revision);
            unchanged = unchanged.min(line);
            count += 1;
        }
        if self.revision.checked_sub(revision) == Some(count) {
            unchanged
        } else {
            0
        }
    }

    /// 文本变化后的善后：让变化所在行起的高亮失效，递增 `revision`，刷新折叠。
    ///
    /// `first_offset` 是第一处变化的位置（字符偏移）。
    pub(crate) fn after_change(&mut self, first_offset: usize) {
        let line = self.text.line_of(first_offset);
        self.highlighter.invalidate_from(line);
        // 粘滞滚动记下的结果在改动之上足够远时仍然适用：在屏幕下方打字时不必从头向上查找。
        let (key, memo) = &mut self.sticky_memo;
        if key.0 == self.revision {
            memo.retain(|a| a.survives_edit_at(line));
            key.0 += 1;
        }
        self.revision += 1;
        self.note_edit(line);
        self.refresh_folds();
        // 正在预览的 SVG 内容变化（例如文件被外部修改后重新读取）时，按新内容重新绘制。
        if self.picture.is_some() {
            self.picture = Some(PictureState::Loading);
        }
    }
}

/// 是否第 `line` 行（从 0 起）的开头处于多行字符串或块注释中。
///
/// 普通文档只使用已经推算好的高亮状态，不向后推算，尚未推算到的行视为不在其中；大文件按
/// `HIGHLIGHT_LOOKBACK` 回看推算。
pub(crate) fn starts_quoted(highlighter: &mut Highlighter, text: &Content, line: usize) -> bool {
    let state = match text {
        Content::Rope(rope) => highlighter.state_at(rope, line, 0),
        Content::Large(large) => Some(highlighter.state_near(&**large, line, HIGHLIGHT_LOOKBACK)),
    };
    state.is_some_and(|s| s != LineState::Normal)
}
