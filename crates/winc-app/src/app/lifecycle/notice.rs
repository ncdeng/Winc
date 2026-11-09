//! 提示条：编辑区右上角的提示、状态栏的临时消息，以及提示条上的按钮。

use winc_i18n::t;

use super::*;

/// 点击提示条按钮后执行的操作。
#[derive(Debug, Clone, PartialEq)]
pub enum NoticeAction {
    /// 「查看输出」：在只读标签中打开格式化工具的完整输出。
    ShowOutput(String),
    /// 「安装方法」：在只读标签中打开各个格式化工具的安装说明。
    ///
    /// `dir` 是文件所在的目录。
    Install { tools: Vec<String>, dir: PathBuf },
    /// 「去设置」：打开设置的格式化一页，在那里指定工具命令。
    FormatSettings,
    /// 「查看输出」：在只读标签中打开 Git 操作的输出。
    GitOutput,
    /// 「下载」：下载 PDFium，之后 PDF 可以选中文字、加批注。
    InstallPdfEngine,
}

/// 到期后正在淡出的提示条。
pub struct NoticeGhost {
    /// 提示的消息。
    pub message: String,
    /// 按钮上的文字；没有按钮时为 `None`。
    pub label: Option<&'static str>,
    /// 淡出的动画时钟。
    pub clock: Clock,
}

impl NoticeAction {
    /// 按钮上显示的文字。
    pub fn label(&self) -> &'static str {
        match self {
            NoticeAction::ShowOutput(_) | NoticeAction::GitOutput => {
                t!("workspace-notice-show-output")
            }
            NoticeAction::Install { .. } => t!("workspace-notice-install"),
            NoticeAction::FormatSettings => t!("workspace-notice-open-settings"),
            NoticeAction::InstallPdfEngine => t!("preview-pdf-engine-install"),
        }
    }
}

/// 状态栏消息（由 [`App::flash`] 设置）显示的时间（4 秒）。
pub(in crate::app) const FLASH_DURATION: Duration = Duration::from_secs(4);

/// 编辑区右上角提示条显示的时间（5 秒）。
const NOTICE_DURATION: Duration = Duration::from_secs(5);

/// 带按钮的提示条显示的时间（10 秒）；比普通提示更长，便于用户来得及点击。
const NOTICE_ACTION_DURATION: Duration = Duration::from_secs(10);

impl App {
    /// 在状态栏显示一条临时消息，显示时间为 [`FLASH_DURATION`]，用于操作结果和一般提示。
    ///
    /// 隐藏状态栏时改用右上角的提示（[`App::notify`]），否则这条消息看不到。
    pub(crate) fn flash(&mut self, message: impl Into<String>) {
        if !self.status_bar_shown() {
            self.notify(message);
            return;
        }
        self.status_message = Some(message.into());
        self.status_until = Some(Instant::now() + FLASH_DURATION);
        self.request_redraw();
    }

    /// 清除 [`App::flash`] 显示的消息：状态已经变化，消息不再成立时调用。
    ///
    /// 隐藏状态栏时消息显示在提示条上，清除提示条。
    pub(crate) fn clear_flash(&mut self) {
        self.status_message = None;
        self.status_until = None;
        if !self.status_bar_shown() {
            self.clear_notice();
        }
        self.request_redraw();
    }

    /// 在编辑区右上角显示一条提示，显示时间为 [`NOTICE_DURATION`]，用于出错、冲突、未保存等
    /// 需要用户注意的情况。
    ///
    /// 出错消息统一写成「无法……：原因」。
    pub(in crate::app) fn notify(&mut self, message: impl Into<String>) {
        // 换一条消息时重新播放入场动画；同一条消息再来一次只延长显示时间，不重新播放。
        let message = message.into();
        if self.notice.as_deref() != Some(message.as_str()) {
            self.notice_in = self.clock(POPUP_IN);
        }
        self.notice = Some(message);
        self.notice_until = Some(Instant::now() + NOTICE_DURATION);
        self.notice_action = None;
        self.notice_out = None;
        self.request_redraw();
    }

    /// 返回 `shown`，即系统的提示框或面板是否弹出了。
    ///
    /// 没有弹出（窗口上已挂着别的提示框或面板）时提示用户先关闭它，
    /// 否则这次操作既不执行也不说明原因。
    pub(in crate::app) fn dialog_shown(&mut self, shown: bool) -> bool {
        if !shown {
            self.flash(t!("common-close-dialog-first"));
        }
        shown
    }

    /// 显示带一个按钮的提示（例如「查看输出」），显示时间为 [`NOTICE_ACTION_DURATION`]。
    pub(in crate::app) fn notify_with(&mut self, message: impl Into<String>, action: NoticeAction) {
        self.notify(message);
        self.notice_until = Some(Instant::now() + NOTICE_ACTION_DURATION);
        self.notice_action = Some(action);
    }

    /// 收起提示条，返回它的文字和按钮操作；清除提示的各处都经过这里，避免漏掉某个字段。
    pub(super) fn clear_notice(&mut self) -> (Option<String>, Option<NoticeAction>) {
        self.notice_until = None;
        self.notice_button = None;
        self.notice_card = None;
        self.notice_in = None;
        (self.notice.take(), self.notice_action.take())
    }

    /// 执行提示条上按钮对应的操作，并收起提示。
    pub(in crate::app) fn run_notice_action(&mut self) {
        let Some(action) = self.notice_action.take() else {
            return;
        };
        self.clear_notice();
        match action {
            NoticeAction::ShowOutput(text) => {
                self.editor
                    .show_output(t!("workspace-format-output-title"), &text);
                self.focus = Focus::Editor;
                self.relayout();
                self.ensure_caret_visible();
            }
            NoticeAction::Install { tools, dir } => self.show_install_guide(&tools, &dir),
            NoticeAction::FormatSettings => {
                self.open_settings();
                self.settings_show_page(SettingsPage::Format);
            }
            NoticeAction::GitOutput => self.show_git_output(),
            NoticeAction::InstallPdfEngine => self.install_pdf_engine(),
        }
        self.request_redraw();
    }
}
