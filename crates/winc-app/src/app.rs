//! 应用主体：`App` 的状态、窗口和事件循环。
//!
//! 各项功能的方法按主题分在子模块里：子模块只读取和修改 `App` 的字段，不持有自己的长期状态；
//! `view/` 读取这些状态绘制一帧。事件从 `winit` 的事件循环进入，经 `keyboard`、`mouse` 转成
//! [`Action`](crate::keymap::Action)，或直接修改状态。

pub(crate) mod animation;
pub(crate) mod diff;
pub(crate) mod editor;
mod fields;
pub(crate) mod git;
mod input;
mod lifecycle;
pub(crate) mod preview;
mod settings;
#[cfg(test)]
mod snapshot;
mod status;
pub(crate) mod terminal;
pub(crate) mod tickets;
mod workspace;

pub use animation::Clock;
use animation::{
    EXIT_FADE, FOLD_ANIMATION, PICKER_ANIMATION, POPUP_IN, POPUP_OUT, SIDEBAR_ANIMATION, TAB_SLIDE,
    TREE_ANIMATION,
};
pub(crate) use diff::{DIFF_TEXT_PAD, DiffParts, DiffTarget, HunkKind, diff_parts};
pub use editor::conflict::{ConflictAction, OverwriteChoice};
pub use editor::editing::BraceKey;
pub use editor::editing::ScrollGlide;
pub use editor::folding::FoldMotion;
pub(crate) use editor::folding::fold_column;
pub use editor::format::{FormatState, ToolAnswer};
pub use editor::hint::NameHint;
pub(crate) use editor::scrollbar::{ScrollArea, Scrollbars, ThumbLook};
pub use editor::sticky::{StickyCache, StickyRow};
pub(crate) use git::branch::picker::{GitAnchor, PickerPart};
pub(crate) use git::changes::{ChangeTarget, MARK_W, mark_x, ruler_rows};
pub(crate) use git::commit::{
    COMMIT_ROW, CommitGroup, CommitLabels, CommitName, CommitOverflow, CommitParts, CommitRow,
    CommitRows, CommitTarget, MESSAGE_LINE, MESSAGE_PAD, STASH_GAP, StashLine, StashView,
    SyncState,
};
pub(crate) use input::context_menu::{
    CHECK_COLUMN, ICON_COLUMN, MenuIcon, POPUP_PAD, POPUP_TEXT_X, PopupLevel, PopupMenu, PopupRow,
    folder_rows,
};
pub use input::context_menu::{ContextMenu, MenuButton, MenuCommand};
pub use input::palette::palette;
pub(crate) use lifecycle::alert::{AlertButton, AlertKey, AlertPart, AlertParts};
use lifecycle::appearance::{configured_font, forced_theme};
pub(crate) use lifecycle::backup::{restore_at_launch, restore_tabs};
pub use lifecycle::closing::{CloseRequest, SaveChoice, SavePrompt};
pub use lifecycle::notice::{NoticeAction, NoticeGhost};
pub use lifecycle::title_bar::WindowControl;
pub(crate) use lifecycle::title_bar::resize_cursor;
use lifecycle::window::frame_interval;
pub use settings::{DialogCache, SettingsState};
pub use status::motion::{StatusEntry, StatusKey, StatusMotion};
pub use status::status_bar::{StatusItem, indent_label};
pub use workspace::file_drop::DropZone;
pub use workspace::files::{Pick, PlaceholderAction, TabBar, TabHit};
pub use workspace::global::{GlobalSearch, ResultRow};
pub(crate) use workspace::outline::{OUTLINE_GAP, has_children as outline_has_children};
pub use workspace::outline::{OutlineEmpty, OutlineState, OutlineTarget};
use workspace::picker::PickerMotion;
pub use workspace::picker::{
    PICKER_ICON_X, PICKER_MAX_ROWS, PICKER_PAD, PICKER_ROW, PICKER_TEXT_X, PLACES_FOOTER,
    PlacesGhost,
};
pub(crate) use workspace::recent::folder_label;
pub use workspace::session::StartAction;
pub use workspace::tab_drag::{TabDrag, TabSlide};
pub use workspace::tool_fold::ToolFold;
use workspace::tree::SidebarMotion;
pub(crate) use workspace::tree::tree_max_scroll;
pub use workspace::tree::{NameInput, TreeMotion};
pub use workspace::tree_drag::TreeDrag;
pub use workspace::tree_label::{LabelCause, LabelView, TreeLabels};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
// 只在 macOS 的系统回调（⌘Q 前是否询问）中读写。
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Theme, Window, WindowId};

use winc_core::text::{self, MAX_LINE_BYTES, MAX_LINE_CHARS};
use winc_core::{Lines, Selection};
use winc_editor::{COPY_LIMIT, Command, DocumentId, Editor, Entry, EntryKind, FileIndex, Search};
use winc_render::{FontError, FontKind, FontSpec, Metrics, Rect, Renderer};

use winc_config::{Config, Session, State, ThemeChoice, resolve_font};

use crate::bench::{Bench, Next};
use crate::field::{TextField, Unit};
use crate::layout::{
    Layout, PanelButton, RowAction, Scrollbar, SearchButton, Setting, SettingControl,
    SettingsLayout, SettingsPage, SettingsPart, SidebarView, THEME_CHOICES, TreeTool,
    editor_content_height, scroll_to_show,
};
use crate::theme::*;
use crate::view;

/// 后台线程和系统回调（面板、菜单、提示框）送回事件循环的事件。
#[derive(Debug)]
pub enum UserEvent {
    /// 打开或另存为面板已关闭；取消时路径为 `None`。
    Picked(Pick, Option<PathBuf>),
    /// 全局搜索产生了新结果，或已搜索完成。
    Searched,
    /// 快速打开的文件索引已建好。
    Indexed,
    /// macOS 上按下 ⌘Q（或程序坞中的「退出」）。
    ///
    /// 系统侧的退出已被取消，由事件循环按关闭窗口的规则处理。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    QuitRequested,
    /// 菜单栏的「设置…」（或 ⌘,）被触发；设置已打开时关闭设置。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenSettings,
    /// macOS 上全屏过渡中红绿灯的位置变了（见 `crate::macos::TrafficLights`）：重新计算布局并
    /// 重画。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    TrafficLightsMoved,
    /// 询问是否将应用移到「应用程序」文件夹（见 `crate::macos::install`）；在窗口显示后处理，
    /// 提示框依附于窗口。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OfferMoveToApplications,
    /// 「移到「应用程序」文件夹」提示框中的选择：是否移动，以及是否勾选「不再询问」。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MoveToApplications(bool, bool),
    /// 选择面板打不开：哪一次选择，以及原因（没有门户时为 `None`）；只有经桌面门户选择的平台
    /// 有这种情况。
    #[cfg(all(unix, not(target_os = "macos")))]
    PickFailed(Pick, Option<String>),
    /// 后台检查更新找到了新版本（版本号），或者状态栏上的提示可以收起了（`None`）。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateFound(Option<String>),
    /// 到了定时检查更新的时候：设置里开着时检查。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateTick,
    /// 应用内更新的状态变了。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateState(crate::winc::UpdateState),
    /// 右键菜单中选中一项，参数是 `AppKit` 菜单项的编号（`tag`）。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MenuCommand(isize),
    /// 删除确认框中选择了「移到废纸篓」，参数是要删除的各个路径。
    Delete(Vec<PathBuf>),
    /// 「要保存修改吗」提示框中的一个按钮被点击。
    SaveChoice(SaveChoice),
    /// 状态栏路径的目录菜单中选中了一个文件。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FolderItem(PathBuf),
    /// 文件树拖动后确认了「移动」：前面的各个路径移入后一个目录；勾选「不再询问」时第三个参数
    /// 为 `true`。
    Move(Vec<PathBuf>, PathBuf, bool),
    /// 「要在哪里打开」提示框中做了选择：目录、是否在新窗口打开、是否勾选了「不再询问」。
    OpenFolderIn(PathBuf, bool, bool),
    /// 后台格式化已有结果。
    Formatted,
    /// 提示框中回答了是否允许运行项目自带的格式化工具。
    ToolChoice(ToolAnswer),
    /// 确认框上同意了运行这个配置。
    RunApproved(Box<winc_config::RunConfig>),
    /// 提示框中回答了是否信任 Git 仓库（第一个参数）指纹为第二个参数的配置。
    GitTrust(PathBuf, u64, bool),
    /// 已取得用户 shell 的环境，格式化工具在其 `PATH` 中查找。
    FormatterReady,
    /// 后台查找定义有新进展。
    Definitions,
    /// 已打开目录中有文件被外部修改。
    FilesChanged,
    /// 后台读到一段标准输入（`winc -`）。
    StdinRead,
    /// 新启动的 `winc` 把要打开的目录和文件，以及它的激活令牌交给了整个应用（见
    /// [`crate::instance`]）；由应用层决定切到哪个窗口或新建窗口，收不收经通道答复。
    ///
    /// 不会交到窗口里。
    Launched(crate::instance::Launch, std::sync::mpsc::Sender<bool>),
    /// 系统交来要打开的文件和目录（拖到程序坞的图标上、文件管理器里的「打开方式」）；与
    /// [`UserEvent::Launched`] 一样由应用层决定开在哪个窗口里。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Opened(Vec<PathBuf>),
    /// 程序坞菜单里选了「新建窗口」。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NewWindow,
    /// 提示框里选了切到开着这个目录的另一个窗口。
    FocusFolder(PathBuf),
    /// 保存磁盘上已被修改的文档时，提示框上选择了是否覆盖。
    Overwrite(DocumentId, OverwriteChoice),
    /// 用户确认放弃未保存的修改，按指定编码重新打开文档。
    ReopenEncoding(DocumentId, winc_editor::Encoding),
    /// 后台完成了一张图片的解码，或一个 SVG 的绘制。
    ImageReady,
    /// 粘贴到文件树中的文件已在后台复制完：新路径，或失败的原因。
    Pasted(Result<Vec<PathBuf>, String>),
    /// 拖进文件树的文件和目录已在后台复制完：各个副本的路径，以及复制失败的说明。
    Copied(Vec<PathBuf>, Vec<String>),
    /// 窗口已关闭时点击了程序坞中的图标（macOS）：重新显示窗口。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Reopen,
    /// 后台的 `git status` 有结果了。
    GitStatus,
    /// 后台取到了某个文件在 Git 里的版本，可以打开差异了。
    GitContent,
    /// 后台重新比较好了一个差异标签。
    Rediffed,
    /// 后台提取好了大纲的符号。
    Outline,
    /// 后台的提交、回滚做完了，或者取到了上一次提交的信息。
    GitCommit,
    /// 确认框上确认了回滚这些文件。
    GitRollback(Vec<PathBuf>),
    /// 确认框上确认了中止进行中的合并、变基等操作。
    GitAbort,
    /// 提示框上选了切换到这个分支；`true` 是先储藏未提交的更改，否则把它们带过去。
    GitSwitchBranch(winc_editor::git::Branch, bool),
    /// 确认框上确认了强制推送当前分支。
    GitForcePush,
    /// 提示框上确认了提交勾上的文件并推送。
    GitCommitAndPush,
    /// 提示框上确认了撤销上一次提交。
    GitUndoCommit,
    /// 后台取到了某个文件在 `HEAD` 里的内容（行号栏改动标记的基准）。
    GitBase,
    /// 后台的分支操作（获取、拉取、推送、签出……）做完了。
    GitOperated,
    /// 后台的 Git 操作发来了输入请求。
    GitQuestion,
    /// 后台读取好了三栏合并的两侧。
    GitMergeSides,
    /// 后台取到了日志的一批提交、一个提交的详情或一个文件的差异。
    GitLog,
    /// 终端的辅助进程发来了消息。
    Terminal,
    /// 确认框上确认了关掉这个终端标签（里面还有程序在运行）。
    TerminalClose(u16),
    /// 确认框上确认了重新启动这个终端标签（伪终端断开了，里面的程序可能还在运行）。
    TerminalRestart(u16),
    /// 需要授权的 `winc` 命令安装或卸载结束了。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ShellCommandDone(crate::shell_command::Done),
    /// 本机的 shell 检测完了。
    TerminalShells(Vec<terminal::shells::ShellChoice>),
    /// 退出或关闭窗口前的确认结果（终端中有程序在运行）：用户确认时为真，取消时为假。
    TerminalQuit(bool),
    /// 确认框上确认了把文字粘贴到指定的终端标签（多行、程序没有开启括号粘贴）。
    TerminalPaste(u16, String),
    /// 确认框上确认了日志里对一个提交要做的事（签出提交本身、硬重置）。
    GitLogConfirmed(git::log::LogConfirmed),
    /// 确认框上确认了移除这个工作树；第二项是要一起删除的分支（勾选了「同时删除分支」）。
    GitRemoveWorktree(PathBuf, Option<String>),
    /// 后台算好了一个文档的逐行追溯。
    GitBlame,
    /// 后台检查或重读好了被其他程序改过的文件。
    Reread,
    /// 后台重读好了文件树里被其他程序改过的目录。
    TreeScanned,
    /// 后台存好了一个大文件。
    Saved,
    /// 后台读好了一个刚打开的文件。
    Loaded,
    /// 后台写好了一批热退出备份。
    BackedUp,
    /// 跨文件替换已在后台写回全部未打开的文件。
    Replaced,
    /// 确认框上确认了全部替换；参数为弹出确认框时结果列表中的匹配总数。
    ReplaceAllConfirmed(usize),
}

/// 编辑区顶上那条输入条现在用来输入什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// 跳转到行（`⌃G`）。
    GoTo,
    /// 储藏改动的说明（可以不写）。
    Stash,
    /// 新标签的名字；`at` 是打在哪个提交上（完整的提交号）。
    NewTag { at: String },
    /// 分支的新名字；`old` 是现在的名字。
    RenameBranch { old: String },
    /// PDF 批注的注释文字：给选中的字加带注释的高亮，或改写已有批注的注释。
    PdfComment(preview::pdf::Pending),
}

/// 键盘焦点所在的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Tree,
    /// 查找条的查找输入。
    Search,
    /// 查找条第二行的替换输入。
    Replace,
    /// 跳转条，或文件树中输入名字的那一行。
    Prompt,
    /// 侧栏搜索面板的查找输入。
    GlobalQuery,
    /// 侧栏搜索面板的替换输入。
    GlobalReplace,
    /// 侧栏搜索面板的「包含」输入。
    GlobalInclude,
    /// 侧栏搜索面板的「排除」输入。
    GlobalExclude,
    /// 侧栏搜索面板的结果列表。
    Results,
    /// 终端面板。
    Terminal,
    /// 终端的查找条。
    TerminalFind,
    /// 终端标签上改名字的输入框。
    TerminalRename,
    /// 快速打开的输入框（⌘P）。
    Picker,
    /// 设置浮层中正在编辑的字体名输入框。
    Setting,
    /// 设置浮层已打开且没有输入框在编辑：方向键在设置项之间移动并修改值。
    Settings,
    /// 设置中「快捷键」页的搜索框。
    KeySearch,
    /// 设置浮层分类栏顶部的搜索框。
    SettingsSearch,
    /// 侧栏提交视图的提交信息输入框。
    CommitMessage,
    /// 侧栏提交视图的改动列表。
    CommitList,
    /// 文件视图底部大纲的符号列表。
    Outline,
    /// 日志标签标题栏的筛选输入框。
    LogFilter,
    /// 功能对话框（新建工作树）里的输入框。
    Dialog,
}

impl Focus {
    /// 是否由某个输入框持有焦点（光标闪烁、输入法、剪贴板都按输入框处理）。
    pub fn is_field(self) -> bool {
        !matches!(
            self,
            Focus::Editor
                | Focus::Tree
                | Focus::Results
                | Focus::Settings
                | Focus::CommitList
                | Focus::Outline
                | Focus::Terminal
        )
    }

    /// 是否由侧栏搜索面板持有焦点（四个输入框或结果列表）。
    pub fn in_panel(self) -> bool {
        matches!(
            self,
            Focus::GlobalQuery
                | Focus::GlobalReplace
                | Focus::GlobalInclude
                | Focus::GlobalExclude
                | Focus::Results
        )
    }
    /// 是否由终端面板持有焦点（网格、查找框、标签上改名字的输入框）。
    pub fn in_terminal(self) -> bool {
        matches!(
            self,
            Focus::Terminal | Focus::TerminalFind | Focus::TerminalRename
        )
    }
}

/// 鼠标悬停的目标，用于绘制悬停效果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    None,
    /// PDF 上带注释的批注：（页号，在该页批注中的位置），停留后显示注释文字。
    PdfNote(usize, usize),
    /// 文件视图底部大纲里的一处（符号行、行首箭头、标题行及其右端的按钮）。
    Outline(OutlineTarget),
    /// 编辑区右侧的缩略图。
    Minimap,
    /// 应用绘制的标题栏上的窗口按钮。
    WindowControl(WindowControl),
    /// 可以拖动改变窗口大小的边缘，参数是方向。
    ResizeEdge(winit::window::ResizeDirection),
    /// 解决合并冲突的按钮。
    Merge(git::merge::MergeTarget),
    /// 快速打开面板或位置浮窗中的一行，参数是第几条结果（从 0 起）。
    PickerRow(usize),
    /// 选择面板里的一个按钮。
    PickerPart(PickerPart),
    ProjectName,
    SidebarButton,
    NewTab,
    /// 标签栏无法容纳全部标签时显示在最右端的「⌄」，用于列出全部标签。
    TabList,
    /// 标签栏右端「源代码 / 分栏 / 预览」三段切换里的一个。
    Preview(winc_editor::PreviewMode),
    /// 标签栏右端「在浏览器中打开」。
    Browser,
    /// Markdown 预览里的链接（`MarkdownPreview::links` 的序号）。
    MarkdownLink(usize),
    /// Markdown 预览左侧的标题指示条或展开的标题列表。
    MarkdownToc(crate::app::preview::markdown::TocTarget),
    Search(SearchButton),
    TreeRow(usize),
    /// 标签序号，以及是否悬停在该标签的关闭按钮上。
    Tab(usize, bool),
    Splitter,
    /// 编辑区和终端面板之间的间距（拖动改面板的大小）。
    TerminalDivider,
    /// 文件视图里大纲标题行的上沿（上下拖动改大纲的高度）。
    OutlineDivider,
    /// 分栏预览中间的分隔线（左右拖动改两半的宽度）。
    SplitDivider,
    /// 终端面板标签栏上的标签、关闭按钮、「+」和位置按钮。
    TerminalHeader(terminal::HeaderHit),
    /// 终端里命令块的色带、吸顶的命令行。
    TerminalBlock(terminal::BlockHit),
    Placeholder(PlaceholderAction),
    /// 开始页上的一个可点击项。
    Start(StartAction),
    /// 文件树工具栏上的按钮。
    TreeTool(TreeTool),
    /// 文件树工具栏收起后的「⋯」按钮。
    TreeMore,
    /// 侧栏顶部的「文件」「搜索」切换按钮。
    ViewTab(SidebarView),
    /// 搜索面板中的按钮。
    Panel(PanelButton),
    /// 搜索结果的行序号。
    Result(usize),
    /// 悬停某一行搜索结果时出现的按钮，参数依次为行序号和按钮。
    RowAction(usize, RowAction),
    /// 状态栏路径中的一段，参数是段序号。
    Crumb(usize),
    /// 状态栏右侧可点击的一项（缩进、行尾）。
    Status(StatusItem),
    /// 编辑区顶部钉住的一层，参数是层级。
    Sticky(usize),
    /// 编辑区行号栏：显示可折叠行的箭头。
    Gutter,
    /// 已折叠行末尾的 `⋯`，参数是行号（从 0 起）。
    Folded(usize),
    /// 滚动条的可点击区域。
    Scrollbar(Scrollbar),
    /// 提示条上的按钮。
    NoticeButton,
    /// 冲突卡片上的按钮。
    ConflictButton(ConflictAction),
    /// 差异标签里可以点的地方。
    Diff(DiffTarget),
    /// 提交视图里可以点的地方。
    Commit(CommitTarget),
    /// 行号栏改动卡片上的按钮。
    Change(ChangeTarget),
    /// 行号栏的改动标记，参数是第几处改动。
    ChangeMark(usize),
    /// 滚动条（概览标尺）上的改动标记，参数是第几处改动。
    RulerChange(usize),
    /// 日志标签里可以点的地方。
    Log(git::log::LogTarget),
    /// 逐行追溯那一栏的第几行（从 0 起）。
    Blame(usize),
}

impl Hover {
    /// 是否为悬停足够久后会显示提示文字的按钮。
    pub fn has_tooltip(self) -> bool {
        matches!(
            self,
            Hover::TreeTool(_)
                | Hover::PdfNote(..)
                | Hover::TerminalHeader(
                    terminal::HeaderHit::Position
                        | terminal::HeaderHit::ShellMenu
                        | terminal::HeaderHit::Find(_)
                        | terminal::HeaderHit::Hide
                        | terminal::HeaderHit::Run(_),
                )
                | Hover::Status(StatusItem::Terminal | StatusItem::Settings)
                | Hover::TreeMore
                | Hover::SidebarButton
                | Hover::Tab(..)
                | Hover::TabList
                | Hover::Preview(_)
                | Hover::Browser
                | Hover::ViewTab(_)
                | Hover::Outline(OutlineTarget::FoldAll)
                | Hover::Search(_)
                | Hover::Panel(
                    PanelButton::Tool(_)
                        | PanelButton::Details
                        | PanelButton::Expand
                        | PanelButton::Case
                        | PanelButton::Word
                        | PanelButton::Regex
                        | PanelButton::ReplaceAll
                )
                | Hover::RowAction(..)
                | Hover::Change(_)
                | Hover::RulerChange(_)
                | Hover::Log(
                    git::log::LogTarget::CopyOid
                        | git::log::LogTarget::Refresh
                        | git::log::LogTarget::Avatar(_)
                        | git::log::LogTarget::Row(_)
                        | git::log::LogTarget::File(_)
                        | git::log::LogTarget::Line(..)
                )
                | Hover::Blame(_)
                | Hover::PickerPart(PickerPart::Pull | PickerPart::Push | PickerPart::Fetch)
                | Hover::Commit(
                    CommitTarget::Log
                        | CommitTarget::Sync
                        | CommitTarget::Location
                        | CommitTarget::Refresh
                        | CommitTarget::More
                        | CommitTarget::Commit
                        | CommitTarget::CommitMenu
                        | CommitTarget::OpenFile(_)
                        | CommitTarget::RevertFile(_)
                        | CommitTarget::GroupRevert(_)
                        | CommitTarget::Check(_)
                )
                | Hover::Diff(
                    DiffTarget::Previous
                        | DiffTarget::Next
                        | DiffTarget::PreviousFile
                        | DiffTarget::NextFile
                        | DiffTarget::Layout
                        | DiffTarget::Collapse
                        | DiffTarget::Revert(_)
                        | DiffTarget::Include(_)
                        | DiffTarget::IncludeLine(..)
                )
                | Hover::Merge(git::merge::MergeTarget::Previous | git::merge::MergeTarget::Next)
        )
    }
}

/// 正在进行的鼠标拖动。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    None,
    /// 拖选文字。
    ///
    /// `by_word` 表示双击后按词扩展；`origin` 是起点（按词时是起始词的范围）。
    Select {
        by_word: bool,
        origin: (usize, usize),
    },
    /// 拖选整行：三击或点击行号栏后，按整行扩大选区；`line` 是按下的那一行。
    SelectLine {
        line: usize,
    },
    /// 列选择：`⇧⌥` 拖动或中键拖动；`anchor` 是按下处的（行，显示列）。
    Columns {
        anchor: (usize, usize),
    },
    /// `⌥` 拖动：添加一个选区；`origin` 是按下处的位置（字符偏移）。
    AddSelect {
        origin: usize,
    },
    Splitter {
        start_x: f32,
        start_width: f32,
    },
    /// 在日志的详情里拖着选字（`App::drag_log_text`）。
    LogText,
    /// 在 Markdown 预览里拖着选字（`App::drag_markdown`）。
    MarkdownText,
    /// 在 PDF 预览里拖着选字（`App::drag_pdf`）。
    PdfText,
    /// 在表格中拖着选单元格（`App::drag_table`）。
    TableCells,
    /// 在终端里拖着选字。
    TerminalSelect,
    /// 终端里的程序要了鼠标：按着这个键拖动，松开时上报。
    TerminalMouse(winc_pty::input::Button),
    /// 拖编辑区和终端面板之间的间距：按下时的坐标和面板当时的尺寸。
    ///
    /// 面板在下面时是纵坐标
    /// 和高度，在右边时是横坐标和宽度。
    TerminalDivider {
        start: f32,
        size: f32,
    },
    /// 拖大纲标题行的上沿：按下时的纵坐标、符号列表当时的高度（逻辑像素）。
    OutlineDivider {
        start: f32,
        size: f32,
    },
    /// 拖分栏预览中间的分隔线：按下时的横坐标、源代码一侧当时的宽度（逻辑像素）。
    SplitDivider {
        start_x: f32,
        start_left: f32,
    },
    /// 拖日志标签里列表和详情之间的分隔线：按下时的横坐标、详情一栏的宽度。
    LogDivider {
        start_x: f32,
        start_width: f32,
    },
    /// 拖动缩略图的滑块；指针在纵坐标 `from`（逻辑像素）时，滚动位置是 `scroll`。
    Minimap {
        from: f32,
        scroll: f32,
    },
    /// 拖动滚动条。
    ///
    /// `grab` 是按下点到滑块起点（顶边或左边）的距离（像素）。
    Scrollbar {
        bar: Scrollbar,
        grab: f32,
    },
    /// 在当前持有焦点的输入框中拖选文字。
    Field,
    /// 在差异的右侧拖选：`unit` 是按字、按词还是按行扩展，`origin` 是按下时选中的范围
    /// （右侧文档里的字符偏移）；`add` 为 `true` 时是 `⌥` 拖动，只改主选区、保留其余光标。
    DiffSelect {
        unit: diff::SelectUnit,
        origin: (usize, usize),
        add: bool,
    },
    /// 在差异的右侧框出一列（`⇧⌥` 拖动、中键拖动）；`anchor` 是按下处的（行，显示列）。
    DiffColumns {
        anchor: (usize, usize),
    },
    /// 在差异只读的一侧拖选；`unit`、`origin` 的含义同 [`Drag::DiffSelect`]，按第 `side` 侧的
    /// 文本算。
    DiffSelectRead {
        side: usize,
        unit: diff::SelectUnit,
        origin: (usize, usize),
    },
}

/// 一次鼠标按下的记录，用于识别连击。
#[derive(Debug, Clone, Copy)]
struct Click {
    /// 按下的时刻。
    at: Instant,
    /// 按下位置的横坐标（逻辑像素）。
    x: f32,
    /// 按下位置的纵坐标（逻辑像素）。
    y: f32,
    /// 连击次数：1 为单击，2 为双击选词，3 为三击选行。
    count: u8,
}

/// 光标闪烁的半周期，即显示与隐藏各持续的时间。
///
/// 530 ms 取自调参：与常见终端和编辑器的闪烁节奏一致，肉眼可辨又不显跳动。
const BLINK: Duration = Duration::from_millis(530);
/// 多久没有输入以后光标停止闪烁、一直亮着（秒）。
///
/// 停下以后不再定时重绘，窗口长时间不变时不再按半秒的间隔唤醒。
const BLINK_STOP: Duration = Duration::from_secs(10);
/// 两次按下判定为连击的最大间隔。
///
/// 400 ms 有约束：不得短于系统双击间隔的常见取值，否则正常的双击会被判成两次单击。
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// 全局搜索进行中，界面主动收取一次结果的间隔。
///
/// 100 ms 取自调参：搜索结果以这个频率并入界面，等待期间输入仍能及时响应。
const GLOBAL_POLL: Duration = Duration::from_millis(100);

/// 「只显示匹配的行」的缓存键，各项依次为：文档、版本号、匹配的行数、已搜索到的位置、是否有查询。
type FilterKey = (DocumentId, u64, usize, usize, bool);

/// 应用状态：编辑器、界面状态、绘制时记录的几何，以及窗口和事件循环的句柄。
///
/// 各功能的事件处理方法分布在 `app/` 的子模块中，均只读写本结构的字段。
pub struct App {
    // 编辑器、配置和历史
    /// 打开的文档、工作区、光标和选区。
    pub editor: Editor,
    /// 终端面板（终端模块）。
    pub(crate) terminal: terminal::TerminalPanel,
    /// 当前目录的运行配置。
    pub(crate) run: terminal::run::RunConfigs,
    /// 当前生效的配置。
    pub config: Config,
    /// 配置文件当前的内容，即上次读入或写出时的配置。
    ///
    /// 写回时只写入 `config` 中与本字段不同的项，程序运行期间在配置文件里手动修改的其他项保持
    /// 不变。
    pub(crate) saved_config: Config,
    /// 配置写回的文件；`None` 时只修改内存中的配置（测试使用，避免改动用户的配置）。
    pub config_path: Option<PathBuf>,
    /// 配置中的主题选择；`Auto` 时跟随系统外观。
    pub theme_choice: ThemeChoice,
    /// 最近打开的目录和各目录的现场。
    pub state: State,
    /// 状态写回的文件；`None` 时只记录在内存中（测试和性能测量使用，避免改动用户的历史）。
    pub state_path: Option<PathBuf>,
    /// 未保存内容的备份，用于退出后恢复。
    pub backup: lifecycle::backup::BackupState,
    /// 临时打开（`--wait`、`-`）时需要等待的标签和标准输入的读取状态。
    pub(crate) one_off: lifecycle::one_off::OneOff,
    /// 测试和测量时不真正打开终端，只记录要在哪个目录打开。
    pub opened_terminal: Option<PathBuf>,

    // 外观
    /// 当前主题的颜色。
    pub palette: &'static Palette,
    /// 界面字体和等宽字体的字体、字号。
    pub fonts: ViewFonts,
    /// 界面字体的行高、基线等尺寸（逻辑像素）。
    pub ui_metrics: Metrics,
    /// 等宽字体的行高、基线等尺寸（逻辑像素）。
    pub mono_metrics: Metrics,
    /// 等宽字体单个字符格的宽度（逻辑像素）。
    pub cell: f32,
    /// 终端字体的行高、基线等尺寸（逻辑像素）。
    pub terminal_metrics: Metrics,
    /// 终端里一个字符格的宽度（逻辑像素）。
    pub terminal_cell: f32,

    // 焦点、鼠标和键盘
    /// 键盘焦点所在的区域。
    pub focus: Focus,
    /// 鼠标悬停的目标。
    pub hover: Hover,
    pub(crate) modifiers: ModifiersState,
    /// 左、右 ⌥ 各自按着没有（终端里哪一边当 Meta，见 `terminal.optionAsMeta`）。
    option_side: (bool, bool),
    /// 鼠标位置（逻辑像素）。
    mouse: (f32, f32),
    drag: Drag,
    last_click: Option<Click>,
    /// 上一次在顶栏空白处的按下，用于识别双击。
    title_click: Option<Click>,
    /// 是否由应用绘制标题栏和窗口按钮（见 `lifecycle::title_bar`）。
    pub(crate) window_controls: bool,
    /// 鼠标按下时所在的标签关闭按钮（或中键按下的标签）；松开时仍在同一个标签上才关闭。
    close_pressed: Option<DocumentId>,
    cursor_icon: CursorIcon,
    window_focused: bool,
    /// 拖选时最近一次因鼠标超出编辑区上下边缘而滚动的时刻，用于按经过的时间计算滚动量
    /// （见 `scroll_while_dragging`）；鼠标不在边缘之外时为 `None`。
    drag_scrolled_at: Option<Instant>,

    // 光标和输入法
    /// 是否绘制光标：闪烁到亮的一半时为真；窗口没有焦点时为假。
    pub caret_visible: bool,
    /// 光标闪烁周期的起点；按键、点击等操作会重置该起点，使光标立即显示。
    blink_epoch: Instant,
    /// 编辑区中输入法正在组字的文字。
    pub preedit: String,
    /// 输入框中输入法正在组字的文字。
    pub field_preedit: String,
    /// 输入法候选框应出现的位置。
    pub ime_cursor: Option<Rect>,
    last_ime_area: Option<Rect>,

    // 布局，以及绘制时记录、点击时使用的几何
    /// 上一帧排出的几何：各区域的位置与大小，点击时按它反推点到了哪里。
    pub layout: Layout,
    /// 标签栏的标签和滚动状态。
    pub tabs: TabBar,
    /// 无法打开的文件的占位页上，各按钮的位置。
    pub placeholder_hits: Vec<(Rect, PlaceholderAction)>,
    /// 差异标签里可以点的地方（标题栏的按钮、折起来的行），绘制时更新。
    pub diff_hits: Vec<(Rect, DiffTarget)>,
    /// 开始页上可点击的区域，绘制时记录。
    ///
    /// 同一处有两个时（最近一行及其 ✕），✕ 排在前面。
    pub start_hits: Vec<(Rect, StartAction)>,
    /// 状态栏路径中各段的可点击区域，以及段序号。
    pub crumb_hits: Vec<(Rect, usize)>,
    /// 状态栏右侧各可点击项的区域。
    pub status_hits: Vec<(Rect, StatusItem)>,
    /// 编辑区顶部钉住的各行，绘制时记录，点击和悬停时使用。
    pub sticky: Vec<StickyRow>,
    sticky_cache: Option<StickyCache>,
    /// 滚动条上标出的光标所在词的缓存。
    pub(crate) ruler_cache: Option<crate::view::RulerCache>,
    /// 图片预览缩放后的图片缓存。
    pub(crate) picture_cache: Option<crate::view::PictureCache>,
    /// 后台的图片解码和 SVG 绘制任务。
    pub(crate) images: preview::images::ImageJobs,
    pub(crate) tables: preview::tables::TableJobs,
    /// 表格预览上次绘制的几何：工作表标签、最大滚动距离和列宽缓存。
    pub(crate) table_layout: crate::view::preview::table::TableLayout,
    /// 分栏预览右边那张图。
    pub(crate) split: preview::images::SplitPreview,
    /// Markdown 预览（分栏右边或只看预览）。
    pub(crate) markdown: preview::markdown::MarkdownPreview,
    /// 当前标签的 PDF 预览。
    pub(crate) pdf: preview::pdf::PdfPreview,
    /// 状态栏上键路径的缓存，键为（文档、版本号、光标位置）。
    key_path_cache: Option<((DocumentId, u64, usize), Option<String>)>,
    /// 当前文档 JSON 键路径的扫描记录，换了文档就重来。
    key_path_scan: Option<(DocumentId, winc_editor::keypath::KeyPathScan)>,
    /// 已折叠行末尾 `⋯` 的可点击区域和行号，绘制时记录，点击后展开。
    pub folded_hits: Vec<(Rect, usize)>,
    /// 侧栏标题行项目名的可点击区域，绘制时填入。
    pub project_hit: Rect,
    /// 文件树工具栏各按钮的位置，绘制时填入，点击时使用。
    pub tree_tools: Vec<(Rect, TreeTool)>,
    /// 工具栏收起后「⋯」按钮的位置；所有按钮都放得下时为 `None`。
    pub tree_more: Option<Rect>,
    /// 工具栏按钮收进「⋯」、从「⋯」里出来的动画。
    pub(crate) tool_fold: ToolFold,
    /// 鼠标所在文件树行的名字显示不全时，叠加在编辑区上方绘制的区域，以及该行的序号。
    pub tree_overflow: Option<(Rect, usize)>,
    /// 各区域滚动条的显示状态：平时隐藏，滚动后或鼠标移到滚动条上时显示。
    pub(crate) scrollbars: Scrollbars,

    // 侧栏和文件树
    panel_width: f32,
    /// 分栏预览时源代码一侧占编辑区宽度的比例，0 到 1；`None` 表示各占一半。拖动中间的分隔线
    /// 时改变，所有分栏的标签共用。
    pub(crate) split_share: Option<f32>,
    /// 侧栏是否应显示；动画进行中取动画的终点。
    panel_visible: bool,
    sidebar_motion: Option<SidebarMotion>,
    /// ⌘P 面板上次排版时的高度，以及当时的窗口大小。
    picker_height: Option<(f32, (f32, f32))>,
    picker_motion: Option<PickerMotion>,
    /// 分支、位置选择器里正在展开或收起的目录、组的动画。
    pub(crate) picker_fold: Option<git::branch::picker::PickerFold>,
    /// 位置浮窗（定义、用法、书签）在名字上方时，浮窗底边的纵坐标：高度变化时底边不动，
    /// 顶边移动。
    ///
    /// 在下方或没有打开时为 `None`。
    places_bottom: Option<f32>,
    /// 光标所在名字旁边的提示气泡（例如没有找到定义）；没有时为 `None`。
    pub(crate) name_hint: Option<NameHint>,
    /// 位置浮窗中滚轮还没攒够一行的滚动量（逻辑像素）：浮窗按整行滚动。
    places_wheel: f32,
    /// 位置浮窗出现的动画（滑出并淡入）；动画结束后为 `None`。
    pub(crate) places_in: Option<Clock>,
    /// 位置浮窗关掉后正在淡出的样子；没有在淡出时为 `None`。
    pub(crate) places_out: Option<PlacesGhost>,
    /// `⌘P` 面板出现、消失的动画。
    pub(crate) picker_card: animation::CardFx,
    /// 编辑区顶上输入条出现、消失的动画。
    pub(crate) bar_card: animation::CardFx,
    /// 新建工作树对话框；没打开时为 `None`。
    ///
    /// 打开时盖在整个窗口上，键盘和鼠标都先交给它。
    pub(crate) form_dialog: Option<git::form::FormDialog>,
    /// 窗口里画出来的提示框（没有系统提示框时，见 `lifecycle/alert.rs`）。
    pub(crate) alert: Option<lifecycle::alert::InAppAlert>,
    /// 对话框出现、消失的动画。
    pub(crate) dialog_card: animation::CardFx,
    /// 窗口中画出来的提示框出现、消失的动画。
    pub(crate) alert_card: animation::CardFx,
    /// 侧栏当前显示文件树还是全局搜索。
    pub sidebar_view: SidebarView,
    /// 切换侧栏视图时，视图切换里凸起的那一段从上一个视图滑过来：上一个视图和动画时钟。
    pub view_slide: Option<(SidebarView, animation::Clock)>,
    /// Git、终端模块开关时视图切换里的「提交」一段、状态栏的终端开关和分支名出现、消失的动画。
    pub(crate) module_fx: animation::ModuleFx,
    /// 状态栏右侧各项的过渡状态。
    pub(crate) status_motion: StatusMotion,
    /// 文件树的纵向滚动量（逻辑像素）。
    pub tree_scroll: f32,
    /// 文件树的横向滚动量；缩进、图标和名字整体向左移动。
    pub tree_scroll_x: f32,
    /// 文件树中最宽一行的内容宽度，横向滚动范围和滑块据此计算；绘制时量出。
    pub tree_content_w: f32,
    /// 屏幕上合并显示的目录占的横向范围：所在行的下标、左右边界（逻辑像素）和目录路径；绘制时量出。
    pub tree_segments: Vec<(usize, f32, f32, PathBuf)>,
    /// 鼠标所在的合并显示的目录；不在这样的目录上时为 `None`。
    pub tree_hover_segment: Option<PathBuf>,
    /// 文件树各行名字的过渡状态。
    pub tree_labels: TreeLabels,
    /// 编辑区水平滚动条的内容宽度（逻辑像素），绘制时更新；没有水平滚动条时为 0。
    pub text_content_w: f32,
    /// 文件树中连续键入的字符与最后一次键入的时刻，用于跳到名称以这些字符开头的行
    /// （`App::on_key_tree`）。
    pub(crate) tree_typed: Option<(String, std::time::Instant)>,
    /// 目录展开、收起的动画。
    pub tree_motion: Option<TreeMotion>,
    /// 代码折叠、展开的动画。
    pub fold_motion: Option<FoldMotion>,
    /// 编辑区跳转式滚动（翻页、跳到行、查找跳转）的平滑滚动状态。
    pub(crate) scroll_glide: Option<ScrollGlide>,
    /// 鼠标滚轮滚动的过渡状态；没有进行中的过渡时为 `None`。
    pub(crate) wheel_glide: Option<input::mouse::WheelGlide>,
    /// 上一次鼠标滚轮整格滚动的时刻，用于按两格的间隔确定过渡时长。
    wheel_at: Option<Instant>,
    /// 预览标签的文档；是否仍是预览标签见 [`App::preview_index`]。
    preview_tab: Option<winc_editor::DocumentId>,
    /// 文件树中正在改名字的项和输入框；没有时为 `None`。
    pub name_input: Option<NameInput>,
    /// 在文件树中按住的项；拖动该项可移动到其他目录。
    pub tree_drag: Option<TreeDrag>,
    /// 从系统文件管理器等来源拖着文件经过窗口时，松手会落在哪里；没有在拖时为 `None`。
    file_hover: Option<DropZone>,

    // 编辑区
    /// 高亮状态尚未推算到可见区域，需要再绘制一帧。
    pub highlight_pending: bool,
    /// 光标旁括号的配对结果缓存。
    pub brace_cache: Option<(BraceKey, Option<(usize, usize)>)>,
    /// 系统剪贴板，首次使用时才初始化。
    #[cfg(not(test))]
    clipboard: Option<arboard::Clipboard>,
    /// 测试中代替系统剪贴板的文本。
    #[cfg(test)]
    pub copied: Option<String>,
    /// 测试中代替系统剪贴板的文件列表。
    #[cfg(test)]
    pub copied_files: Vec<PathBuf>,
    /// 测试中代替系统剪贴板的图片（PNG）。
    #[cfg(test)]
    pub copied_image: Option<Vec<u8>>,
    /// 最近一次整行复制的内容；粘贴相同内容时按整行插入到当前行上方。
    line_clipboard: Option<String>,

    // 查找条和跳转条
    /// 当前查找的内容、各文档里已找到的匹配和替换状态。
    pub search: Search,
    /// 查找条中的查找输入；内容变化后同步到 `search.query`。
    pub find_input: TextField,
    /// 查找条中的替换输入；内容变化后同步到 `search.replacement`。
    pub replace_input: TextField,
    search_open: bool,
    /// 查找条上是否开启了「只显示匹配的行」。
    pub(crate) filter_lines: bool,
    /// 上次筛选使用的缓存键，以及筛选后是否隐藏了行。
    filter_key: Option<(FilterKey, bool)>,
    replace_open: bool,
    /// 大文件尚未搜索完时，等找到「下一个」匹配再跳转：（文档，起点位置，是否向前查找）。
    pending_jump: Option<(DocumentId, usize, bool)>,
    /// 跳转条（⌃G）的输入；`None` 表示跳转条未打开。
    pub goto_input: Option<TextField>,

    /// 侧栏搜索面板的状态。
    pub global: GlobalSearch,
    /// 文件视图底部大纲的状态。
    pub outline: OutlineState,

    // 快速打开
    /// 工作区中各文件的路径与匹配用的索引。
    pub picker: FileIndex,
    /// 快速打开面板是否打开。
    pub picker_open: bool,
    /// 面板打开时的当前文档；之后当前文档变了就关掉面板（见 `relayout`）。
    picker_doc: Option<DocumentId>,
    /// 快速打开面板中的查询输入。
    pub picker_input: TextField,
    /// 当前查询命中的文件索引下标。
    pub picker_hits: Vec<usize>,
    /// 当前选中的结果在 `picker_hits` 中的序号（从 0 起）。
    pub picker_selected: usize,
    /// 分支选择器打开时自动选中的行（`picker_selected` 的值）；移动选中项或输入查询后为 `None`。
    ///
    /// 自动选中的行以浅色底显示：它不一定是当前分支（单列最近使用时选中上一个用过的分支，回车即
    /// 切回），强调色底会被误认为当前分支。当前分支由行首的对勾标出。
    pub picker_default: Option<usize>,
    /// 结果开头属于「最近打开」的条数（未输入查询时列在最前面）。
    pub picker_recent: usize,
    /// 查询末尾的 `:行` 或 `:行:列`，打开文件后跳转到该位置。
    pub picker_goto: Option<String>,
    /// 目录监视不可用时，窗口重新获得焦点后目录内容可能已有增删；下次打开 ⌘P 时在后台重新
    /// 扫描文件表。
    picker_stale: bool,
    /// 已打开目录的文件监视器；目录内的文件被外部修改时发出通知。
    watcher: Option<winc_editor::Watcher>,
    /// 已收到目录变化通知后，统一处理变化的时刻。
    changes_due: Option<Instant>,
    /// 在后台重读的、被其他程序改过的文件。
    rereads: workspace::watch::Rereads,
    /// 在后台重读的、文件树里被其他程序改过的目录。
    tree_scans: workspace::watch::TreeScans,
    /// 在后台保存的大文件。
    saves: workspace::background_save::BackgroundSaves,
    /// 在后台读的、刚打开的文件。
    loads: workspace::loading::Loads,
    /// 结果列表的滚动位置（像素）。
    pub picker_scroll: f32,
    /// 最近切换到的文件，最近的在前。
    ///
    /// 快速打开（`⌘P`）未输入时先列出这些（当前文件除外），
    /// 回车即可切回上一个。
    pub recent_files: Vec<PathBuf>,
    /// 命令面板中最近使用的命令，最近的在前；未输入时先列出这些。
    pub recent_commands: Vec<crate::keymap::Action>,
    /// 在快速打开中输入 `@` 时列出的当前文件的符号。
    pub picker_symbols: Option<editor::symbols::SymbolList>,
    /// 两段组合键（如 `⌘K ⌘S`）的第一段，正在等待第二段。
    pub(crate) pending_chord: Option<crate::keymap::Stroke>,
    /// 「后退」「前进」使用的位置历史。
    pub(crate) navigation: editor::navigation::Navigation,
    /// 转到定义找到多个位置时，卡片中列出的位置。
    pub picker_places: Option<editor::definition::Places>,
    /// 面板列出分支（分支选择器）时的各行。
    pub picker_branch_rows: Option<Vec<git::branch::picker::BranchRow>>,
    /// 面板列出位置（位置选择器）时的各行。
    pub picker_location_rows: Option<Vec<git::location_picker::LocationRow>>,
    /// 面板是否列出最近打开的目录（`⌃R`）；为真时 `picker_hits` 是最近列表中的下标。
    pub picker_folders: bool,
    /// 分支选择器、位置选择器从哪儿展开。
    pub picker_anchor: GitAnchor,
    /// 选择面板这一帧画出来的按钮：点击、悬停都按它们认。
    pub picker_part_hits: Vec<(Rect, PickerPart)>,
    /// 位置选择器里改变过默认展开状态的仓库（主工作树的目录）。
    pub location_toggled: std::collections::HashSet<PathBuf>,
    /// 分支选择器里改变过默认展开状态的目录：（是否远程，目录路径）。
    pub branch_folders: std::collections::HashSet<(bool, String)>,
    /// 正在后台项目中进行的定义查找。
    definitions: Option<editor::definition::DefinitionSearch>,

    /// 设置浮层：当前页、分类和各输入框。
    pub settings: SettingsState,

    /// 保存时格式化和修复的状态。
    pub format: FormatState,
    /// Git 模块：仓库状态、文件树装饰。
    ///
    /// 模块关着时为空。
    pub git: git::GitState,
    /// Git 的提交视图：勾选、提交信息。
    pub commit: git::commit::CommitState,
    /// 提交视图里可以点的地方，绘制时更新。
    pub commit_hits: Vec<(Rect, CommitTarget)>,
    /// 行号栏的 Git 改动标记和点开的卡片。
    pub changes: git::changes::LineChanges,
    /// Git 日志标签的后台通道和点击区域。
    pub log: git::log::LogState,
    /// 逐行追溯（git blame）。
    pub blame: git::blame::BlameState,
    /// 正在做的分支操作。
    pub branch: git::branch::BranchState,
    /// 在文件里解决合并冲突：认出的冲突、可以点的地方。
    pub merge: git::merge::MergeState,
    /// 差异里「»」换掉一段改动以后正在淡出的颜色。
    pub diff_fade: Option<diff::DiffFade>,
    /// 跳到一处改动时那一段的短暂高亮；没有时为 `None`。
    pub(crate) diff_pulse: Option<Clock>,
    /// 差异切换文件时，新文件打开以后停在哪里：`true` 是最后一处改动，`false` 是第一处；没有在切换
    /// 文件时为 `None`。
    pub(crate) diff_land: Option<bool>,
    /// 差异走到头时等待再按一次的状态和提示的时刻；没有在等待时为 `None`。
    pub(crate) diff_cross: Option<(Instant, diff::DiffCross)>,
    /// 大文件的差异在后台重新比较。
    pub(crate) rediff: diff::RediffState,
    /// 跳转条现在用来输入什么。
    pub prompt_kind: PromptKind,

    // 右键菜单、关闭和退出
    /// 右键菜单的目标和内容，选中一项时据此执行。
    pub context_menu: Option<ContextMenu>,
    /// 系统菜单打开期间，点开该菜单的按钮绘制为按下状态。
    pub menu_button: Option<MenuButton>,
    /// 删除文件使用的函数：正常运行时移到废纸篓，测试中替换为直接删除，避免写入用户的废纸篓。
    pub trash: fn(&Path) -> std::io::Result<()>,
    /// 正在询问「要保存修改吗」。
    pub save_prompt: Option<SavePrompt>,
    /// 已选择「保存」、正在等待另存为面板：保存完成后继续处理余下的步骤。
    pub save_continuation: Option<SavePrompt>,
    /// 退出前的处理均已完成；当前一轮事件处理结束后退出。
    pub quit_requested: bool,
    /// 窗口已关闭（隐藏），应用仍在运行：macOS 上关闭窗口不退出应用，点击程序坞中的图标时
    /// 重新显示窗口。
    pub window_hidden: bool,
    /// 最近关闭的标签（路径和光标位置），⇧⌘T 按关闭的相反顺序重新打开。
    pub closed: Vec<(PathBuf, usize)>,
    /// 退出前是否需要询问用户（各窗口共用同一个值，见 [`crate::winc::Shared::ask_on_quit`]）。
    pub(crate) ask_on_quit: Arc<AtomicBool>,
    /// 正在确认是否结束终端中运行的程序时，确认后要执行的关闭（退出或关闭窗口）。
    ///
    /// 为 `Some` 期间，退出流程尚未得出结果（见 [`crate::winc`]）。
    pub(crate) quit_confirming: Option<CloseRequest>,

    // 状态栏消息和悬停提示
    /// 状态栏上的临时消息。
    pub status_message: Option<String>,
    /// 文件在磁盘上被删掉、自动保存没写的文档：已经提示过用户的，不每次失去焦点都再说一遍。
    deleted_told: HashSet<DocumentId>,
    status_until: Option<Instant>,
    /// 编辑区右上角需要用户注意的提示。
    pub notice: Option<String>,
    notice_until: Option<Instant>,
    /// 提示条上的按钮。
    pub notice_action: Option<NoticeAction>,
    /// 提示条上按钮的位置，绘制时记录。
    pub notice_button: Option<Rect>,
    /// 提示条的位置，绘制时记录：浮在编辑区上面，点击、悬停、右键都不落到下面的文字上。
    pub notice_card: Option<Rect>,
    /// 鼠标是否在窗口内：移进来时置位，离开窗口时清除。
    cursor_inside: bool,
    /// 冲突卡片及其按钮的位置，绘制时记录；当前文档没有冲突时为 `None`。
    pub conflict_card: Option<(Rect, Vec<(Rect, ConflictAction)>)>,
    /// 提示条出现的动画；结束后为 `None`。
    pub(crate) notice_in: Option<Clock>,
    /// 到期后正在淡出的提示条：消息、按钮文字和消失的动画。
    pub(crate) notice_out: Option<NoticeGhost>,
    /// 鼠标开始悬停在工具栏按钮上的时刻；悬停足够久后才显示提示。
    pub tooltip_since: Option<Instant>,
    /// 提示是否已绘制出来；已绘制时无需再为提示唤醒事件循环。
    pub tooltip_shown: bool,

    // 窗口和事件循环
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// 后台线程和系统回调通过该代理把事件送回事件循环（带着本窗口的编号，见 [`crate::winc`]）；
    /// 测试中没有事件循环，为 `None`。
    proxy: Option<crate::winc::WindowProxy>,
    /// 窗口的红绿灯（见 [`crate::macos::TrafficLights`]），窗口创建后才有，随窗口一同释放。
    #[cfg(target_os = "macos")]
    traffic_lights: Option<crate::macos::TrafficLights>,
    /// 本窗口在进程里的编号；测试中单独建的 `App` 没有。
    pub(crate) key: Option<crate::winc::WindowKey>,
    /// 各窗口共用的状态：别的窗口开着什么、剪切的文件，以及请应用层做的事。
    pub(crate) shared: std::rc::Rc<crate::winc::Shared>,
    /// 配置的版本：本窗口每写一次配置文件加一，应用层据此把新配置发给其他窗口。
    pub(crate) config_rev: u64,
    /// 新窗口的位置（外框左上角，物理像素）和打开它的窗口的缩放比例：比打开它的窗口往右下错开
    /// 一段。建窗口时按当前的显示器校验后使用一次，见 `App::cascade_from`。
    pub(crate) cascade: Option<((f64, f64), f64)>,
    /// 窗口正常显示（没有最大化、全屏、最小化）时的大小和位置：关闭时记下来；最小化期间不更新，
    /// 从最小化的窗口打开新窗口时按它错开（见 `App::cascade_from`）。
    pub(crate) window_frame: Option<winc_config::WindowFrame>,
    /// 新窗口和打开它的窗口共用的字体和字形缓存（建渲染器时用一次）。
    pub(crate) glyphs_from: Option<winc_render::SharedGlyphs>,
    /// 待使用的激活令牌（见 [`App::set_activation`]）。
    #[cfg(all(unix, not(target_os = "macos")))]
    activation: Option<winit::window::ActivationToken>,
    /// 窗口完全看不见（被挡住、最小化、在别的桌面上）：不画，屏幕上那块画面标成可清除。
    occluded: bool,
    /// 动画下一帧的绘制时刻（没有显示链接时按计时器安排）。
    next_frame: Option<Instant>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(target_os = "macos")]
    display_link: Option<crate::macos::DisplayLink>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(windows)]
    display_link: Option<crate::windows::DisplayLink>,
    /// 相邻两帧的间隔；动画开始时按窗口所在显示器的刷新率计算。
    frame_interval: Duration,
    bench: Bench,
    /// 帧日志（`WINC_FRAME_LOG`）。
    frame_log: crate::frame_log::FrameLog,
    /// 已呈现的帧数，供测量钩子使用。
    presented: usize,
    dirty: bool,
}

impl App {
    /// 请求在下一帧重画本窗口。
    pub(crate) fn request_redraw(&mut self) {
        self.dirty = true;
    }

    /// 在后台线程上做 `job`，做完以事件 `done` 叫醒界面线程收结果（[`App::finished`]）。
    ///
    /// 测试里没有事件循环：在当前线程上执行 `job`，随后直接调用 [`App::finished`] 收结果。
    fn spawn_then(&mut self, done: UserEvent, job: impl FnOnce() + Send + 'static) {
        match self.proxy.clone() {
            Some(proxy) => winc_editor::background::spawn(move || {
                job();
                let _ = proxy.send_event(done);
            }),
            None => {
                job();
                self.finished(done);
            }
        }
    }

    /// 和 [`App::spawn_then`] 相同，但 `job` 在单独创建的线程上执行
    /// （[`winc_editor::background::spawn_blocking`]）：`job` 运行子进程并等待其结束，耗时
    /// 没有上限。
    ///
    /// 测试里没有事件循环：在当前线程上执行 `job`，随后直接调用 [`App::finished`] 收结果。
    fn spawn_blocking_then(&mut self, done: UserEvent, job: impl FnOnce() + Send + 'static) {
        match self.proxy.clone() {
            Some(proxy) => winc_editor::background::spawn_blocking(move || {
                job();
                let _ = proxy.send_event(done);
            }),
            None => {
                job();
                self.finished(done);
            }
        }
    }

    /// 后台任务做完了（`done`，见 [`App::spawn_then`]）：收下它交回来的结果。
    ///
    /// 事件循环和测试里就地运行时都走这里，哪个事件收哪份结果只写一处。
    fn finished(&mut self, done: UserEvent) {
        match done {
            UserEvent::GitStatus => self.poll_git(),
            UserEvent::GitContent => self.poll_git_content(),
            UserEvent::GitCommit => self.poll_commit(),
            UserEvent::GitBase => self.poll_bases(),
            UserEvent::GitOperated => self.poll_branch_ops(),
            UserEvent::GitMergeSides => self.poll_merge_sides(),
            UserEvent::GitLog => self.poll_log(),
            UserEvent::GitBlame => self.poll_blame(),
            UserEvent::Reread => self.poll_rereads(),
            UserEvent::TreeScanned => self.poll_tree_scans(),
            UserEvent::Saved => self.poll_saves(),
            UserEvent::Loaded => self.poll_loads(),
            UserEvent::BackedUp => self.collect_backups(false),
            UserEvent::Replaced => self.poll_replace_all(),
            other => debug_assert!(false, "{other:?} 不是后台任务做完的事件"),
        }
    }

    /// 本窗口的系统窗口编号；窗口还没建好时为 `None`。
    pub(crate) fn window_id(&self) -> Option<winit::window::WindowId> {
        self.window.as_ref().map(|w| w.id())
    }

    /// 返回鼠标位置（逻辑像素）。
    pub(crate) fn mouse(&self) -> (f32, f32) {
        self.mouse
    }

    /// 是否正按着鼠标拖动（选文字、拖分隔条等）。
    pub(crate) fn dragging(&self) -> bool {
        self.drag != Drag::None
    }

    /// 是否按着主修饰键：macOS 上是 ⌘，其他系统上是 Ctrl（⌘ 单击多选、比较、打开链接都按它）。
    pub(crate) fn command_held(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        }
    }

    /// 设置悬停目标。
    ///
    /// 目标是带提示的按钮时记录悬停开始的时刻，悬停足够久后才显示提示。
    pub(super) fn set_hover(&mut self, hover: Hover) {
        // 窗口内的菜单盖在上面：下面的部件都不算悬停（`hover_at` 也一样）。
        let hover = if self.popup().is_some() {
            Hover::None
        } else {
            hover
        };
        if hover == self.hover {
            return;
        }

        // 鼠标移到预览的标题指示条上：标题列表展开，淡入。
        let toc = |h: Hover| matches!(h, Hover::MarkdownToc(_));
        if toc(hover) && !toc(self.hover) {
            self.markdown.toc_fade = self.clock(animation::TOC_FADE);
        }
        self.hover = hover;
        self.tooltip_since = hover.has_tooltip().then(Instant::now);
        self.tooltip_shown = false;
        self.request_redraw();
    }

    /// 结束所有进行中的拖动。
    ///
    /// 按住鼠标时切换到其他应用不会收到松开事件；若不结束拖动，切回后不按键移动鼠标，分隔条、选区、
    /// 滚动条和标签仍会跟随移动。
    fn drop_drags(&mut self) {
        self.cancel_tree_drag();
        self.release_tab();
        self.drag = Drag::None;
        self.close_pressed = None;
    }

    fn reset_blink(&mut self) {
        self.blink_epoch = Instant::now();
        self.caret_visible = true;
    }

    /// 在每帧绘制之前推进搜索：继续搜索一个时间片；等待中的跳转已有结果时执行跳转，并按
    /// 搜索结果过滤行。
    fn advance_search(&mut self) {
        // 查找条关闭时也可能有等待中的跳转：⌘G、F3 在大文件中一个时间片内没有搜到下一处。
        let searching = self.search_open || self.pending_jump.is_some();
        // 查找对着预览时查预览的文字。
        let preview = searching && self.markdown_focused() && self.markdown_find_doc().is_some();
        let doc = if preview {
            self.markdown.find_doc.as_ref()
        } else {
            self.editor.active()
        };
        match doc {
            Some(doc) if searching => {
                self.search.update(doc);

                // 记录的跳转只对当时的文档有效，切换标签后作废。
                match self.pending_jump {
                    Some((id, pos, backwards)) if id == doc.id => {
                        self.jump_to_match(pos, backwards);
                    }
                    Some(_) => self.pending_jump = None,
                    None => {}
                }
            }
            // 标签全部关闭时，等待中的跳转作废；否则会一直按「尚未搜完」处理，每帧都继续绘制。
            None => self.pending_jump = None,
            Some(_) => {}
        }
        self.apply_line_filter();
    }
}
