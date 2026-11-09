//! 右键菜单（文件树、标签栏和编辑区）、点击项目名弹出的最近项目菜单，以及状态栏和设置中的
//! 菜单。
//!
//! 每个菜单先用 [`MenuEntry`] 描述各项（文字、快捷键、图标、子菜单），再交给平台显示；
//! 选中后返回 [`MenuCommand`]，由 [`App::run_menu_command`] 统一执行。

use std::path::{Path, PathBuf};
use winc_editor::encoding::CHOICES;

use winc_core::{Indent, LineEnding};
use winc_editor::format::{Auto, PRESETS, Task};
use winc_editor::{DocumentId, EntryKind};
use winc_i18n::t;
#[cfg(any(target_os = "macos", test))]
use winc_render::Image;

use super::*;
use crate::app::git::location_picker::Place;
use crate::icons;
use crate::keymap::{self, Action, Context, Stroke};
use crate::layout::{goto_bar, hit};

mod menus;
mod popup;
mod present;
mod run;
#[cfg(test)]
mod tests;

pub use menus::*;
pub(crate) use popup::{
    CHECK_COLUMN, ICON_COLUMN, POPUP_PAD, POPUP_TEXT_X, PopupLevel, PopupMenu, PopupRow,
    folder_rows,
};
pub(crate) use present::MenuIcon;
pub use present::MenuLabel;

/// 菜单项左边图标的宽和高（逻辑像素），等于 `icons::folder` 绘制的目录图标的尺寸 15×12。
const MENU_ICON: (f32, f32) = (15.0, 12.0);

/// 系统菜单在第一项上方留出的空白（逻辑像素）。
///
/// 弹出位置是第一项的左上角，因此菜单外框比内容高出此值。5 为 macOS 26 上的实测值。
const MENU_TOP_PADDING: f32 = 5.0;

/// 按钮下方弹出的菜单与按钮的间距（逻辑像素，经验值），所有按钮菜单（包括「⋯」）共用。
const MENU_GAP: f32 = 4.0;

/// 弹出系统菜单的按钮。
///
/// 菜单打开期间，按钮绘制为按下的样子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuButton {
    /// 侧栏顶部的项目名。
    Project,
    /// 提交视图标题行的「⋯」。
    CommitMore,
    /// 提交视图「提交 ⌄」拆分按钮的箭头。
    CommitButton,
    /// 提交视图底部「上一次提交」那一栏的「⋯」。
    LastCommit,
    /// 文件树工具栏收起按钮以后的「⋯」。
    TreeMore,
    /// 标签栏无法容纳全部标签时显示在最右端的「⌄」。
    TabList,
    /// 终端面板标签栏上的运行配置名。
    RunConfigs,
    /// 终端面板标签栏上「+」右边选 shell 的按钮。
    TerminalShells,
    /// 状态栏路径中的第几段（从 0 起）。
    Crumb(usize),
    /// 状态栏右侧的缩进、行尾等项。
    Status(StatusItem),
}

/// 菜单中的一条命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    NewFile,
    NewFolder,
    /// 在系统文件管理器中显示。
    Reveal,
    OpenInTerminal,
    /// 只在该目录中做全局搜索。
    FindInFolder,
    CopyPath,
    CopyRelativePath,
    /// 编辑区：复制 `路径:行`。
    CopyReference,
    Rename,
    Delete,
    /// 文件树里多选时：在编辑区打开选中的各个文件（目录跳过）；搜索结果里：打开这一行所在的
    /// 文件，匹配行还选中这一处。
    OpenFiles,
    /// 文件树里正好选中两个文件时：并排比较它们。
    CompareFiles,
    Close,
    CloseOthers,
    CloseRight,
    CloseLeft,
    /// 关闭所有没有未保存修改的标签。
    CloseSaved,
    CloseAll,
    /// 在文件树中显示并选中该文件。
    RevealInTree,
    /// 标签列表：切换到指定标签，参数为 `documents` 的下标。
    ShowTab(usize),
    /// 选中第几个运行配置。
    RunSelect(usize),
    /// 用第几个 shell 新建终端。
    NewTerminalShell(usize),
    /// 打开运行配置文件。
    RunEdit,
    /// 项目名菜单：最近列表中的目录，参数为列表下标（从 0 起）。
    OpenRecent(usize),
    /// 项目名菜单（按住 `⌥`）：在新窗口中打开最近列表中的目录。
    OpenRecentInNewWindow(usize),
    OpenFolder,
    /// 项目菜单：打开 WSL 发行版中的目录。
    OpenWslFolder(usize),
    CloseFolder,
    /// 提交视图「⋯」：新建工作树（输入新分支的名字）。
    NewWorktree,
    /// 位置选择器的操作菜单：在新窗口打开这个位置。
    LocationNewWindow,
    /// 位置选择器的操作菜单：在系统的文件管理器里显示。
    LocationReveal,
    /// 位置选择器的操作菜单：移除这个工作树。
    LocationRemove,
    /// 位置选择器的操作菜单：清理目录已不存在的工作树。
    LocationPrune,
    Cut,
    Copy,
    Paste,
    GoToDefinition,
    FindReferences,
    /// 在当前行添加书签。
    AddBookmark,
    /// 删除当前行的书签。
    RemoveBookmark,
    /// 列出当前文档的全部书签。
    ListBookmarks,
    Format,
    FormatSelection,
    Fix,
    /// 设置中「添加语言…」菜单的一项，参数为语言在 `winc_syntax::languages()` 中的序号（从 0 起）。
    Language(usize),
    /// 设置中「排除的语言」菜单的一项：切换某种语言保存时是否跳过格式化和修复，
    /// 参数为语言序号（从 0 起）。
    SkipOnSave(usize),
    /// 设置中「语言」菜单的一项，参数为 [`LocaleChoice::ALL`](winc_i18n::LocaleChoice::ALL)
    /// 中的序号（从 0 起）。
    Locale(usize),
    /// 设置中「空白符」菜单的一项，参数为
    /// [`RenderWhitespace::ALL`](winc_config::RenderWhitespace::ALL) 中的序号（从 0 起）。
    Whitespace(usize),
    /// 设置中「默认 Shell」菜单的一项：`None` 为系统默认，`Some(i)` 为检测到的第 `i` 个 shell
    /// （从 0 起）。
    DefaultShell(Option<usize>),
    /// 设置中「默认 Shell」菜单的「自定义命令…」，进入输入框。
    DefaultShellCustom,
    /// 设置中「界面缩放」菜单的一项，参数为 [`ZOOM_STEPS`](crate::app::settings::ZOOM_STEPS)
    /// 中的序号（从 0 起）。
    Zoom(usize),
    /// 设置中终端「位置」菜单的一项：`true` 为右侧，`false` 为底部。
    TerminalPosition(bool),
    /// 设置中「Meta 键」菜单的一项，参数为
    /// [`OptionAsMeta::ALL`](winc_config::OptionAsMeta::ALL) 中的序号（从 0 起）。
    OptionAsMeta(usize),
    /// 设置中字体菜单的「系统默认」（终端字体为「跟随编辑器」）。
    FontDefault,
    /// 设置中字体菜单的一项，参数为 [`winc_render::installed_fonts`] 中的序号（从 0 起）。
    Font(usize),
    /// 设置中某种语言的格式化（修复）工具：自动选择。
    ToolAuto,
    /// 设置中某种语言的格式化（修复）工具：预设，参数为 `PRESETS` 中的序号（从 0 起）。
    ToolPreset(usize),
    /// 设置中某种语言的格式化（修复）工具：自定义命令，进入输入框。
    ToolCustom,
    /// 设置中某种语言的格式化（修复）工具：显示安装方法。
    ToolInstall,
    /// 设置中某种语言的格式化工具：把这种语言从列表中移除。
    ToolRemove,
    IndentUsingSpaces,
    IndentUsingTabs,
    /// 缩进宽度：空格缩进时为空格数，制表符缩进时为一个 Tab 占的列数。
    IndentSize(usize),
    /// 把已有的缩进转换为空格。
    ConvertToSpaces,
    /// 把已有的缩进转换为制表符。
    ConvertToTabs,
    /// 从内容中重新检测缩进方式。
    DetectIndentation,
    /// 更换行尾。
    LineEnding(LineEnding),
    /// 通过编码重新打开，参数为编码在 `encoding::CHOICES` 中的序号（从 0 起）。
    ReopenEncoding(usize),
    /// 通过编码保存，参数为编码在 `encoding::CHOICES` 中的序号（从 0 起）。
    SaveEncoding(usize),
    /// 状态栏语言菜单：改用这种语言，参数为语言在 `winc_syntax::languages()` 中的序号（从 0 起）。
    UseLanguage(usize),
    /// 状态栏语言菜单：改为纯文本。
    UsePlainText,
    /// 状态栏语言菜单：按文件名重新识别。
    DetectLanguage,
    /// Git：获取所有远程。
    GitFetch,
    /// Git：拉取。
    GitPull,
    /// 拉取并把本地提交变基到上游（`pull --rebase`）。
    GitPullRebase,
    /// 添加远程仓库。
    GitAddRemote,
    /// 对第几个远程仓库做的事。
    GitRemote(crate::app::git::branch::RemoteOp, usize),
    /// Git：推送当前分支。
    GitPush,
    /// Git：继续停下来的合并、变基……（合并是打开提交视图去完成）。
    GitContinue,
    /// 菜单项「Git：中止停下来的合并、变基……」。
    GitAbort,
    /// 菜单里分支一组的小标题（不能点）：`false` 本地分支，`true` 远程分支。
    GitHeading(bool),
    /// 对分支列表里第几个分支做的事。
    GitBranch(crate::app::git::branch::BranchOp, usize),
    /// 提交视图里的文件：打开它和 `HEAD` 的差异。
    GitShowDiff,
    /// 提交视图里的文件：在编辑区打开文件本身。
    GitOpenFile,
    /// 提交视图里的文件：回滚这一个文件（先确认）。
    GitRollbackFile,
    /// 提交视图里多选时：勾上（`true`）或勾掉选中的各个文件。
    CommitCheck(bool),
    /// 差异里的文字：把光标处的一处改动换回 `HEAD` 的样子。
    GitRevertHunk,
    /// 文件树、标签上的文件：打开它的 Git 历史。
    FileHistory,
    /// 日志的分支筛选：0 是全部分支，`i + 1` 是分支列表里第 `i` 个分支。
    LogBranch(usize),
    /// 状态栏的位置菜单：切换到第几个位置。
    StatusPlace(usize),
    /// 新建工作树的起点：0 是当前分支，`i + 1` 是分支列表里第 `i` 个分支。
    FormBase(usize),
    /// 日志的作者筛选：0 是全部作者，`i + 1` 是作者菜单里第 `i` 个作者。
    LogAuthor(usize),
    /// 日志里的提交：签出（有本地分支指着它就签出该分支，没有就签出提交本身，先确认）。
    LogCheckout,
    /// 日志里的提交：从这里新建分支（输入名字）。
    LogNewBranch,
    /// 日志：在新工作树中签出这个提交（有能签出的本地分支指着它时签出该分支）。
    LogNewWorktree,
    /// 日志里的提交：拣选到当前分支（cherry-pick）。
    LogCherryPick,
    /// 日志里的提交：撤销（revert，新提交一次反过来的改动）。
    LogRevert,
    /// 日志里的提交：把当前分支重置到这里，保留改动并暂存（`reset --soft`）。
    LogResetSoft,
    /// 同上，保留改动不暂存（`reset --mixed`）。
    LogResetMixed,
    /// 同上，丢弃改动（`reset --hard`，先确认）。
    LogResetHard,
    /// 日志里的提交：在这里打一个标签（输入名字）。
    LogNewTag,
    /// 从这个提交开始交互式变基。
    LogInteractiveRebase,
    /// 交互式变基列表一行的处理方式。
    RebaseAction(winc_editor::git::RebaseAction),
    /// 对提交上第几个标签做的事。
    LogTag(crate::app::git::branch::TagOp, usize),
    /// 日志里的提交：复制完整的提交号。
    LogCopyOid,
    /// 日志里的提交：复制前 7 位提交号。
    LogCopyShortOid,
    /// 日志里的提交：复制完整的说明。
    LogCopyMessage,
    /// 编辑区：开关逐行追溯（开着时打勾）。
    ToggleBlame,
    /// 提交视图「⋯」：储藏改动（输入说明）。
    GitStash,
    /// 提交视图「⋯」：对第几条储藏做的事。
    GitStashOp(crate::app::git::branch::StashOp, usize),
    /// 提交视图的「⋯」：在按目录排成树和平铺之间切换（排成树时打勾）。
    CommitTree,
    /// 提交视图的「⋯」：展开所有组和目录。
    CommitExpandAll,
    /// 提交视图的「⋯」：收起所有组。
    CommitCollapseAll,
    /// 提交视图的「⋯」：在只读标签中打开 Git 操作的输出。
    GitShowOutput,
    /// 撤销上一次提交（先确认）。
    CommitUndo,
    /// 复制上一次提交的提交号。
    CommitCopyOid,
    /// 「提交 ⌄」：提交以后继续推送。
    CommitPush,
    /// 「提交 ⌄」（卡片太窄、底行放不下勾选框时）和「上一次提交」的「⋯」：开关「修改上一次
    /// 提交」（开着时打勾）。
    CommitAmend,
    /// 文件树工具栏的「⋯」：刷新目录。
    TreeRefresh,
    /// 文件树工具栏的「⋯」：折叠所有目录。
    TreeCollapseAll,
    /// 文件树工具栏的「⋯」：定位当前文件。
    TreeLocate,
    SelectAll,
    /// Markdown 预览：把光标放到右键点中的字在原文中的位置，切回文字（只看预览时改为分栏）。
    RevealSource,
    /// PDF：给选中的字加高亮。
    PdfHighlight,
    /// PDF：给选中的字加下划线。
    PdfUnderline,
    /// PDF：给选中的字加删除线。
    PdfStrikeOut,
    /// PDF：给选中的字加高亮并写注释。
    PdfComment,
    /// PDF：改写右键点中的批注的注释。
    PdfEditNote,
    /// PDF：删除右键点中的批注。
    PdfRemoveNote,
    /// PDF：下载 PDFium（没有 PDFium 时 PDF 只能预览）。
    PdfInstallEngine,
    /// 终端：查找。
    TerminalFind,
    /// 终端：清屏和回滚缓冲。
    TerminalClear,
    /// 终端：最大化面板（最大化时打勾）。
    TerminalMaximize,
    /// 把终端面板停靠到另一边。
    TerminalDock,
    NewTerminal,
    /// 终端标签：改名字。
    RenameTerminal,
    CloseTerminal,
    /// 终端：右键点着的命令块的命令行。
    CopyCommand,
    /// 终端：右键点着的命令块的整段输出。
    CopyOutput,
    /// 终端：再运行一次右键点着的命令。
    Rerun,
    /// 搜索结果：替换这一行，`true` 是文件行（替换这个文件里剩下的全部匹配），`false` 是匹配行
    /// （只替换这一处）。
    ///
    /// 替换输入展开时才有。
    ReplaceResult(bool),
    /// 搜索结果：把这一行从结果中移除，不改文件。
    DismissResult,
    /// 可编辑的预览中光标位于表格内时的表格操作：增删行和列、修改对齐、删除整张表。
    ///
    /// 操作名与表格工具栏共用，见 `view::preview::markdown` 的 `table_tools`。
    Table(&'static str),
}

/// 行尾菜单中可列出的行尾；CR 只在文件本来就是 CR 时才列出。
const LINE_ENDINGS: [LineEnding; 3] = [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr];

/// 缩进宽度菜单中列出的宽度范围，1 到 8（空格数，或一个 Tab 占的列数）。
pub const INDENT_SIZES: std::ops::RangeInclusive<usize> = 1..=8;

impl MenuCommand {
    /// 返回菜单项的标题；带数据的项（最近的目录、工作树）由调用方给出标题，这里返回空串。
    pub fn label(self) -> &'static str {
        match self {
            MenuCommand::NewFile => t!("menu-new-file"),
            MenuCommand::NewFolder => t!("menu-new-folder"),
            MenuCommand::Reveal | MenuCommand::LocationReveal => crate::system::reveal_label(),
            MenuCommand::OpenInTerminal => t!("menu-open-in-terminal"),
            MenuCommand::FindInFolder => t!("cmd-find-in-folder"),
            MenuCommand::CopyPath => t!("cmd-copy-path"),
            MenuCommand::CopyRelativePath => t!("cmd-copy-relative-path"),
            MenuCommand::CopyReference => t!("cmd-copy-reference"),
            MenuCommand::Rename => t!("cmd-rename-file"),
            MenuCommand::Delete => t!("cmd-delete-file"),
            MenuCommand::Close => t!("common-close"),
            MenuCommand::CloseOthers => t!("cmd-close-other-tabs"),
            MenuCommand::CloseRight => t!("menu-close-tabs-right"),
            MenuCommand::CloseLeft => t!("menu-close-tabs-left"),
            MenuCommand::OpenFiles => t!("cmd-open-selected"),
            MenuCommand::CompareFiles => t!("menu-compare-selected"),
            MenuCommand::CommitCheck(true) => t!("menu-commit-check"),
            MenuCommand::CommitCheck(false) => t!("menu-commit-uncheck"),
            MenuCommand::CloseSaved => t!("menu-close-saved-tabs"),
            MenuCommand::CloseAll => t!("cmd-close-all-tabs"),
            MenuCommand::RevealInTree => t!("menu-reveal-in-tree"),
            MenuCommand::RunEdit => t!("cmd-edit-run-configurations"),
            MenuCommand::OpenRecent(_)
            | MenuCommand::ShowTab(_)
            | MenuCommand::RunSelect(_)
            | MenuCommand::NewTerminalShell(_)
            | MenuCommand::DefaultShell(Some(_))
            | MenuCommand::Zoom(_)
            | MenuCommand::TerminalDock
            | MenuCommand::OpenRecentInNewWindow(_) => "",
            MenuCommand::DefaultShell(None) => t!("settings-terminal-shell-default"),
            MenuCommand::DefaultShellCustom => t!("settings-terminal-shell-custom"),
            MenuCommand::TerminalPosition(false) => t!("settings-terminal-position-bottom"),
            MenuCommand::TerminalPosition(true) => t!("settings-terminal-position-right"),
            MenuCommand::OptionAsMeta(i) => winc_config::OptionAsMeta::ALL
                .get(i)
                .map_or("", |&choice| crate::layout::option_as_meta_label(choice)),
            MenuCommand::NewWorktree => t!("menu-new-worktree"),
            MenuCommand::LocationNewWindow => t!("git-location-new-window"),
            MenuCommand::LocationRemove => t!("git-location-remove"),
            // 文字来自当时的分支和位置（`App::menu_label`）。
            MenuCommand::StatusPlace(_) => "",
            MenuCommand::LocationPrune => t!("git-location-prune"),
            MenuCommand::GitFetch => t!("menu-git-fetch"),
            MenuCommand::GitPull => t!("menu-git-pull"),
            MenuCommand::GitPush => t!("menu-git-push"),
            MenuCommand::GitPullRebase => t!("menu-git-pull-rebase"),
            MenuCommand::GitAddRemote => t!("git-remote-add"),
            MenuCommand::GitRemote(op, _) => op.label(),
            MenuCommand::GitContinue => t!("menu-git-continue"),
            MenuCommand::GitShowDiff => t!("menu-git-show-diff"),
            MenuCommand::GitOpenFile => t!("menu-git-open-file"),
            MenuCommand::GitRollbackFile => t!("menu-git-roll-back"),
            MenuCommand::GitRevertHunk => t!("menu-git-roll-back-change"),
            MenuCommand::FileHistory => t!("menu-git-file-history"),
            MenuCommand::LogCheckout => t!("menu-log-checkout"),
            MenuCommand::LogNewBranch => t!("git-branch-new-from"),
            MenuCommand::LogNewWorktree => t!("menu-log-new-worktree"),
            MenuCommand::LogCherryPick => t!("menu-log-cherry-pick"),
            MenuCommand::LogRevert => t!("menu-log-revert"),
            MenuCommand::LogResetSoft => t!("menu-log-reset-soft"),
            MenuCommand::LogResetMixed => t!("menu-log-reset-mixed"),
            MenuCommand::LogResetHard => t!("menu-log-reset-hard"),
            MenuCommand::LogNewTag => t!("menu-log-new-tag"),
            MenuCommand::LogTag(op, _) => op.label(),
            MenuCommand::LogInteractiveRebase => t!("menu-log-interactive-rebase"),
            MenuCommand::RebaseAction(action) => crate::app::git::rebase::action_label(action),
            MenuCommand::LogCopyOid => t!("menu-log-copy-oid"),
            MenuCommand::LogCopyShortOid => t!("menu-log-copy-short-oid"),
            MenuCommand::LogCopyMessage => t!("menu-log-copy-message"),
            MenuCommand::ToggleBlame => t!("menu-git-blame"),
            MenuCommand::GitStash => t!("menu-git-stash"),
            MenuCommand::CommitTree => t!("menu-commit-tree"),
            MenuCommand::CommitExpandAll => t!("menu-commit-expand-all"),
            MenuCommand::CommitCollapseAll => t!("menu-commit-collapse-all"),
            MenuCommand::GitShowOutput => t!("menu-git-show-output"),
            MenuCommand::CommitUndo => t!("menu-commit-undo"),
            MenuCommand::CommitCopyOid => t!("menu-log-copy-oid"),
            MenuCommand::CommitPush => t!("menu-commit-and-push"),
            MenuCommand::CommitAmend => t!("menu-commit-amend"),
            MenuCommand::TreeRefresh => t!("workspace-tree-tool-refresh"),
            MenuCommand::TreeCollapseAll => t!("menu-tree-collapse-all"),
            MenuCommand::TreeLocate => t!("menu-tree-locate"),
            MenuCommand::SelectAll => t!("cmd-select-all"),
            MenuCommand::RevealSource => t!("menu-reveal-source"),
            MenuCommand::PdfHighlight => t!("menu-pdf-highlight"),
            MenuCommand::PdfUnderline => t!("menu-pdf-underline"),
            MenuCommand::PdfStrikeOut => t!("menu-pdf-strikeout"),
            MenuCommand::PdfComment => t!("menu-pdf-comment"),
            MenuCommand::PdfEditNote => t!("menu-pdf-edit-note"),
            MenuCommand::PdfRemoveNote => t!("menu-pdf-remove-note"),
            MenuCommand::PdfInstallEngine => t!("menu-pdf-install-engine"),
            MenuCommand::TerminalFind => t!("cmd-find"),
            MenuCommand::TerminalClear => t!("menu-terminal-clear"),
            MenuCommand::TerminalMaximize => t!("menu-terminal-maximize"),
            MenuCommand::NewTerminal => t!("cmd-new-terminal"),
            MenuCommand::RenameTerminal => t!("cmd-rename-file"),
            MenuCommand::CloseTerminal => t!("menu-terminal-close"),
            MenuCommand::CopyCommand => t!("menu-terminal-copy-command"),
            MenuCommand::CopyOutput => t!("menu-terminal-copy-output"),
            MenuCommand::Rerun => t!("menu-terminal-rerun"),
            MenuCommand::ReplaceResult(true) => t!("cmd-replace-all"),
            MenuCommand::ReplaceResult(false) => t!("cmd-replace"),
            MenuCommand::DismissResult => t!("menu-search-dismiss"),
            MenuCommand::Table(op) => match op {
                "row-above" => t!("menu-table-insert-row-above"),
                "row" => t!("menu-table-insert-row-below"),
                "column-left" => t!("menu-table-insert-column-left"),
                "column" => t!("menu-table-insert-column-right"),
                "delete-row" => t!("preview-table-delete-row"),
                "delete-column" => t!("preview-table-delete-column"),
                "left" => t!("menu-table-align-left"),
                "center" => t!("menu-table-align-center"),
                "right" => t!("menu-table-align-right"),
                "delete" => t!("preview-table-delete"),
                _ => "",
            },
            MenuCommand::GitStashOp(op, _) => op.label(),
            MenuCommand::LogBranch(0) => t!("menu-log-all-branches"),
            MenuCommand::LogAuthor(0) => t!("menu-log-all-authors"),
            MenuCommand::FormBase(0) => t!("menu-git-current-branch"),
            MenuCommand::LogBranch(_) | MenuCommand::LogAuthor(_) | MenuCommand::FormBase(_) => "",
            MenuCommand::GitAbort => t!("git-operation-abort"),
            MenuCommand::GitHeading(false) => t!("menu-git-local-branches"),
            MenuCommand::GitHeading(true) => t!("menu-git-remote-branches"),
            MenuCommand::GitBranch(op, _) => op.label(),
            MenuCommand::Language(i) | MenuCommand::SkipOnSave(i) => {
                winc_syntax::languages().get(i).map_or("", |l| l.name)
            }
            MenuCommand::Whitespace(i) => winc_config::RenderWhitespace::ALL
                .get(i)
                .map_or("", |&choice| crate::layout::whitespace_label(choice)),
            MenuCommand::Locale(i) => winc_i18n::LocaleChoice::ALL
                .get(i)
                .map_or("", |&choice| crate::layout::locale_label(choice)),
            MenuCommand::FontDefault => t!("settings-font-default"),
            MenuCommand::Font(i) => winc_render::installed_fonts()
                .get(i)
                .map_or("", |family| family.name.as_str()),
            MenuCommand::OpenFolder => t!("cmd-open-folder"),
            MenuCommand::OpenWslFolder(i) => crate::system::wsl_distributions()
                .get(i)
                .map_or("", String::as_str),
            MenuCommand::CloseFolder => t!("cmd-close-folder"),
            MenuCommand::Cut => t!("cmd-cut"),
            MenuCommand::Copy => t!("cmd-copy"),
            MenuCommand::Paste => t!("cmd-paste"),
            MenuCommand::GoToDefinition => t!("cmd-go-to-definition"),
            MenuCommand::FindReferences => t!("cmd-find-references"),
            MenuCommand::AddBookmark => t!("menu-add-bookmark"),
            MenuCommand::RemoveBookmark => t!("menu-remove-bookmark"),
            MenuCommand::ListBookmarks => t!("cmd-list-bookmarks"),
            MenuCommand::Format => t!("cmd-format"),
            MenuCommand::FormatSelection => t!("cmd-format-selection"),
            MenuCommand::Fix => t!("cmd-fix"),
            MenuCommand::ToolAuto => t!("format-tool-auto"),
            MenuCommand::ToolPreset(i) => PRESETS.get(i).map_or("", |p| p.name),
            MenuCommand::ToolCustom => t!("format-tool-custom-command"),
            MenuCommand::ToolInstall => t!("format-install-link"),
            MenuCommand::ToolRemove => t!("format-tool-remove"),
            MenuCommand::IndentUsingSpaces => t!("cmd-indent-using-spaces"),
            MenuCommand::IndentUsingTabs => t!("cmd-indent-using-tabs"),
            MenuCommand::IndentSize(n) => ["1", "2", "3", "4", "5", "6", "7", "8"]
                .get(n.wrapping_sub(1))
                .copied()
                .unwrap_or(""),
            MenuCommand::ConvertToSpaces => t!("cmd-convert-indentation-to-spaces"),
            MenuCommand::ConvertToTabs => t!("cmd-convert-indentation-to-tabs"),
            MenuCommand::DetectIndentation => t!("cmd-detect-indentation"),
            MenuCommand::LineEnding(eol) => eol.label(),
            MenuCommand::ReopenEncoding(i) | MenuCommand::SaveEncoding(i) => {
                CHOICES.get(i).map_or("", |(_, label)| label)
            }
            MenuCommand::UseLanguage(i) => winc_syntax::languages().get(i).map_or("", |l| l.name),
            MenuCommand::UsePlainText => winc_syntax::detect("").name,
            MenuCommand::DetectLanguage => t!("menu-detect-language"),
        }
    }

    /// 返回键位表中对应的命令，菜单右侧据此显示该命令的快捷键；没有对应命令时返回 `None`。
    pub fn action(self) -> Option<Action> {
        Some(match self {
            MenuCommand::Reveal => Action::RevealInFinder,
            MenuCommand::OpenInTerminal => Action::OpenInTerminal,
            MenuCommand::FindInFolder => Action::FindInFolder,
            MenuCommand::CopyPath => Action::CopyPath,
            MenuCommand::CopyRelativePath => Action::CopyRelativePath,
            MenuCommand::CopyReference => Action::CopyReference,
            MenuCommand::Rename => Action::RenameFile,
            MenuCommand::Delete => Action::DeleteFile,
            MenuCommand::Close => Action::CloseTab,
            MenuCommand::CloseOthers => Action::CloseOtherTabs,
            MenuCommand::CloseAll => Action::CloseAllTabs,
            MenuCommand::OpenFolder => Action::OpenFolder,
            MenuCommand::CloseFolder => Action::CloseFolder,
            MenuCommand::Cut => Action::Cut,
            MenuCommand::Copy => Action::Copy,
            MenuCommand::Paste => Action::Paste,
            MenuCommand::SelectAll => Action::SelectAll,
            MenuCommand::TerminalFind => Action::FindInTerminal,
            MenuCommand::TerminalClear => Action::ClearTerminal,
            MenuCommand::TerminalMaximize => Action::MaximizeTerminal,
            MenuCommand::TerminalDock => Action::ToggleTerminalPosition,
            MenuCommand::NewTerminal => Action::NewTerminal,
            MenuCommand::RenameTerminal => Action::RenameTerminal,
            MenuCommand::CloseTerminal => Action::CloseTerminal,
            MenuCommand::GitRevertHunk => Action::GitRevertChange,
            MenuCommand::GoToDefinition => Action::GoToDefinition,
            MenuCommand::FindReferences => Action::FindReferences,
            MenuCommand::AddBookmark | MenuCommand::RemoveBookmark => Action::ToggleBookmark,
            MenuCommand::ListBookmarks => Action::ListBookmarks,
            MenuCommand::Format => Action::Format,
            MenuCommand::FormatSelection => Action::FormatSelection,
            MenuCommand::Fix => Action::Fix,
            MenuCommand::NewFile => Action::NewFile,
            MenuCommand::IndentUsingSpaces => Action::IndentUsingSpaces,
            MenuCommand::IndentUsingTabs => Action::IndentUsingTabs,
            MenuCommand::ConvertToSpaces => Action::ConvertIndentationToSpaces,
            MenuCommand::ConvertToTabs => Action::ConvertIndentationToTabs,
            MenuCommand::DetectIndentation => Action::DetectIndentation,
            _ => return None,
        })
    }

    /// 返回菜单右侧显示的快捷键：在此菜单弹出位置可用的单个按键。
    ///
    /// 两步组合键无法写入系统菜单，因此不返回。同一个键在这里先给了别的命令时也不返回：
    /// 搜索面板里 `⌥⌘C` 是「区分大小写」开关，写在「复制路径」旁边时按下它不会生效。
    pub fn shortcut(self, context: Context) -> Option<Stroke> {
        // 右键菜单中的「新建文件…」是在点中的目录里新建，与 `⌘N` 新建未命名文档不同，
        // 因此不显示快捷键。
        if self == MenuCommand::NewFile {
            return None;
        }
        let action = self.action()?;
        let stroke = keymap::single_stroke(action, context)?;
        match keymap::lookup(keymap::current(), stroke, None, context) {
            keymap::Lookup::Run(actions) if actions.first() == Some(&action) => Some(stroke),
            _ => None,
        }
    }
}

/// 菜单所作用的目标：右键点击的位置，或弹出菜单的按钮。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// 文件树中的一项；`None` 表示空白处，即根目录。
    Tree(Option<PathBuf>),
    /// 标签栏中的一个标签。
    Tab(DocumentId),
    /// 编辑区中当前标签的文字。
    Editor(DocumentId),
    /// Markdown 预览（分栏右边，或只看预览时整个编辑区）。
    Markdown,
    /// PDF 预览（能选字时）。
    Pdf,
    /// 差异标签里的文字：`true` 是右侧（工作区，可以编辑），`false` 是左侧。
    Diff(DocumentId, bool),
    /// 侧栏顶部的项目名。
    Project,
    /// 设置的格式化页中格式化工具（修复工具）一组下面的「添加语言…」。
    AddLanguage(Task),
    /// 设置的格式化页中的「排除的语言」。
    SkipOnSave,
    /// 设置的外观页中的「语言」。
    Locale,
    /// 设置的编辑器页中的「空白符」。
    Whitespace,
    /// 设置的外观页中的「界面缩放」。
    Zoom,
    /// 设置的终端页中的「默认 Shell」。
    DefaultShell,
    /// 设置的终端页中的「位置」。
    TerminalPosition,
    /// 设置的终端页中的「Meta 键」。
    OptionAsMeta,
    /// 设置中字体一行的下拉按钮。
    Font(Setting),
    /// 设置的格式化页中一种语言的格式化（修复）工具。
    Tool(Task, &'static str),
    /// 状态栏中一个文档的缩进。
    Indent(DocumentId),
    /// 状态栏中一个文档的行尾。
    LineEnding(DocumentId),
    /// 状态栏中一个文档的编码。
    Encoding(DocumentId),
    /// 状态栏中一个文档的语言。
    Language(DocumentId),
    /// 分支选择器里第几个分支（[`crate::app::git::GitState::branches`] 的下标）的操作。
    BranchOps(usize),
    /// 位置选择器里一个位置的操作：目录，以及它在当前仓库的工作树列表里的下标。
    Location(PathBuf, Option<usize>),
    /// 提交视图标题行的「⋯」。
    CommitMore,
    /// 提交视图「提交 ⌄」拆分按钮的箭头。
    CommitButton,
    /// 提交视图底部「上一次提交」那一栏的「⋯」。
    LastCommit,
    /// 文件树工具栏的「⋯」：收起的各个工具。
    TreeTools,
    /// 标签栏最右端「⌄」弹出的标签列表。
    TabList,
    /// 终端面板标签栏上的运行配置列表。
    RunConfigs,
    /// 终端面板标签栏上可选的 shell。
    TerminalShells,
    /// 提交视图改动列表里的一个文件。
    CommitFile(PathBuf),
    /// 日志标题栏的「分支」下拉。
    LogBranch,
    /// 新建工作树对话框的「基于」。
    FormBase,
    /// 交互式变基列表第几行的处理方式。
    RebaseAction(usize),
    /// 提交视图里第几条储藏的菜单：应用、应用并删除、删除。
    StashRow(usize),
    /// 提交视图里储藏太多时「还有 N 条」弹出的菜单：每条储藏一个子菜单。
    StashList,
    /// 状态栏的位置菜单：弹出时的各个位置，菜单项按下标对应。
    StatusPlaces(Vec<Place>),
    /// 日志标题栏的「作者」下拉。
    LogAuthor,
    /// 日志里的一个提交（完整的提交号）。
    LogCommit(String),
    /// 日志详情（或比较）里改动的一个文件：文件列表里的下标和文件的路径。
    LogFile(usize, PathBuf),
    /// 终端标签栏上的一个标签（编号）。
    TerminalTab(u16),
    /// 终端面板里正在看的标签，以及右键点着的命令块（编号；没点在运行过的块上时为 `None`）。
    Terminal(Option<u32>),
    /// 全局搜索结果里的一行：文件的路径和该文件中第几处匹配（`None` 是文件行），和
    /// `GlobalSearch::selected` 的记法相同。
    SearchResult(PathBuf, Option<usize>),
    /// 状态栏路径一段的目录菜单（没有系统菜单时，各行是文件和目录，见 `PopupRow::Entry`）。
    Folder,
}

/// 菜单中的一行：一条命令（带是否可用），分隔线，或悬停时展开的子菜单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    /// 一条命令，布尔值表示是否可用。
    Item(MenuCommand, bool),
    /// 按住 `⌥` 时代替前一项显示的命令（系统菜单的备选项），布尔值表示是否可用。
    Alternate(MenuCommand, bool),
    /// 分隔线。
    Separator,
    /// 子菜单，参数为标题和子项。
    Submenu(String, Vec<MenuEntry>),
}

impl MenuEntry {
    /// 菜单 `entries` 里第 `tag` 个命令：按显示顺序从 1 起数，子菜单里的也数进去，分隔线和子
    /// 菜单的标题不算；0 和超出的编号是「没有」。
    ///
    /// 系统菜单上每一项的编号（`NSMenuItem` 的 tag）就是该序号（`App::prepare_menu` 按同样
    /// 的顺序编）。选中以后按弹出时记下的这份菜单（[`ContextMenu`]）找回命令，编号只在这一个
    /// 菜单里有意义。
    pub fn command_at(entries: &[MenuEntry], tag: isize) -> Option<MenuCommand> {
        fn walk(entries: &[MenuEntry], seen: &mut isize, tag: isize) -> Option<MenuCommand> {
            for entry in entries {
                match entry {
                    MenuEntry::Item(command, _) | MenuEntry::Alternate(command, _) => {
                        *seen += 1;
                        if *seen == tag {
                            return Some(*command);
                        }
                    }
                    MenuEntry::Submenu(_, items) => {
                        if let Some(command) = walk(items, seen, tag) {
                            return Some(command);
                        }
                    }
                    MenuEntry::Separator => {}
                }
            }
            None
        }
        walk(entries, &mut 0, tag)
    }

    /// `entries`（含子菜单）中是否包含可用的 `command`。
    fn offers(entries: &[MenuEntry], command: MenuCommand) -> bool {
        entries.iter().any(|entry| match entry {
            MenuEntry::Item(c, enabled) | MenuEntry::Alternate(c, enabled) => {
                *c == command && *enabled
            }
            MenuEntry::Submenu(_, items) => MenuEntry::offers(items, command),
            MenuEntry::Separator => false,
        })
    }
}

/// 当前弹出的菜单：作用的目标和菜单项。
///
/// 菜单项是否打勾、显示什么文字都根据该目标计算。
#[derive(Debug, Clone)]
pub struct ContextMenu {
    /// 菜单作用的目标。
    pub target: MenuTarget,
    /// 菜单项。
    pub entries: Vec<MenuEntry>,
    /// 弹出菜单时的分支列表：菜单项中的分支下标按此列表解释。
    ///
    /// 菜单打开期间 Git 状态可能刷新，最新的列表与菜单项不再对应。只有分支、日志的分支筛选和
    /// 「基于」菜单记录此列表，其余菜单为空。
    pub branches: Vec<winc_editor::git::Branch>,
    /// 弹出菜单时的储藏列表：菜单项中的储藏下标按此列表解释，记录规则同 `branches`。
    pub stashes: Vec<winc_editor::git::Stash>,
    /// 没有系统菜单时在窗口里画出来的菜单；用系统菜单时为 `None`。
    pub popup: Option<PopupMenu>,
}
