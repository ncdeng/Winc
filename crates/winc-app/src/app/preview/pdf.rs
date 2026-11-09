//! PDF 预览的状态：渲染线程、各页大小、渲染好的页和滚动位置，以及选字和批注。
//!
//! 每个正在显示的 PDF 有一个渲染线程：文档在该线程中打开（系统的 PDF 对象不能跨线程使用），按
//! 请求逐页渲染。绘制时只请求视口内和视口附近的页（`App::request_pdf_pages`）：不再需要的页从
//! 缓存中移除，线程中排队但已不需要的请求跳过，内存只占这几页的位图。切换标签或文件被修改时
//! 结束线程。
//!
//! 文档能读取文字时（Windows 上有 PDFium，见 `winc_preview::pdf`），线程还读出这几页的文字和
//! 批注：拖动鼠标按字选中，右键给选中的字加高亮、下划线、删除线或写批注，批注随即保存进 PDF
//! 文件。保存引起的文件变化不重新打开文档（文档在线程中已经是修改后的样子），只重画改过的页。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

use winc_editor::{DocumentId, Unsupported};
use winc_preview::image::Pixels;
use winc_preview::pdf::{Annotation, AnnotationKind, Document, PageRect, PageSize, TextChar};
use winc_render::Rect;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, Focus, PromptKind, UserEvent};
use crate::field::TextField;

/// 方向键每次滚动的距离（逻辑像素）：约为正文三行的高度。
const ARROW_STEP: f32 = 48.0;

/// 渲染线程的请求。
enum Job {
    /// 渲染一页：页号（从 0 起）和位图大小（物理像素）。
    Render {
        page: usize,
        width: u32,
        height: u32,
    },
    /// 读取一页的文字和批注。
    Read(usize),
    /// 按顺序修改各页的批注（页号，修改），全部成功后保存到文件。
    Edit(Vec<(usize, Edit)>),
}

/// 对一页批注的修改。
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    /// 给序号在范围内的字（[`PageText::chars`] 中的序号）加批注。
    Add {
        kind: AnnotationKind,
        chars: Range<usize>,
        comment: String,
    },
    /// 改写一个批注（[`Annotation::index`]）的注释文字。
    Comment { index: usize, comment: String },
    /// 删除一个批注（[`Annotation::index`]）。
    Remove(usize),
}

/// 渲染线程送回的结果，带有打开时的序号：序号与当前序号不同时说明文档已经更换，结果丢弃。
enum Done {
    /// 打开的结果：各页大小和能否读写文字、批注，或失败的原因。
    Opened(u64, Result<(Vec<PageSize>, bool), String>),
    /// 渲染好的一页：页号和位图；渲染失败时位图为 `None`。
    Rendered(u64, usize, Option<Arc<Pixels>>),
    /// 读好的一页文字和批注；读取失败时为 `None`。
    Read(u64, usize, Option<Arc<PageText>>),
    /// 修改批注的结果：改过的页，以及保存后文件的修改时间或失败的原因。
    Edited(u64, Vec<usize>, Result<Option<SystemTime>, String>),
}

/// 一页的文字和批注。
#[derive(Debug, Default)]
pub(crate) struct PageText {
    /// 页上的字，按阅读顺序。
    pub chars: Vec<TextChar>,
    /// 页上的批注。
    pub notes: Vec<Annotation>,
}

/// 文字中的位置：第 `page` 页第 `index` 个字之前（[`PageText::chars`] 中的序号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Caret {
    /// 页号（从 0 起）。
    pub page: usize,
    /// 字的序号。
    pub index: usize,
}

/// 输入条里正在写的批注是给谁的。
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    /// 给选中的字加带注释的高亮：各页的（页号，字的序号范围）。
    Highlight(Vec<(usize, Range<usize>)>),
    /// 改写第 `page` 页上一个批注（[`Annotation::index`]）的注释。
    Comment { page: usize, index: usize },
}

/// 当前标签的 PDF 预览。
pub(crate) struct PdfPreview {
    /// 预览的文档及其版本号（`revision`）；文档或版本改变时重新打开。
    doc: Option<(DocumentId, u64)>,
    /// 当前打开的序号，每次重新打开加一，用来丢弃旧线程送回的结果。
    generation: u64,
    /// 向渲染线程发请求；丢弃时线程结束，文档随之关闭。
    requests: Option<mpsc::Sender<Job>>,
    /// 当前需要的（页号，位图宽度）：渲染线程开始渲染前检查，不在其中的请求跳过。
    wanted: Arc<Mutex<HashSet<(usize, u32)>>>,
    /// 已发给渲染线程、尚未送回的（页号，位图宽度）。
    requested: HashSet<(usize, u32)>,
    /// 已请求读取、尚未送回的文字页。
    reading: HashSet<usize>,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
    /// 打开的结果：各页大小（点），或失败的原因；尚未打开完时为 `None`。
    pub pages: Option<Result<Arc<[PageSize]>, String>>,
    /// 能否选字和读写批注（文档由 PDFium 打开）。
    pub editable: bool,
    /// 渲染好的页：页号到位图，只保留当前需要的页。
    pub rendered: HashMap<usize, Arc<Pixels>>,
    /// 文件被修改、重新渲染期间暂时显示的旧位图：新位图到了就不再显示。
    pub stale: HashMap<usize, Arc<Pixels>>,
    /// 读好的文字和批注：页号到内容，只保留当前需要的页和选区经过的页。
    pub texts: HashMap<usize, Arc<PageText>>,
    /// 选区：起点和终点（终点随鼠标移动）；没有选字时为 `None`。
    pub selection: Option<(Caret, Caret)>,
    /// 最近一次点在 PDF 上：此时 ⌘C、⌘A 对着 PDF 的文字。
    pub focused: bool,
    /// 右键点中的批注：（页号，在 [`PageText::notes`] 中的位置）。
    pub menu_note: Option<(usize, usize)>,
    /// 输入条里正在写的批注是给谁的。
    pending: Option<Pending>,
    /// 最近一次保存批注后文件的修改时间：文件的修改时间与它相同时是自己保存的，不重新打开。
    own_write: Option<SystemTime>,
    /// 滚动位置（逻辑像素，内容顶部到视口顶部的距离）。
    pub scroll: f32,
    /// 上一帧绘制的区域（窗口坐标）；不显示时为空矩形。
    pub area: Rect,
    /// 上一帧排出的内容总高度（逻辑像素），滚动时按它限制范围。
    pub height: f32,
    /// 上一帧各页的位置（逻辑像素，相对内容左上角），点击时换算鼠标落在哪一页。
    pub page_rects: Vec<Rect>,
}

impl Default for PdfPreview {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        PdfPreview {
            doc: None,
            generation: 0,
            requests: None,
            wanted: Arc::default(),
            requested: HashSet::new(),
            reading: HashSet::new(),
            sender,
            receiver,
            pages: None,
            editable: false,
            rendered: HashMap::new(),
            stale: HashMap::new(),
            texts: HashMap::new(),
            selection: None,
            focused: false,
            menu_note: None,
            pending: None,
            own_write: None,
            scroll: 0.0,
            area: Rect::default(),
            height: 0.0,
            page_rects: Vec::new(),
        }
    }
}

impl PdfPreview {
    /// 结束渲染线程，释放各页的位图，回到没有文档的状态。
    fn reset(&mut self) {
        self.doc = None;
        self.requests = None;
        self.generation += 1;
        self.requested.clear();
        self.reading.clear();
        if let Ok(mut wanted) = self.wanted.lock() {
            wanted.clear();
        }
        self.pages = None;
        self.editable = false;
        self.rendered.clear();
        self.stale.clear();
        self.texts.clear();
        self.selection = None;
        self.menu_note = None;
        self.pending = None;
        self.own_write = None;
        self.scroll = 0.0;
        self.height = 0.0;
        self.page_rects.clear();
    }

    /// 修改时间为 `mtime` 的文件是不是刚由本预览保存批注写出的。
    pub(crate) fn wrote(&self, mtime: Option<SystemTime>) -> bool {
        self.own_write.is_some() && mtime == self.own_write
    }

    /// 最大滚动位置（逻辑像素）。
    pub(crate) fn max_scroll(&self) -> f32 {
        (self.height - self.area.h).max(0.0)
    }

    /// 第 `page` 页在窗口中的位置（上一帧的排版）。
    fn page_in_window(&self, page: usize) -> Option<Rect> {
        let r = self.page_rects.get(page)?;
        Some(Rect::new(
            self.area.x + r.x,
            self.area.y + r.y - self.scroll,
            r.w,
            r.h,
        ))
    }

    /// 窗口坐标 `(x, y)` 附近的页和页面上的显示坐标（0 到 1）：落在两页之间或页外时取上下
    /// 最近的一页，坐标限制在页面以内。
    fn page_near(&self, x: f32, y: f32) -> Option<(usize, f32, f32)> {
        let (page, rect) = (0..self.page_rects.len())
            .filter_map(|i| Some((i, self.page_in_window(i)?)))
            .min_by(|(_, a), (_, b)| {
                let gap = |r: &Rect| (r.y - y).max(y - r.bottom()).max(0.0);
                gap(a).total_cmp(&gap(b))
            })?;
        let fx = ((x - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
        let fy = ((y - rect.y) / rect.h.max(1.0)).clamp(0.0, 1.0);
        Some((page, fx, fy))
    }

    /// 窗口坐标 `(x, y)` 所在的页和页面上的显示坐标；不在任何一页上时为 `None`。
    fn page_at(&self, x: f32, y: f32) -> Option<(usize, f32, f32)> {
        let (page, fx, fy) = self.page_near(x, y)?;
        self.page_in_window(page)?
            .contains(x, y)
            .then_some((page, fx, fy))
    }

    /// 第 `page` 页上离显示坐标 `(fx, fy)` 最近的文字位置：先找竖直方向上覆盖该点的字（同一行），
    /// 没有时取上下最近的字；点在字的右半边时位置在该字之后。页上没有文字时为 `None`。
    fn caret_near(&self, page: usize, fx: f32, fy: f32) -> Option<Caret> {
        let text = self.texts.get(&page)?;
        let distance = |r: &PageRect| {
            let dy = (r.top - fy).max(fy - r.bottom).max(0.0);
            let dx = (r.left - fx).max(fx - r.right).max(0.0);
            (dy, dx)
        };
        let (index, rect) = text
            .chars
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((i, c.rect?)))
            .min_by(|(_, a), (_, b)| {
                let (da, db) = (distance(a), distance(b));
                da.0.total_cmp(&db.0).then(da.1.total_cmp(&db.1))
            })?;
        let after = fx > (rect.left + rect.right) / 2.0;
        Some(Caret {
            page,
            index: index + usize::from(after),
        })
    }

    /// 选区经过的各页：（页号，字的序号范围），只含有字的段；文字还没读好的页跳过。
    pub(crate) fn segments(&self) -> Vec<(usize, Range<usize>)> {
        let Some((a, b)) = self.selection else {
            return Vec::new();
        };
        let (from, to) = (a.min(b), a.max(b));
        (from.page..=to.page)
            .filter_map(|page| {
                let len = self.texts.get(&page)?.chars.len();
                let start = if page == from.page { from.index } else { 0 };
                let end = if page == to.page { to.index } else { len };
                let range = start.min(len)..end.min(len);
                (!range.is_empty()).then_some((page, range))
            })
            .collect()
    }

    /// 第 `page` 页上选中的字的序号范围；该页没有选中的字时为 `None`。
    pub(crate) fn selected_on(&self, page: usize) -> Option<Range<usize>> {
        self.segments()
            .into_iter()
            .find_map(|(p, range)| (p == page).then_some(range))
    }

    /// 窗口坐标 `(x, y)` 处的批注：（页号，在 [`PageText::notes`] 中的位置）；有多个时取最后
    /// 加的（画在最上面）。
    pub(crate) fn note_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (page, fx, fy) = self.page_at(x, y)?;
        let notes = &self.texts.get(&page)?.notes;
        let i = notes
            .iter()
            .rposition(|n| n.rects.iter().any(|r| r.contains(fx, fy)))?;
        Some((page, i))
    }

    /// 窗口坐标 `(x, y)` 是否在某个字上（用来显示文字光标）。
    pub(crate) fn text_at(&self, x: f32, y: f32) -> bool {
        self.page_at(x, y).is_some_and(|(page, fx, fy)| {
            self.texts.get(&page).is_some_and(|t| {
                t.chars
                    .iter()
                    .any(|c| c.rect.is_some_and(|r| r.contains(fx, fy)))
            })
        })
    }

    /// 是否还在等渲染线程的结果（测试中等待用）。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn is_busy(&self) -> bool {
        self.requests.is_some()
            && (self.pages.is_none() || !self.requested.is_empty() || !self.reading.is_empty())
    }
}

/// 把修改过的文档保存进 `path`：先写进同一目录下的临时文件，再替换原文件，读者不会读到写了
/// 一半的文件。返回保存后文件的修改时间。
fn save_to(document: &Document, path: &Path) -> Result<Option<SystemTime>, String> {
    let failed = || winc_i18n::t!("preview-pdf-save-failed").to_owned();
    let bytes = document.save()?;
    let dir = path.parent().ok_or_else(failed)?;
    let name = path.file_name().ok_or_else(failed)?.to_string_lossy();
    let (temp, mut file) =
        winc_config::files::create_temporary(dir, &name, false).map_err(|e| e.to_string())?;
    let written = file
        .write_all(&bytes)
        .and_then(|()| winc_config::files::sync_file(&file));
    drop(file);
    if let Err(e) = written.and_then(|()| winc_config::files::replace_file(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.to_string());
    }
    Ok(std::fs::metadata(path).and_then(|m| m.modified()).ok())
}

/// 在渲染线程中按顺序做完 `edits`。
fn apply(document: &mut Document, edits: Vec<(usize, Edit)>) -> Result<(), String> {
    for (page, edit) in edits {
        match edit {
            Edit::Add {
                kind,
                chars,
                comment,
            } => document.annotate(page, kind, chars, &comment)?,
            Edit::Comment { index, comment } => document.set_comment(page, index, &comment)?,
            Edit::Remove(index) => document.remove_annotation(page, index)?,
        }
    }
    Ok(())
}

/// 一页上第 `range` 个字的文字：PDFium 补出的 `\r\n` 换成 `\n`。
fn page_text(text: &PageText, range: Range<usize>) -> String {
    let end = range.end.min(text.chars.len());
    let raw: String = text.chars[range.start.min(end)..end]
        .iter()
        .map(|c| c.ch)
        .collect();
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

impl App {
    /// 当前标签是可以预览的 PDF 时，按需要打开渲染线程；不再显示 PDF 时结束线程、释放位图。
    ///
    /// 每次绘制前调用。
    pub(crate) fn advance_pdf(&mut self) {
        let active = self
            .editor
            .active()
            .filter(|d| d.unsupported == Some(Unsupported::Pdf))
            .map(|d| (d.id, d.revision, d.path.clone(), d.disk_mtime()));
        let pdf = &mut self.pdf;
        let Some((id, revision, Some(path), mtime)) = active else {
            if pdf.doc.is_some() {
                pdf.reset();
            }
            return;
        };
        if pdf.doc == Some((id, revision)) {
            return;
        }
        let same = pdf.doc.map(|(d, _)| d) == Some(id);
        // 自己保存批注引起的版本变化：线程里的文档已经是修改后的样子，不必重新打开。
        if same && pdf.requests.is_some() && pdf.own_write.is_some() && mtime == pdf.own_write {
            pdf.doc = Some((id, revision));
            return;
        }
        // 同一文档的版本改变（磁盘上的文件被修改）时保留滚动位置，新位图到达之前先显示旧的。
        let scroll = same.then_some(pdf.scroll);
        let stale = if same {
            let mut old = std::mem::take(&mut pdf.rendered);
            old.extend(std::mem::take(&mut pdf.stale));
            old
        } else {
            HashMap::new()
        };
        pdf.reset();
        pdf.scroll = scroll.unwrap_or(0.0);
        pdf.stale = stale;
        pdf.doc = Some((id, revision));
        let (requests, queue) = mpsc::channel::<Job>();
        pdf.requests = Some(requests);
        let (generation, sender, wanted, proxy) = (
            pdf.generation,
            pdf.sender.clone(),
            Arc::clone(&pdf.wanted),
            self.proxy.clone(),
        );
        let notify = move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 系统的 PDF 对象只能在打开它的线程上使用：文档在线程里打开，请求通道关闭时线程结束。
        let spawned = std::thread::Builder::new()
            .name("winc-pdf".to_owned())
            .spawn(move || {
                let mut document = match Document::open(&path) {
                    Ok(document) => document,
                    Err(reason) => {
                        let _ = sender.send(Done::Opened(generation, Err(reason)));
                        notify();
                        return;
                    }
                };
                let opened = (document.pages().to_vec(), document.editable());
                if sender.send(Done::Opened(generation, Ok(opened))).is_err() {
                    return;
                }
                notify();
                for job in queue {
                    let done = match job {
                        Job::Render {
                            page,
                            width,
                            height,
                        } => {
                            if !wanted.lock().is_ok_and(|w| w.contains(&(page, width))) {
                                continue;
                            }
                            let pixels = document.render(page, width, height).ok().map(Arc::new);
                            Done::Rendered(generation, page, pixels)
                        }
                        Job::Read(page) => {
                            let text = document.text(page).ok().map(|chars| {
                                let notes = document.annotations(page).unwrap_or_default();
                                Arc::new(PageText { chars, notes })
                            });
                            Done::Read(generation, page, text)
                        }
                        Job::Edit(edits) => {
                            let pages = edits.iter().map(|&(page, _)| page).collect();
                            let saved = apply(&mut document, edits)
                                .and_then(|()| save_to(&document, &path));
                            Done::Edited(generation, pages, saved)
                        }
                    };
                    if sender.send(done).is_err() {
                        return;
                    }
                    notify();
                }
            });
        if spawned.is_err() {
            pdf.requests = None;
            pdf.pages = Some(Err(winc_i18n::t!("preview-pdf-render-failed").to_owned()));
        }
    }

    /// 收下渲染线程送回的结果。
    pub(crate) fn poll_pdf(&mut self) {
        let mut changed = false;
        while let Ok(done) = self.pdf.receiver.try_recv() {
            let pdf = &mut self.pdf;
            match done {
                Done::Opened(generation, opened) if generation == pdf.generation => {
                    match opened {
                        Ok((pages, editable)) => {
                            pdf.pages = Some(Ok(Arc::from(pages)));
                            pdf.editable = editable;
                        }
                        Err(reason) => {
                            pdf.pages = Some(Err(reason));
                            pdf.requests = None;
                            pdf.stale.clear();
                        }
                    }
                    changed = true;
                }
                Done::Rendered(generation, page, pixels) if generation == pdf.generation => {
                    pdf.requested.retain(|&(p, _)| p != page);
                    if let Some(pixels) = pixels {
                        pdf.rendered.insert(page, pixels);
                        pdf.stale.remove(&page);
                    }
                    changed = true;
                }
                Done::Read(generation, page, text) if generation == pdf.generation => {
                    pdf.reading.remove(&page);
                    if let Some(text) = text {
                        pdf.texts.insert(page, text);
                    }
                    changed = true;
                }
                Done::Edited(generation, pages, saved) if generation == pdf.generation => {
                    match saved {
                        Ok(mtime) => {
                            pdf.own_write = mtime;
                            // 改过的页重新渲染、重新读取批注；新位图到达之前先显示旧的。
                            for page in pages {
                                if let Some(old) = pdf.rendered.remove(&page) {
                                    pdf.stale.insert(page, old);
                                }
                                pdf.requested.retain(|&(p, _)| p != page);
                                pdf.texts.remove(&page);
                                pdf.reading.remove(&page);
                            }
                            self.flash(winc_i18n::t!("preview-pdf-saved"));
                        }
                        Err(reason) => self.notify(reason),
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 这一帧需要的页：（页号，位图宽度，位图高度），单位为物理像素。
    ///
    /// 不在其中的页从缓存中移除，也不再渲染；缺少的页交给渲染线程。能读取文字时也读取这几页的
    /// 文字和批注。绘制时调用。
    pub(crate) fn request_pdf_pages(&mut self, pages: &[(usize, u32, u32)]) {
        let pdf = &mut self.pdf;
        let keys: HashSet<(usize, u32)> = pages.iter().map(|&(p, w, _)| (p, w)).collect();
        let shown: HashSet<usize> = pages.iter().map(|&(p, _, _)| p).collect();
        pdf.rendered
            .retain(|&page, pixels| keys.contains(&(page, pixels.width)));
        pdf.stale.retain(|page, _| shown.contains(page));
        pdf.requested.retain(|key| keys.contains(key));
        // 选区经过的页即使滚出了视口也保留文字，复制、加批注时还要用。
        let selected = pdf
            .selection
            .map(|(a, b)| a.page.min(b.page)..=a.page.max(b.page));
        pdf.texts.retain(|page, _| {
            shown.contains(page) || selected.as_ref().is_some_and(|r| r.contains(page))
        });
        if let Ok(mut wanted) = pdf.wanted.lock() {
            wanted.clone_from(&keys);
        }
        let Some(requests) = pdf.requests.as_ref() else {
            return;
        };
        for &(page, width, height) in pages {
            let key = (page, width);
            let ready = pdf.rendered.get(&page).is_some_and(|p| p.width == width);
            if !ready
                && !pdf.requested.contains(&key)
                && requests
                    .send(Job::Render {
                        page,
                        width,
                        height,
                    })
                    .is_ok()
            {
                pdf.requested.insert(key);
            }
            if pdf.editable
                && !pdf.texts.contains_key(&page)
                && !pdf.reading.contains(&page)
                && requests.send(Job::Read(page)).is_ok()
            {
                pdf.reading.insert(page);
            }
        }
    }

    /// 处理 PDF 预览中的按键：上下方向键滚动一小段，`PageUp`、`PageDown`、空格滚动一屏，
    /// `Home`、`End` 到开头、结尾，`Esc` 取消选字。返回是否处理；当前标签不是 PDF 时返回
    /// `false`。
    pub(in crate::app) fn pdf_key(&mut self, key: &Key) -> bool {
        if self.pdf.doc.is_none() {
            return false;
        }
        // 翻一屏时与上一屏重叠一段，翻页后仍能看到上一屏末尾的内容。
        let page = (self.pdf.area.h - ARROW_STEP * 2.0).max(ARROW_STEP);
        match key {
            Key::Named(NamedKey::Escape) => {
                if self.pdf.selection.take().is_none() {
                    return false;
                }
                self.request_redraw();
            }
            Key::Named(NamedKey::ArrowUp) => self.scroll_pdf(ARROW_STEP),
            Key::Named(NamedKey::ArrowDown) => self.scroll_pdf(-ARROW_STEP),
            Key::Named(NamedKey::PageUp) => self.scroll_pdf(page),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.scroll_pdf(-page),
            Key::Named(NamedKey::Home) => self.scroll_pdf(f32::MAX),
            Key::Named(NamedKey::End) => self.scroll_pdf(-f32::MAX),
            _ => return false,
        }
        true
    }

    /// 把 PDF 预览滚动 `dy`（逻辑像素，向下滚动为负）。
    pub(crate) fn scroll_pdf(&mut self, dy: f32) {
        let pdf = &mut self.pdf;
        pdf.scroll = (pdf.scroll - dy).clamp(0.0, pdf.max_scroll());
        self.request_redraw();
    }

    /// PDF 是否拿着键盘：当前标签是 PDF，且最近一次点在 PDF 上。
    pub(crate) fn pdf_focused(&self) -> bool {
        self.pdf.doc.is_some() && self.pdf.focused && self.focus == Focus::Editor
    }

    /// 在 PDF 上按下鼠标：拿到键盘；能选字时开始选字（按住 Shift 时延伸选区），双击选词，
    /// 三击选一行。
    pub(in crate::app) fn click_pdf(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        self.pdf.focused = true;
        self.request_redraw();
        if !self.pdf.editable {
            return;
        }
        let count = self.click_count(x, y);
        let shift = self.modifiers.shift_key();
        let pdf = &self.pdf;
        let at = pdf
            .page_near(x, y)
            .and_then(|(page, fx, fy)| pdf.caret_near(page, fx, fy));
        let Some(at) = at else {
            self.pdf.selection = None;
            return;
        };
        self.pdf.selection = match self.pdf.selection {
            _ if count >= 2 => self.pdf_unit_at(at, count >= 3),
            Some((anchor, _)) if shift => Some((anchor, at)),
            _ => Some((at, at)),
        };
        self.drag = crate::app::Drag::PdfText;
    }

    /// 位置 `at` 所在的词（`line` 为 `false`）或行：词是连着的字母数字、连着的中日韩文字，或者
    /// 单个别的字；行到换行为止。
    fn pdf_unit_at(&self, at: Caret, line: bool) -> Option<(Caret, Caret)> {
        let chars = &self.pdf.texts.get(&at.page)?.chars;
        let i = at.index.min(chars.len().checked_sub(1)?);
        let class = |c: char| {
            if line {
                u8::from(c != '\r' && c != '\n')
            } else if c.is_alphanumeric() && (c as u32) < 0x2E80 || c == '_' {
                1
            } else if c.is_alphanumeric() {
                2
            } else {
                0
            }
        };
        let kind = class(chars[i].ch);
        let (start, end) = if kind == 0 {
            (i, i + 1)
        } else {
            let start = chars[..i]
                .iter()
                .rposition(|c| class(c.ch) != kind)
                .map_or(0, |p| p + 1);
            let end = chars[i..]
                .iter()
                .position(|c| class(c.ch) != kind)
                .map_or(chars.len(), |p| i + p);
            (start, end)
        };
        let caret = |index| Caret {
            page: at.page,
            index,
        };
        Some((caret(start), caret(end)))
    }

    /// 拖着选字：拖出上下边缘时跟着滚。
    pub(in crate::app) fn drag_pdf(&mut self, x: f32, y: f32) {
        let area = self.pdf.area;
        if y < area.y {
            self.scroll_pdf((area.y - y).min(60.0));
        } else if y > area.bottom() {
            self.scroll_pdf(-(y - area.bottom()).min(60.0));
        }
        let y = y.clamp(area.y, area.bottom() - 1.0);
        let pdf = &self.pdf;
        let at = pdf
            .page_near(x, y)
            .and_then(|(page, fx, fy)| pdf.caret_near(page, fx, fy));
        if let Some(at) = at
            && let Some((anchor, _)) = self.pdf.selection
        {
            self.pdf.selection = Some((anchor, at));
        }
        self.request_redraw();
    }

    /// PDF 里选中的文字，跨页时各页之间换行；没选中时返回 `None`。
    pub(crate) fn pdf_selected_text(&self) -> Option<String> {
        let parts: Vec<String> = self
            .pdf
            .segments()
            .into_iter()
            .filter_map(|(page, range)| Some(page_text(self.pdf.texts.get(&page)?, range)))
            .collect();
        let text = parts.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    /// 选中视口中间那一页的全部文字（⌘A）。
    pub(crate) fn pdf_select_all(&mut self) {
        let pdf = &self.pdf;
        let middle = pdf.area.y + pdf.area.h / 2.0;
        let Some((page, _, _)) = pdf.page_near(pdf.area.x + pdf.area.w / 2.0, middle) else {
            return;
        };
        let Some(text) = pdf.texts.get(&page) else {
            return;
        };
        let end = Caret {
            page,
            index: text.chars.len(),
        };
        self.pdf.selection = Some((Caret { page, index: 0 }, end));
        self.request_redraw();
    }

    /// 处理 PDF 上的右键点击，返回是否点在可以选字的 PDF 上。
    ///
    /// PDF 拿到键盘，点击位置在选区外时取消选区；点在批注上时记下这个批注，菜单里可以改写、
    /// 删除它。
    pub(in crate::app) fn pdf_context_target(&mut self, x: f32, y: f32) -> bool {
        if !self.pdf.area.contains(x, y) || !self.pdf.editable {
            return false;
        }
        self.focus = Focus::Editor;
        let pdf = &mut self.pdf;
        pdf.focused = true;
        pdf.menu_note = pdf.note_at(x, y);
        let at = pdf
            .page_at(x, y)
            .and_then(|(page, fx, fy)| pdf.caret_near(page, fx, fy));
        let inside = pdf
            .selection
            .zip(at)
            .is_some_and(|((a, b), at)| (a.min(b)..=a.max(b)).contains(&at));
        if !inside {
            pdf.selection = None;
        }
        self.request_redraw();
        true
    }

    /// 是否选中了能加批注的字（菜单项是否可用）。
    pub(crate) fn pdf_can_mark(&self) -> bool {
        self.pdf.editable && self.pdf_selected_text().is_some()
    }

    /// 把 `edits` 交给渲染线程，改完后保存进文件。
    fn send_pdf_edits(&mut self, edits: Vec<(usize, Edit)>) {
        if edits.is_empty() {
            return;
        }
        if let Some(requests) = self.pdf.requests.as_ref()
            && requests.send(Job::Edit(edits)).is_ok()
        {
            return;
        }
        self.notify(winc_i18n::t!("preview-pdf-annotate-failed").to_owned());
    }

    /// 给选中的字加一个 `kind` 批注（不写注释），取消选区。
    pub(in crate::app) fn pdf_mark(&mut self, kind: AnnotationKind) {
        let edits = self
            .pdf
            .segments()
            .into_iter()
            .map(|(page, chars)| {
                let comment = String::new();
                (
                    page,
                    Edit::Add {
                        kind,
                        chars,
                        comment,
                    },
                )
            })
            .collect();
        self.pdf.selection = None;
        self.send_pdf_edits(edits);
        self.request_redraw();
    }

    /// 给选中的字加带注释的高亮：弹出输入条写注释，回车后保存（见 [`App::pdf_submit_comment`]）。
    pub(in crate::app) fn pdf_comment_selection(&mut self) {
        let segments = self.pdf.segments();
        if segments.is_empty() {
            return;
        }
        self.pdf.pending = Some(Pending::Highlight(segments));
        self.open_pdf_comment_prompt(TextField::default());
    }

    /// 改写右键点中的批注的注释：输入框里是现在的注释并已全选。
    pub(in crate::app) fn pdf_edit_note(&mut self) {
        let Some((page, i)) = self.pdf.menu_note.take() else {
            return;
        };
        let Some(note) = self.pdf.texts.get(&page).and_then(|t| t.notes.get(i)) else {
            return;
        };
        let mut field = TextField::default();
        field.set_text(&note.comment);
        self.pdf.pending = Some(Pending::Comment {
            page,
            index: note.index,
        });
        self.open_pdf_comment_prompt(field);
    }

    /// 删除右键点中的批注。
    pub(in crate::app) fn pdf_remove_note(&mut self) {
        let Some((page, i)) = self.pdf.menu_note.take() else {
            return;
        };
        let Some(index) = self
            .pdf
            .texts
            .get(&page)
            .and_then(|t| t.notes.get(i))
            .map(|n| n.index)
        else {
            return;
        };
        self.send_pdf_edits(vec![(page, Edit::Remove(index))]);
    }

    /// 弹出写批注的输入条。
    fn open_pdf_comment_prompt(&mut self, field: TextField) {
        self.open_prompt(PromptKind::PdfComment, field);
    }

    /// 输入条里写完批注、按下回车：加带注释的高亮，或改写已有批注的注释。
    pub(in crate::app) fn pdf_submit_comment(&mut self, input: &str) {
        let comment = input.trim().to_owned();
        let edits = match self.pdf.pending.take() {
            Some(Pending::Highlight(segments)) => {
                self.pdf.selection = None;
                segments
                    .into_iter()
                    .map(|(page, chars)| {
                        let kind = AnnotationKind::Highlight;
                        let comment = comment.clone();
                        (
                            page,
                            Edit::Add {
                                kind,
                                chars,
                                comment,
                            },
                        )
                    })
                    .collect()
            }
            Some(Pending::Comment { page, index }) => {
                vec![(page, Edit::Comment { index, comment })]
            }
            None => return,
        };
        self.send_pdf_edits(edits);
    }

    /// 测试中等渲染线程完成打开和已发出的渲染请求，并收下结果。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn wait_for_pdf(&mut self) {
        self.advance_pdf();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.pdf.is_busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_pdf();
        }
    }
}
