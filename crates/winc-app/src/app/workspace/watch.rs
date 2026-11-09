//! 监视已打开的目录：其他程序新建、删除、改名或改写文件时，增量更新界面。
//!
//! 受影响的部分包括文件树、`⌘P` 面板的文件表、已打开的文档，以及全局搜索结果中涉及的文件。
//! 各部分只处理变化涉及的路径，不需要切换窗口，也不需要重新遍历整个目录。
//!
//! 收到系统通知后立即唤醒界面，但等待 [`SETTLE`] 之后才处理：切换分支、解压、格式化整个
//! 项目等操作产生的密集改动合并为一批，只更新一次。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use winc_editor::{
    Changes, DirListing, DiskChange, DiskRead, DiskSync, DiskTaken, EncodingRead, Watcher,
};

use super::*;
use crate::app::tickets::Tickets;

/// 收到文件变化通知后到开始处理之前的等待时间。
///
/// 取 100 ms（经验值）：该值越大，合并的连续通知越多，界面反映变化的时间越晚。
const SETTLE: Duration = Duration::from_millis(100);

/// 后台返回的一次结果：文档、读取代号、结果。
type ReadBack = (DocumentId, u64, DiskBack);

/// 后台对一个文档完成的一步磁盘检查。
pub(crate) enum DiskBack {
    /// 读到的文件元数据（文件不存在或无法访问时为 `None`），交给 `Document::check_disk_with`。
    Checked(Option<std::fs::Metadata>),
    /// 读到的文件内容（末尾追加的部分或整个文件），交给 `Document::apply_reread`。
    Read(DiskRead),
    /// 按用户选择的编码读到的文件内容，交给 `Document::finish_reopen_with_encoding`。
    Reencoded(EncodingRead),
    /// 「使用磁盘版本」读到的内容和比较结果，交给 `Document::finish_take_disk_version`。
    Taken(DiskTaken),
}

/// 在后台检查、重读被其他程序修改过的文件。
///
/// 界面线程不访问磁盘：网络盘、外接硬盘刚唤醒时，读取元数据都可能阻塞很久；整个重读（读文件、
/// 解码、建 rope）和读取末尾追加的大段内容都与文件大小成正比。先在后台读元数据，界面线程据此
/// 判断文件没有变化、存在冲突还是需要读取；需要读取时再交给后台，读好之后核对文档期间没有变化
/// 才换上。
pub(crate) struct Rereads {
    /// 各文档最近一次发出的检查或重读：同一个文件连续被修改时只采用最后一次。
    pub(in crate::app) tickets: Tickets<DocumentId>,
    /// 各文档最近一次由用户发起的读取：「通过编码重新打开」和「使用磁盘版本」。
    ///
    /// 与 `tickets` 分开：确认框关闭后窗口重新获得焦点，会对所有文档发出检查，共用读取代号时
    /// 这次检查会使用户发起的读取作废。
    pub(in crate::app) reencodes: Tickets<DocumentId>,
    /// 后台检查、重读结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    pub(in crate::app) results: (mpsc::Sender<ReadBack>, mpsc::Receiver<ReadBack>),
}

impl Default for Rereads {
    fn default() -> Self {
        Self {
            tickets: Tickets::default(),
            reencodes: Tickets::default(),
            results: mpsc::channel(),
        }
    }
}

/// 后台重读的一批目录：各目录的读取代号（按读取顺序）与目录内容。
type ScanBack = (Vec<u64>, DirListing);

/// 在后台重读的、文件树中被其他程序修改过的目录。
///
/// 界面线程只挑出需要读取的目录，读好后再换进缓存。
pub(crate) struct TreeScans {
    /// 各目录最近一次发出的重读：同一个目录连续被修改时只采用最后一次，不同目录互不影响。
    pub(in crate::app) tickets: Tickets<PathBuf>,
    /// 后台重读结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    pub(in crate::app) results: (mpsc::Sender<ScanBack>, mpsc::Receiver<ScanBack>),
}

impl Default for TreeScans {
    fn default() -> Self {
        Self {
            tickets: Tickets::default(),
            results: mpsc::channel(),
        }
    }
}

impl App {
    /// 开始监视已打开的目录；目录改变时改为监视新目录。
    ///
    /// 没有事件循环（测试中）时不监视，测试直接把变化传给 [`App::apply_changes`]。
    pub(in crate::app) fn watch_folder(&mut self) {
        let root = self.editor.root().map(Path::to_path_buf);
        if self.watcher.as_ref().map(Watcher::root) == root.as_deref() {
            return;
        }
        self.watcher = None;
        self.changes_due = None;
        self.git_folder_changed();
        let (Some(root), Some(proxy)) = (root, self.proxy.clone()) else {
            return;
        };
        // 监视启动失败（例如超出系统允许的监视数量）时 `watcher` 为 `None`，
        // 由 `check_disk` 在窗口重新获得焦点时检查文档并重读文件树。
        self.watcher = Watcher::start(&root, move || {
            let _ = proxy.send_event(UserEvent::FilesChanged);
        })
        .ok();
        self.index_folder();
    }

    /// 发行版内的监视程序监视过后退出了、发行版已恢复时重新启动监视；开始监视后程序要求重新扫描。
    pub(in crate::app) fn restart_exited_watcher(&mut self) {
        let root = (self.watcher.as_ref())
            .filter(|watcher| watcher.exited())
            .map(|watcher| watcher.root().to_path_buf());
        let (Some(root), Some(proxy)) = (root, self.proxy.clone()) else {
            return;
        };
        if !winc_editor::wsl::reachable(&root) {
            return;
        }
        let wake = move || {
            let _ = proxy.send_event(UserEvent::FilesChanged);
        };
        if let Ok(watcher) = Watcher::start(&root, wake) {
            self.watcher = Some(watcher);
        }
    }

    /// 记录系统发来的目录变化通知，并在 [`SETTLE`] 之后统一处理。
    ///
    /// 已有等待中的批次时不重新计时。
    pub(in crate::app) fn files_changed(&mut self) {
        if self.changes_due.is_none() {
            self.changes_due = Some(Instant::now() + SETTLE);
        }
    }

    /// 到达处理时间后处理已累积的变化，返回下一次需要检查的时刻；没有待处理的批次时返回 `None`。
    pub(in crate::app) fn changes_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.changes_due?;
        if now < due {
            return Some(due);
        }
        self.changes_due = None;
        if self.watcher.as_ref().is_some_and(Watcher::take_limit) {
            let name = self.editor.root().map(winc_editor::display_name);
            let name = name.as_deref().unwrap_or_default();
            self.notify(winc_i18n::t!("workspace-watch-limit", name = name));
        }
        if let Some(changes) = self.watcher.as_ref().map(Watcher::take) {
            self.apply_changes(changes);
        }
        None
    }

    /// 按 `changes` 更新界面。
    ///
    /// - 文件树：重读受影响的目录。
    /// - `⌘P` 面板：在后台更新文件表。
    /// - 已打开的文档：从磁盘重新读取；有未保存修改的文档提示冲突。
    /// - 全局搜索结果：重新搜索变化涉及的文件。
    pub(in crate::app) fn apply_changes(&mut self, changes: Changes) {
        if changes.is_empty() {
            return;
        }

        if let Some(ws) = &self.editor.workspace {
            let scan = ws.scan((!changes.rescan).then_some(changes.paths.as_slice()));
            if !scan.dirs().is_empty() {
                let tickets = scan
                    .dirs()
                    .iter()
                    .map(|dir| self.tree_scans.tickets.issue(dir.clone()))
                    .collect();
                let sender = self.tree_scans.results.0.clone();
                self.spawn_then(UserEvent::TreeScanned, move || {
                    let _ = sender.send((tickets, scan.run()));
                });
            }
        }

        self.run_configs_changed(changes.rescan, &changes.paths);
        self.sync_documents(|path| changes.rescan || changes.paths.iter().any(|p| p == path));
        // 全局搜索只重搜变化涉及的文件：打开的目录较大（例如整个主目录）时，其他程序持续写入，
        // 每次都全部重搜的开销过大。只有系统报告丢失通知（`rescan`）时才全部重搜。
        if changes.rescan {
            self.files_changed_under_results();
        } else {
            self.results_touched(&changes.paths);
        }
        self.git_paths_changed(&changes.paths, changes.rescan, changes.git);
        self.picker.apply(changes);

        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 把已打开的文档中 `touched` 返回 `true` 的那些与磁盘内容同步。
    ///
    /// 只在后台读取元数据（见 [`Rereads`]），读好后由 [`App::poll_rereads`] 判断：没有未保存修改
    /// 的文档在后台重新读取；有未保存修改的文档标记为冲突，当前文档由冲突卡片提示（见
    /// `conflict.rs`），其余标签在右上角提示一次。窗口重新获得焦点时，文件可能在未被监视的位置
    /// 被修改，因此对所有文档调用。
    pub(in crate::app) fn sync_documents(&mut self, touched: impl Fn(&Path) -> bool) {
        let mut checks = Vec::new();
        for doc in &self.editor.documents {
            // 正在后台保存的文档不同步：写出的文件来自本程序，保存完成后再检查
            // （见 `background_save.rs`）。
            let Some(path) = doc.path.as_deref().filter(|p| touched(p)) else {
                continue;
            };
            if !self.saves.running.contains(&doc.id) {
                let ticket = self.rereads.tickets.issue(doc.id);
                checks.push((doc.id, ticket, path.to_path_buf()));
            }
        }
        if checks.is_empty() {
            return;
        }
        let sender = self.rereads.results.0.clone();
        self.spawn_then(UserEvent::Reread, move || {
            for (id, ticket, path) in checks {
                let metadata = std::fs::metadata(&path).ok();
                let _ = sender.send((id, ticket, DiskBack::Checked(metadata)));
            }
        });
    }

    /// 收取后台重读完成的目录（见 [`TreeScans`]）并换进文件树；之后又发出过重读的目录不换。
    pub(in crate::app) fn poll_tree_scans(&mut self) {
        let mut changed = false;
        while let Ok((tickets, mut listing)) = self.tree_scans.results.1.try_recv() {
            let mut tickets = tickets.into_iter();
            let scans = &mut self.tree_scans.tickets;
            listing.retain(|dir| tickets.next().is_some_and(|t| scans.redeem(dir, t)));
            if let Some(ws) = &mut self.editor.workspace {
                changed |= ws.apply_scan(listing);
            }
        }
        if changed {
            self.tree_motion = None;
            self.clamp_tree_scroll();
            self.request_redraw();
        }
    }

    /// 收取后台检查、重读的结果（见 [`Rereads`]）。
    ///
    /// 只采用各文档最近一次发出的读取。读元数据后得知需要读取文件内容时再交给后台；读到的内容
    /// 换进文档，期间文档被编辑过（或者换了路径）就不换，改为再检查一次磁盘，此时有未保存修改的
    /// 文档会得出冲突。
    pub(in crate::app) fn poll_rereads(&mut self) {
        let mut report = DiskReport::default();
        let mut recheck = Vec::new();
        let mut rereads = Vec::new();
        let mut reencoded = Vec::new();
        let mut taken = Vec::new();
        let active = self.editor.active().map(|d| d.id);

        while let Ok((id, ticket, back)) = self.rereads.results.1.try_recv() {
            let tickets = match back {
                DiskBack::Reencoded(_) | DiskBack::Taken(_) => &mut self.rereads.reencodes,
                _ => &mut self.rereads.tickets,
            };
            if !tickets.redeem(&id, ticket) || self.saves.running.contains(&id) {
                continue;
            }
            let Some(doc) = self.editor.document_mut(id) else {
                continue;
            };
            let read = match back {
                DiskBack::Checked(metadata) => {
                    match doc.check_disk_with(metadata.as_ref()) {
                        DiskSync::Reread(reread) => rereads.push((id, reread)),
                        DiskSync::Done(DiskChange::Reloaded) => {
                            report.reloaded.push(doc.name().into_owned());
                        }
                        DiskSync::Done(DiskChange::Conflict) => {
                            report.marked = true;
                            if active != Some(id) {
                                report.conflicts.push(doc.name().into_owned());
                            }
                        }
                        DiskSync::Done(_) => {}
                    }
                    continue;
                }
                DiskBack::Read(read) => read,
                DiskBack::Reencoded(read) => {
                    reencoded.push((id, read));
                    continue;
                }
                DiskBack::Taken(read) => {
                    taken.push((id, read));
                    continue;
                }
            };
            if !read.applies_to(doc) {
                recheck.extend(doc.path.clone());
                continue;
            }
            // 光标在最后一行且没有选区时，文件末尾追加内容后光标跟随到新的末尾（跟随日志）；
            // 光标在其他位置时不移动。
            let at_end = doc.selection.is_empty()
                && doc.text().line_of(doc.selection.head) + 1 == doc.text().line_count();
            match doc.apply_reread(read) {
                Ok(DiskChange::Reloaded) => report.reloaded.push(doc.name().into_owned()),
                Ok(DiskChange::Appended) => {
                    report.changed = true;
                    if at_end {
                        doc.selection = Selection::point(doc.text().end_pos());
                        report.follow |= active == Some(id);
                    }
                }
                _ => {}
            }
        }

        for (id, reread) in rereads {
            let ticket = self.rereads.tickets.issue(id);
            let sender = self.rereads.results.0.clone();
            self.spawn_then(UserEvent::Reread, move || {
                let _ = sender.send((id, ticket, DiskBack::Read(reread.read())));
            });
        }
        for (id, read) in reencoded {
            self.finish_reopen_with_encoding(id, read);
        }
        for (id, read) in taken {
            self.finish_take_disk_version(id, read);
        }
        self.report_disk(report);
        if !recheck.is_empty() {
            self.sync_documents(|path| recheck.iter().any(|p| p == path));
        }
    }

    /// 按 `report` 重画、跟随到末尾，并提示重新加载或冲突的文档。
    fn report_disk(&mut self, report: DiskReport) {
        if !report.reloaded.is_empty() || report.marked || report.changed || report.follow {
            self.request_redraw();
            self.sticky_cache = None;
        }
        if report.follow {
            self.relayout();
            self.ensure_caret_visible();
        }
        if !report.conflicts.is_empty() {
            let names: Vec<String> = report
                .conflicts
                .iter()
                .map(|n| winc_i18n::t!("workspace-quoted-name", name = n.as_str()))
                .collect();
            let names = names.join(winc_i18n::t!("workspace-list-separator"));
            self.notify(winc_i18n::t!("workspace-disk-conflicts", names = names));
        } else if !report.reloaded.is_empty() {
            let names = report
                .reloaded
                .join(winc_i18n::t!("workspace-list-separator"));
            self.flash(winc_i18n::t!("workspace-reloaded", names = names));
        }
    }
}

/// 一批检查、重读结果中需要告知用户的内容。
#[derive(Default)]
struct DiskReport {
    /// 重新加载了的文档名。
    reloaded: Vec<String>,
    /// 标记为冲突的非当前文档名。
    conflicts: Vec<String>,
    /// 有文档标记为冲突（包括当前文档）。
    marked: bool,
    /// 有文档末尾接上了内容。
    changed: bool,
    /// 当前文档的光标跟随到了新的末尾。
    follow: bool,
}
