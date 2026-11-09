//! Markdown 预览（分栏右边，或只看预览时整个编辑区）：后台解析、滚动、链接和图片。
//!
//! 同一时刻只在后台解析一个版本，输入快于解析时跳过中间的版本。排版在绘制时进行（需要测量字宽），
//! 按块缓存，见 `view::preview::markdown`。图片读取本地文件（相对文档所在目录）；网址的图片
//! （README 里的徽章、截图）在后台用系统的 `curl` 下载到临时目录缓存，关闭联网后显示替代文字。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use winc_core::Lines;
use winc_editor::{DocumentId, PreviewKind, PreviewMode};
use winc_i18n::t;
use winc_preview::image::{self, Picture, Pixels};
use winc_preview::markdown::{self, Markdown};
use winc_preview::svg;

use super::images::system_glyphs;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, UserEvent};
use crate::system;

mod viewer;
pub(crate) use viewer::{DiagramTool, DiagramView, MAX_ZOOM, MIN_ZOOM};

/// docx 包中的图片：Markdown 中引用的地址和图片文件的内容。
type Media = Vec<(String, Arc<[u8]>)>;

/// 后台任务的结果。
enum Done {
    /// 文档、所依据的文档版本、解析结果。
    Parsed(DocumentId, u64, Markdown),
    /// docx 文档、所依据的文档版本、转换并解析的结果（含包中的图片）或失败的原因。
    Converted(DocumentId, u64, Result<(Markdown, Media), String>),
    /// 图片（解析好的本地路径）、读取结果。
    Loaded(PathBuf, Result<Arc<Picture>, String>),
    /// 图片、按显示大小（物理像素）画好的像素。
    Drawn(PathBuf, Arc<Pixels>),
}

/// 一张图片的状态。
pub(crate) enum ImageSlot {
    /// 在后台读取。
    Loading,
    /// 读好了；`pixels` 是按显示大小画好的像素（同一张图可能以几种大小出现，比如并排的
    /// 截图和表格里的缩略图，最近画的在前，最多留 [`SIZES`] 种），`drawing` 是正在后台画的
    /// 大小。
    Ready {
        picture: Arc<Picture>,
        pixels: Vec<Arc<Pixels>>,
        drawing: Vec<(u32, u32)>,
    },
    /// 读不了（不存在、格式不支持）：显示替代文字。
    Failed,
}

/// 可编辑的预览中光标所在的叶子块（见 [`Markdown::leaves`]）：这一块显示原文。
///
/// 原文位置都按现在的文字算：解析之后打的字都在这一块里（光标就在这里），块的末尾按文字
/// 长度的变化挪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveLeaf {
    /// 所在的顶层块（`Markdown::blocks` 的序号）。
    pub block: usize,
    /// 所在顶层块现在的范围（字节偏移）。
    pub range: std::ops::Range<usize>,
    /// 叶子块现在的范围（字节偏移）；光标在它后面的空行上时延到光标所在的那一行，在它前面
    /// 的块标记上时从光标所在的行算起。
    pub leaf: std::ops::Range<usize>,
    /// 文档版本：打了字这一块就要重排。
    pub revision: u64,
    /// 是正文（段落、标题），见 `LeafRange::prose`。
    pub prose: bool,
    /// 是表格：按单元格编辑。
    pub table: bool,
    /// 查找这一块所用的位置（字节偏移）。
    ///
    /// 通常是光标位置；按住鼠标期间是按下之前的光标位置。表格中该位置所在的单元格显示原文，
    /// 正文中显示该位置接触到的标记。
    pub at: usize,
    /// 光标位于代码块、公式等叶子块的第一行或最后两行时，光标所在的行号。
    ///
    /// 围栏行只在光标位于其上时显示，因此光标进出这几行时需要重排；在中间各行移动时不重排。
    pub edge: Option<usize>,
}

/// 一张图最多保留的已绘制尺寸数（种），超出时移除最早画好的。
const SIZES: usize = 4;

/// 预览左侧的标题指示条上可以指向的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocTarget {
    /// 指示条本身：鼠标停在上面时展开标题列表。
    Bars,
    /// 标题列表里的一项，值为 [`MarkdownPreview::toc_tops`] 的序号：点击后滚到这个标题。
    Heading(usize),
}

/// 当前标签的 Markdown 预览。
pub(crate) struct MarkdownPreview {
    /// 预览的是哪个文档；当前标签换了时从头开始。
    pub doc: Option<DocumentId>,
    /// 最近一次解析的结果。
    pub parsed: Option<Arc<Markdown>>,
    /// 最新解析过的文档版本号（`revision`）。
    checked: Option<u64>,
    /// 正在后台解析的版本号（`revision`）。
    pending: Option<u64>,
    /// 各文档预览滚到的位置（逻辑像素），切走再切回来还在原处。
    scrolls: HashMap<DocumentId, f32>,
    /// 排版结果，见 `view::markdown`。
    pub layout: Option<crate::view::preview::markdown::Laid>,
    /// 排版缓存，见 `view::markdown::Memo`。
    pub memo: crate::view::preview::markdown::Memo,
    /// 用到的图片，键是解析好的本地路径。
    pub images: HashMap<PathBuf, ImageSlot>,
    /// docx 包中的图片：键与 `images` 相同（按文档所在目录解析好的路径），值是图片文件的
    /// 内容；读取这些图片时用这里的内容，不读磁盘。
    embedded: HashMap<PathBuf, Arc<[u8]>>,
    /// docx 读取或转换失败的原因；没有失败时为 `None`。
    pub failed: Option<String>,
    /// 图片全部作废（联网的设置变了）：排版整份重来。
    ///
    /// 每作废一次加 1。
    pub images_version: u64,
    /// 刚读好（或读不了）的图片：只重排用到它们的块，不必整份重排。
    pub loaded: Vec<PathBuf>,
    /// 点过、和原来开合相反的折叠块（`<details>`），见 `view::markdown` 的 `DETAILS`。
    pub toggled: HashSet<u64>,
    /// 刚点的折叠块标题的地址：只重排它所在的块。
    pub toggled_now: Option<String>,
    /// 点了 `其他.md#标题` 打开的文档和要跳的标题：那个文档排好版后滚过去。
    pub pending_anchor: Option<(DocumentId, String)>,
    /// 上一帧画出来的链接：区域和地址，点击时按它找。
    pub links: Vec<(winc_render::Rect, String)>,
    /// 上一帧画出来的标题指示条和标题列表的可点击位置，见 `view::markdown` 的 `draw_toc`。
    pub toc: Vec<(winc_render::Rect, TocTarget)>,
    /// 上一帧标题指示条列出的各标题在内容里的高度（逻辑像素），与 [`TocTarget::Heading`] 的
    /// 序号对应；点击标题时滚到这里。
    pub toc_tops: Vec<f32>,
    /// 标题列表展开时淡入的时钟；不在淡入中时为 `None`。
    pub toc_fade: Option<crate::app::animation::Clock>,
    /// 在标题列表里点过的标题（`toc_tops` 的序号）和点完后的滚动位置。
    ///
    /// 靠近文末的标题滚不到预览顶上，按滚动位置算不出它是当前所在的一节；点过以后、再次滚动
    /// 之前，指示条把它当作当前所在的一节。
    pub toc_picked: Option<(usize, f32)>,
    /// 上一帧标题指示条标出的当前所在一节（`toc_tops` 的序号）；没有时为 `None`。
    pub toc_current: Option<usize>,
    /// 点过的标题在正文里铺的底色：标题的序号与淡出的时钟。
    pub toc_flash: Option<(usize, crate::app::animation::Clock)>,
    /// 上一帧画出来的代码块（屏幕坐标）：鼠标移进移出时重画，好显示、收起复制按钮。
    pub codes: Vec<winc_render::Rect>,
    /// 上一帧画出复制按钮的代码块的内容；按钮在 `links` 里只记 `COPY`，点了才取这里的文字。
    ///
    /// 与排版结果共用同一段文字：鼠标停在代码块上时每帧都要画按钮，不必每帧复制整块。
    pub copy: Option<std::sync::Arc<str>>,
    /// 上一帧绘制了全屏按钮的图的源码。
    ///
    /// `links` 中的按钮只记录 `EXPAND`，点击时从这里取源码，绘制时不必复制源码。
    pub expand: Option<std::sync::Arc<str>>,
    /// 全屏查看中的图；未打开全屏查看时为 `None`。
    pub diagram: Option<DiagramView>,
    /// 鼠标在第几个代码块（`codes` 的序号）上。
    pub code_hover: Option<usize>,
    /// 上一帧画在哪里（分栏的右半边或整个编辑区）；不显示时为空矩形。
    pub area: winc_render::Rect,
    /// 上一帧排出来的总高度，滚动时按它限制范围。
    pub height: f32,
    /// 选中的文字：起点和终点（整份文字里的字符偏移，见 `view::markdown::Laid::text`）。
    pub selection: Option<(usize, usize)>,
    /// 最近一次点在预览上：分栏时 ⌘C、⌘A、⌘F 对着预览，点回左边的文字后不再如此。
    pub focused: bool,
    /// 右键点中的字在原文中的位置（字节偏移），「在源代码中显示」用；点在没有对应原文的
    /// 地方或解析结果不是当前版本时为 `None`。
    pub menu_source: Option<usize>,
    /// 查找用的文字：预览的整段文字装成一个不显示的文档，查找条按普通文档的方式查它。
    ///
    /// 排版重来时作废，要查时再装。
    pub find_doc: Option<winc_editor::Document>,
    /// 装过几次 `find_doc`：每次换一个文档标识，查找的结果跟着作废。
    find_serial: u64,
    /// 分栏时上一次两边对齐的情形：文档、左边文字滚到的行、预览的总高度。
    ///
    /// 左边滚了（滚轮、光标移出屏幕、打字）或者预览重排了，下一帧按左边重新对齐右边；
    /// 在右边滚时直接对齐左边，并记下来。
    pub synced: Option<(DocumentId, f32, f32)>,
    /// 可编辑的预览中拖动选择文字的状态。
    ///
    /// 依次为起点（字符偏移，双击时为起始词的范围）、是否按词扩展、
    /// 按下之前光标的原文位置（字节偏移，见 [`App::live_leaf`]）。
    live_drag: Option<((usize, usize), bool, Option<usize>)>,
    /// 可编辑的预览中 ↑↓ 移动的目标横坐标（相对内容左边）和移动后光标的位置（字符偏移）：
    /// 光标仍在该位置时继续按该横坐标移动，经过短行后仍能回到原来的列。
    goal: Option<(f32, usize)>,
    /// 可编辑的预览中上一帧的文档、光标所在的位置和文档版本：变了说明在编辑，见
    /// [`App::live_editing`]。
    followed: Option<(DocumentId, usize, u64)>,
    /// 在可编辑的预览中编辑过的文档：光标所在的块显示原文。
    editing: Option<DocumentId>,
    /// 刚换成只看预览的文档：排好版后预览滚到文字原来滚到的地方，见 [`App::align_markdown_once`]。
    align: Option<DocumentId>,
    /// 重排后的过渡动画：各块从原位置滑动到新位置，新显示的标记淡入，见 [`BlockMotion`]。
    pub motion: Option<BlockMotion>,
    /// 测试中固定的 [`BlockMotion`] 已用时间，用于截取动画中间帧。
    #[cfg(test)]
    pub frozen: Option<std::time::Duration>,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
}

/// 淡入的项：所在顶层块的序号及块内各项的序号，按块的序号排序。
pub(crate) type Fading = Vec<(usize, Box<[usize]>)>;

/// 可编辑的预览重排后的过渡动画。
///
/// 显示原文的块改变外观或表格删除一行后，各顶层块从原绘制位置滑动到新位置，高度变化的块按
/// 过渡中的高度裁剪；仅移动光标（未修改文字）时，新显示的标记和围栏淡入。输入文字时不淡入，
/// 否则每输入一个字符，标记都会闪烁一次。
pub(crate) struct BlockMotion {
    /// 过渡动画的时钟。
    pub clock: crate::app::Clock,
    /// 各顶层块（按新排版的序号）起始时相对新位置向下的偏移（逻辑像素，向上时为负）。
    pub from: Vec<f32>,
    /// 淡入的项。
    pub fading: Fading,
}

impl BlockMotion {
    /// 返回第 `block` 块当前相对新位置向下的偏移（逻辑像素）。
    pub fn offset(&self, block: usize) -> f32 {
        self.from
            .get(block)
            .map_or(0.0, |d| d * (1.0 - self.clock.progress()))
    }

    /// 是否仍有块未到达新位置。
    pub fn moving(&self) -> bool {
        self.from.iter().any(|d| d.abs() >= 0.5)
    }

    /// 返回第 `block` 块中淡入的项。
    pub fn fading(&self, block: usize) -> &[usize] {
        self.fading
            .binary_search_by_key(&block, |(b, _)| *b)
            .map_or(&[], |i| &self.fading[i].1)
    }
}

impl Default for MarkdownPreview {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            doc: None,
            parsed: None,
            checked: None,
            pending: None,
            scrolls: HashMap::new(),
            layout: None,
            memo: Default::default(),
            images: HashMap::new(),
            embedded: HashMap::new(),
            failed: None,
            images_version: 0,
            loaded: Vec::new(),
            toggled: HashSet::new(),
            toggled_now: None,
            pending_anchor: None,
            links: Vec::new(),
            toc: Vec::new(),
            toc_tops: Vec::new(),
            toc_fade: None,
            toc_picked: None,
            toc_current: None,
            toc_flash: None,
            codes: Vec::new(),
            copy: None,
            expand: None,
            diagram: None,
            code_hover: None,
            area: winc_render::Rect::default(),
            height: 0.0,
            selection: None,
            focused: false,
            menu_source: None,
            find_doc: None,
            find_serial: 0,
            synced: None,
            live_drag: None,
            goal: None,
            followed: None,
            editing: None,
            align: None,
            motion: None,
            #[cfg(test)]
            frozen: None,
            sender,
            receiver,
        }
    }
}

impl MarkdownPreview {
    /// 当前文档预览滚到的位置（逻辑像素）。
    pub fn scroll(&self) -> f32 {
        self.doc
            .and_then(|id| self.scrolls.get(&id))
            .copied()
            .unwrap_or(0.0)
    }

    /// 滚到 `y`（逻辑像素），限制在内容范围内。
    pub fn scroll_to(&mut self, y: f32) {
        let max = (self.height - self.area.h).max(0.0);
        if let Some(id) = self.doc {
            self.scrolls.insert(id, y.clamp(0.0, max));
        }
    }

    /// 忘掉读过的图片（联网的设置变了）：下次画的时候重新排版、重新读。
    pub fn forget_images(&mut self) {
        self.images.clear();
        self.images_version += 1;
    }

    /// 将预览切换为文档 `doc`（不再显示预览时为 `None`）。
    ///
    /// 释放上一个文档的解析结果、排版和图片，再次显示时重新解析和排版；滚动位置保留。约 1 MB 的
    /// 文档，这些合计约 10 MB；单张解码后的图片最多约 26 MB（2560 × 2560 RGBA），
    /// 网络图片再次预览时从磁盘缓存读取。
    fn reset(&mut self, doc: Option<DocumentId>) {
        self.doc = doc;
        self.parsed = None;
        self.checked = None;
        self.pending = None;
        self.layout = None;
        self.memo = Default::default();
        self.find_doc = None;
        self.motion = None;
        self.forget_images();
        self.embedded.clear();
        self.failed = None;
        self.loaded.clear();
    }
}

/// `lines` 中的各行结束后是否处于围栏代码块内。
///
/// 依据 `CommonMark` 统计开、闭围栏：行首最多 3 个空格，之后为至少 3 个反引号或波浪号；
/// 闭围栏与开围栏字符相同、长度不短于开围栏，且其后没有其他字符。
fn in_fence<'a>(lines: impl Iterator<Item = ropey::RopeSlice<'a>>) -> bool {
    let mut open: Option<(u8, usize)> = None;
    for line in lines {
        // 一行在 rope 的同一块里时不复制。
        let text = std::borrow::Cow::<str>::from(line);
        let text = text.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches(' ');
        if text.len() - trimmed.len() > 3 {
            continue;
        }
        let Some(&mark) = trimmed
            .as_bytes()
            .first()
            .filter(|b| matches!(b, b'`' | b'~'))
        else {
            continue;
        };
        let run = trimmed.bytes().take_while(|&b| b == mark).count();
        if run < 3 {
            continue;
        }
        match open {
            None if mark == b'~' || !trimmed[run..].contains('`') => open = Some((mark, run)),
            Some((m, n)) if m == mark && run >= n && trimmed[run..].trim().is_empty() => {
                open = None
            }
            _ => {}
        }
    }
    open.is_some()
}

/// 当前标签是否显示 Markdown 预览，在哪里：分栏的右半边或整个编辑区。
pub(crate) fn shown_area(app: &App) -> Option<winc_render::Rect> {
    let doc = app.editor.active()?;
    // docx 没有源代码可切换，总是以整个编辑区显示阅读视图。
    if doc.unsupported == Some(winc_editor::Unsupported::Docx) {
        return Some(app.layout.editor);
    }
    if doc.preview_kind() != Some(PreviewKind::Markdown) {
        return None;
    }
    match doc.preview_mode() {
        PreviewMode::Split if app.layout.split.w > 0.0 => Some(app.layout.split),
        PreviewMode::Preview => Some(app.layout.editor),
        _ => None,
    }
}

impl App {
    /// 当前标签显示 Markdown 预览且解析结果落后于文字时，在后台解析最新的文字。
    ///
    /// 不再显示预览时释放预览占用的内存，见 [`MarkdownPreview::reset`]。
    ///
    /// 每次绘制前调用。
    pub(crate) fn advance_markdown(&mut self) {
        let shown = shown_area(self).is_some();
        let Some(doc) = self.editor.active().filter(|_| shown) else {
            if self.markdown.doc.is_some() {
                self.markdown.reset(None);
            }
            return;
        };
        let (id, revision) = (doc.id, doc.revision);
        let md = &mut self.markdown;
        if md.doc != Some(id) {
            md.reset(Some(id));
        }
        if md.checked == Some(revision) || md.pending.is_some() {
            return;
        }
        if doc.unsupported == Some(winc_editor::Unsupported::Docx) {
            let Some(path) = doc.path.clone() else {
                return;
            };
            md.pending = Some(revision);
            let (sender, proxy) = (md.sender.clone(), self.proxy.clone());
            winc_editor::background::spawn(move || {
                let result = winc_preview::docx::load(&path)
                    .map(|c| (markdown::parse(&c.markdown), c.media));
                if sender.send(Done::Converted(id, revision, result)).is_ok()
                    && let Some(proxy) = proxy
                {
                    let _ = proxy.send_event(UserEvent::ImageReady);
                }
            });
            return;
        }
        // 克隆 rope 只是多一个引用；拼成整段文字（16 MB 的文件要几毫秒）放到后台做。
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        md.pending = Some(revision);
        let (sender, proxy) = (md.sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let text = rope.to_string();
            let parsed = markdown::parse(&text);
            if sender.send(Done::Parsed(id, revision, parsed)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 收下后台完成的解析、图片。
    pub(crate) fn poll_markdown(&mut self) {
        let mut changed = false;
        while let Ok(done) = self.markdown.receiver.try_recv() {
            let md = &mut self.markdown;
            match done {
                Done::Parsed(id, revision, parsed) => {
                    if md.doc != Some(id) || md.pending != Some(revision) {
                        continue;
                    }
                    md.pending = None;
                    md.checked = Some(revision);
                    md.parsed = Some(Arc::new(parsed));
                }
                Done::Converted(id, revision, result) => {
                    if md.doc != Some(id) || md.pending != Some(revision) {
                        continue;
                    }
                    md.pending = None;
                    md.checked = Some(revision);
                    md.forget_images();
                    md.embedded.clear();
                    match result {
                        Ok((parsed, media)) => {
                            let doc = self.editor.active().and_then(|d| d.path.as_deref());
                            md.embedded = media
                                .into_iter()
                                .filter_map(|(url, bytes)| {
                                    Some((image_path(doc, &url, false)?, bytes))
                                })
                                .collect();
                            md.failed = None;
                            md.parsed = Some(Arc::new(parsed));
                        }
                        Err(reason) => {
                            md.failed = Some(reason);
                            md.parsed = None;
                        }
                    }
                }
                Done::Loaded(path, result) => {
                    // 读取期间图片已作废（切换了文档或联网设置变化），丢弃结果。
                    if !matches!(md.images.get(&path), Some(ImageSlot::Loading)) {
                        continue;
                    }
                    let slot = match result {
                        Ok(picture) => ImageSlot::Ready {
                            picture,
                            pixels: Vec::new(),
                            drawing: Vec::new(),
                        },
                        Err(_) => ImageSlot::Failed,
                    };
                    md.images.insert(path.clone(), slot);
                    md.loaded.push(path);
                }
                Done::Drawn(path, drawn) => {
                    if let Some(ImageSlot::Ready {
                        pixels, drawing, ..
                    }) = md.images.get_mut(&path)
                    {
                        let size = (drawn.width, drawn.height);
                        drawing.retain(|&d| d != size);
                        pixels.retain(|p| (p.width, p.height) != size);
                        pixels.insert(0, drawn);
                        pixels.truncate(SIZES);
                    }
                }
            }
            changed = true;
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 要用图片 `path`（文件里写的地址是 `url`）：还没读过时交给后台读，网址的图片先下载。
    pub(crate) fn request_markdown_image(&mut self, path: &Path, url: &str) {
        if self.markdown.images.contains_key(path) {
            return;
        }
        self.markdown
            .images
            .insert(path.to_path_buf(), ImageSlot::Loading);
        let (sender, proxy, path) = (
            self.markdown.sender.clone(),
            self.proxy.clone(),
            path.to_path_buf(),
        );
        let remote = is_remote(url).then(|| url.to_owned());
        let embedded = self.markdown.embedded.get(&path).cloned();
        let load = move || {
            let result = match (&remote, embedded) {
                (_, Some(bytes)) if image::is_svg(&path) => std::str::from_utf8(&bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|text| svg::load(text, None, Some(system_glyphs()))),
                (_, Some(bytes)) => image::load_bytes(&bytes),
                (Some(url), None) => download(url, &path).and_then(|()| load_downloaded(&path)),
                (None, None) if image::is_svg(&path) => std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| svg::load(&text, path.parent(), Some(system_glyphs()))),
                (None, None) => image::load(&path),
            };
            if sender
                .send(Done::Loaded(path, result.map(Arc::new)))
                .is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 网络图片由 `curl` 下载，最长等待 `REMOTE_TIMEOUT`：在单独的线程上等待，不占用共用的
        // 线程池。
        if is_remote(url) {
            winc_editor::background::spawn_blocking(load);
        } else {
            winc_editor::background::spawn(load);
        }
    }

    /// 请求把图片 `path` 按 `size`（物理像素）绘制：已有该尺寸的像素时不提交。
    ///
    /// 每张图同一时刻只在后台绘制一个尺寸：拖动改变窗口大小时每帧尺寸都不同，正在绘制时不提交，
    /// 那一张完成后界面重绘，届时仍缺的尺寸再提交。
    pub(crate) fn request_markdown_pixels(&mut self, path: &Path, size: (u32, u32)) {
        let Some(ImageSlot::Ready {
            picture,
            pixels,
            drawing,
        }) = self.markdown.images.get_mut(path)
        else {
            return;
        };
        if pixels.iter().any(|p| (p.width, p.height) == size) || !drawing.is_empty() {
            return;
        }
        drawing.push(size);
        let (sender, proxy, picture, path) = (
            self.markdown.sender.clone(),
            self.proxy.clone(),
            Arc::clone(picture),
            path.to_path_buf(),
        );
        winc_editor::background::spawn(move || {
            let pixels = Arc::new(picture.render(size.0, size.1));
            if sender.send(Done::Drawn(path, pixels)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 预览上 `(x, y)` 处的链接序号（见 [`MarkdownPreview::links`]）。
    pub(crate) fn markdown_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.markdown.area.contains(x, y) {
            return None;
        }
        self.markdown
            .links
            .iter()
            .position(|(rect, _)| rect.contains(x, y))
    }

    /// 第 `index` 个链接是否为预览自身的控件（代码块的复制按钮、图的全屏按钮、折叠块的标题、
    /// 勾选框），而非正文中的链接。
    pub(crate) fn markdown_link_is_control(&self, index: usize) -> bool {
        use crate::view::preview::markdown::{COPY, DETAILS, EXPAND, TABLE, TASK};
        self.markdown.links.get(index).is_some_and(|(_, url)| {
            [COPY, DETAILS, EXPAND, TABLE, TASK]
                .iter()
                .any(|prefix| url.starts_with(prefix))
        })
    }

    /// 点了预览里的链接：网址交给浏览器，`#标题` 滚到那个标题，本地文件在编辑器里打开。
    pub(crate) fn open_markdown_link(&mut self, index: usize) {
        let Some((_, url)) = self.markdown.links.get(index).cloned() else {
            return;
        };
        if url == crate::view::preview::markdown::EXPAND {
            self.open_diagram_view();
            return;
        }
        if url == crate::view::preview::markdown::COPY {
            let Some(code) = self.markdown.copy.clone() else {
                return;
            };
            self.set_clipboard(code.to_string(), false);
            self.flash(t!("preview-markdown-code-copied"));
            return;
        }
        if let Some(op) = url.strip_prefix(crate::view::preview::markdown::TABLE) {
            self.live_table_tool(op);
            return;
        }
        if let Some(at) = url.strip_prefix(crate::view::preview::markdown::TASK) {
            if let Ok(at) = at.parse() {
                self.toggle_markdown_task(at);
            }
            return;
        }
        if let Some(id) = url.strip_prefix(crate::view::preview::markdown::DETAILS) {
            if let Ok(id) = u64::from_str_radix(id, 16)
                && !self.markdown.toggled.remove(&id)
            {
                self.markdown.toggled.insert(id);
            }
            self.markdown.toggled_now = Some(url);
            return;
        }
        if let Some(anchor) = url.strip_prefix('#') {
            match crate::view::preview::markdown::anchor_top(self, anchor) {
                Some(y) => self.scroll_markdown_to(y),
                None => self.flash(t!("preview-markdown-heading-not-found")),
            }
            return;
        }
        let local = self
            .editor
            .active()
            .and_then(|d| d.path.as_deref())
            .and_then(|path| local_target(path, &url));
        match local {
            Some(path) if path.is_file() => self.open_markdown_file(&path, &url),
            Some(_) => self.flash(t!("preview-markdown-link-file-not-found")),
            None => {
                if let Err(e) = system::open_url(&url) {
                    self.flash(t!(
                        "preview-markdown-link-open-failed",
                        error = e.to_string()
                    ));
                }
            }
        }
    }

    /// 切换原文 `at` 处（字节偏移）的任务列表勾选框：`[ ]` 与 `[x]` 互换。
    ///
    /// 修改可以撤销；光标和选区不变，可编辑的预览也不因此进入编辑状态。
    /// 该处不是勾选框（解析结果落后于文字）时不修改。
    fn toggle_markdown_task(&mut self, at: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let mark = rope
            .get_byte_slice(at..at.saturating_add(3).min(rope.len_bytes()))
            .map(|s| s.to_string());
        let Some(mark) = mark else {
            return;
        };
        let next = match mark.as_str() {
            "[ ]" => "x",
            "[x]" | "[X]" => " ",
            _ => return,
        };
        let (id, pos) = (doc.id, rope.byte_to_char(at) + 1);
        let (selections, primary) = doc.selections();
        let editing = self.markdown.editing == Some(id);
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(pos, pos + 1);
        }
        self.run(winc_editor::Command::Insert(next.to_owned()));
        if let Some(doc) = self.editor.active_mut() {
            // 替换的只有一个字符，其前后的位置均不变。
            doc.set_selections(selections, primary);
        }
        if !editing {
            self.markdown.editing = None;
            self.markdown.followed = self
                .live_caret_source()
                .zip(self.editor.active())
                .map(|(source, d)| (d.id, source, d.revision));
        }
        self.request_redraw();
    }

    /// 点了预览里指向本地文件的链接：在编辑器里打开。
    ///
    /// 也是 Markdown 时按原来的显示方式（分栏或只看预览）显示，`其他.md#标题` 等排好版后滚到
    /// 那个标题。
    fn open_markdown_file(&mut self, path: &Path, url: &str) {
        let mode = self
            .editor
            .active()
            .map(winc_editor::Document::preview_mode);
        let anchor = url
            .split_once('#')
            .map(|(_, anchor)| anchor.to_owned())
            .filter(|a| !a.is_empty());
        self.open_path_then(path, move |app| {
            let Some(doc) = app.editor.active() else {
                return;
            };
            let id = doc.id;
            if let Some(mode) = mode
                && doc.can_preview()
                && doc.preview_mode() == PreviewMode::Source
            {
                app.set_preview_mode(mode);
            }
            if let Some(anchor) = anchor {
                app.markdown.pending_anchor = Some((id, anchor));
            }
        });
    }

    /// 排好版后：有等着跳的标题（[`App::open_markdown_file`]）且就是这个文档的，滚过去。
    pub(crate) fn scroll_to_pending_anchor(&mut self) {
        let md = &mut self.markdown;
        let Some((_, anchor)) = md.pending_anchor.take_if(|(id, _)| md.doc == Some(*id)) else {
            return;
        };
        match crate::view::preview::markdown::anchor_top(self, &anchor) {
            Some(y) => self.scroll_markdown_to(y),
            None => self.flash(t!("preview-markdown-heading-not-found")),
        }
    }

    /// 键盘对着预览时（只看预览或分栏点了右边）按键滚动：↑↓ 一小段，`PageUp` / `PageDown`、
    /// 空格（加 ⇧ 往上）一屏，Home / End、⌘↑ / ⌘↓ 到头到尾；Esc 清掉选区。
    ///
    /// 返回是否处理了这个键。分栏时在预览上打字、回车、删除：焦点回到左边的文字，按普通编辑
    /// 处理。可编辑的预览中按键对着文档，见 [`App::live_key`]。
    pub(in crate::app) fn markdown_key(&mut self, key: &Key, cmd: bool) -> bool {
        let Some(area) = shown_area(self) else {
            return false;
        };
        if self.markdown_live() {
            return self.live_key(key, cmd, area);
        }
        if !self.markdown_focused() {
            return false;
        }
        if *key == Key::Named(NamedKey::Escape) {
            if self.markdown.selection.take().is_some() {
                self.request_redraw();
                return true;
            }
            return false;
        }
        let split = area != self.layout.editor;
        let typing = matches!(
            key,
            Key::Character(c) if c != " "
        ) || matches!(
            key,
            Key::Named(NamedKey::Enter | NamedKey::Backspace | NamedKey::Delete | NamedKey::Tab)
        );
        if split && typing {
            self.markdown.focused = false;
            self.markdown.selection = None;
            return false;
        }
        let step = self.fonts.line_height * 2.0;
        let page = (area.h * 0.9).max(step);
        let back = self.modifiers.shift_key();
        let y = self.markdown.scroll();
        let to = match key {
            Key::Named(NamedKey::ArrowUp) if cmd => 0.0,
            Key::Named(NamedKey::ArrowDown) if cmd => f32::MAX,
            Key::Named(NamedKey::ArrowUp) => y - step,
            Key::Named(NamedKey::ArrowDown) => y + step,
            Key::Named(NamedKey::PageUp) => y - page,
            Key::Named(NamedKey::PageDown) => y + page,
            Key::Named(NamedKey::Space) if back => y - page,
            Key::Named(NamedKey::Space) => y + page,
            Key::Character(c) if c == " " && back => y - page,
            Key::Character(c) if c == " " => y + page,
            Key::Named(NamedKey::Home) => 0.0,
            Key::Named(NamedKey::End) => f32::MAX,
            _ => return false,
        };
        self.scroll_markdown_to(to);
        true
    }

    /// 键盘、查找当前对着预览的文字（选中、复制、查找排出来的字）：分栏时最近一次点在预览上。
    ///
    /// 只看预览时预览可以直接编辑，键盘对着文档（见 [`App::markdown_live`]）；只读的文档仍然
    /// 对着预览。
    pub(crate) fn markdown_focused(&self) -> bool {
        shown_area(self).is_some_and(|a| {
            if a == self.layout.editor {
                !self.markdown_live()
            } else {
                self.markdown.focused
            }
        })
    }

    /// 当前标签是可以直接编辑的 Markdown 预览（只看预览）。
    pub(crate) fn markdown_live(&self) -> bool {
        shown_area(self).is_some_and(|a| a == self.layout.editor)
            && self
                .editor
                .active()
                .is_some_and(|d| d.edits_text() && !d.readonly)
    }

    /// 可编辑的预览中文字光标的原文位置（字节偏移，按现在的文字算）。
    pub(crate) fn live_caret_source(&self) -> Option<usize> {
        let rope = self.editor.active()?.text().rope()?;
        let head = self.editor.active()?.selection.head;
        Some(rope.char_to_byte(head.min(rope.len_chars())))
    }

    /// 可编辑的预览中光标所在的叶子块，见 [`LiveLeaf`]；还没解析或没有叶子块时为 `None`。
    ///
    /// 光标在第一个叶子块之前时取第一个。拖着选字时取按下处（选区的起点）所在的块：拖到别的
    /// 块上时这一块不换回排版，鼠标下面的内容不跳。
    pub(crate) fn live_leaf(&self) -> Option<LiveLeaf> {
        let doc = self.editor.active()?;
        let parsed = self.markdown.parsed.as_ref()?;
        let rope = doc.text().rope()?;
        let len = rope.len_bytes();
        // 按住鼠标期间按按下之前的光标查找，显示原文的块和标记保持不变，鼠标下方的文字不移动；
        // 否则按下时该块切换为原文，文字移位，单击会变成选中一段。未开始编辑时全部按排版显示。
        let caret = match self.markdown.live_drag {
            Some((.., before)) if self.live_dragging() => before?,
            _ => self.live_caret_source()?,
        };
        let delta = len as i64 - parsed.len as i64;
        let shift = |at: usize| usize::try_from(at as i64 + delta).unwrap_or(0).min(len);
        let i = parsed.leaves.partition_point(|l| l.range.start <= caret);
        let line = rope.byte_to_line(caret);
        let line_start = rope.line_to_byte(line);
        let line_end = rope
            .try_line_to_byte(line + 1)
            .map_or(len, |b| b.saturating_sub(1).max(caret))
            .min(len);
        let block_of = |at: usize| {
            parsed
                .ranges
                .iter()
                .rposition(|r| r.start <= at && at < r.end.max(r.start + 1))
        };
        let prev = i.checked_sub(1).map(|k| &parsed.leaves[k]);
        let next = parsed.leaves.get(i);
        let (found, leaf, gap) = match (prev.map(|l| &l.range), next.map(|l| &l.range)) {
            (Some(p), _) if caret <= shift(p.end).max(p.start) => (p, p.start..shift(p.end), false),
            // 光标在块的标记上（引用的 `>`、提示块的 `[!NOTE]`、列表的序号），还没到它的第一个
            // 叶子块：取这个叶子块，从光标所在的行显示原文。
            (_, Some(n)) if block_of(n.start).is_some() && block_of(n.start) == block_of(caret) => {
                (n, line_start.min(n.start)..shift(n.end).max(n.start), false)
            }
            // 光标在叶子块后面的空行上：延到光标所在的那一行，光标在原文的空行上有位置。
            (Some(p), _) => (p, p.start..line_end, true),
            (None, Some(n)) => (n, n.start..shift(n.end).max(n.start), false),
            (None, None) => return None,
        };
        let kind = prev
            .into_iter()
            .chain(next)
            .find(|l| std::ptr::eq(&l.range, found));
        // 块之间的空行上算正文：回车另起一段。
        let prose = gap || kind.is_some_and(|l| l.prose);
        let table = !gap && kind.is_some_and(|l| l.table);
        let block = block_of(found.start)?;
        let range = parsed.ranges[block].start..shift(parsed.ranges[block].end).max(leaf.end);
        // 打字之后、重新解析之前，范围来自上一版文档：起点可能已超出文档，也可能不再落在字符边界
        // 上，先收拢到现在的文档里，后面才能按它们截取原文。
        let snap = |at: usize| rope.char_to_byte(rope.byte_to_char(at.min(len)));
        let leaf = snap(leaf.start)..snap(leaf.end).max(snap(leaf.start));
        let range = snap(range.start)..snap(range.end).max(snap(range.start));
        let last = rope.byte_to_line(leaf.end.min(len));
        let edge =
            (!prose && !table && (line <= rope.byte_to_line(leaf.start) || line + 1 >= last))
                .then_some(line);
        Some(LiveLeaf {
            block,
            range,
            leaf,
            revision: doc.revision,
            prose,
            table,
            at: caret,
            edge,
        })
    }

    /// Markdown 中选中一段文字（一行之内）后粘贴网址 `content`：返回换掉选区的链接
    /// `[文字](网址)`；不该做成链接时为 `None`，按普通粘贴处理。
    ///
    /// 以下情况不生成链接：选中的文字本身是网址（视为替换网址）、选区位于代码块或行内代码中
    /// （其中的链接语法不生效）、有多个选区。
    pub(in crate::app) fn markdown_link_paste(&self, content: &str) -> Option<String> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(PreviewKind::Markdown) || !doc.extra_selections().is_empty() {
            return None;
        }
        let url = content.trim();
        let is_url = |s: &str| {
            ["https://", "http://", "mailto:"]
                .iter()
                .any(|p| s.len() > p.len() && s.starts_with(p))
                && !s.contains(char::is_whitespace)
        };
        if !is_url(url) {
            return None;
        }
        let rope = doc.text().rope()?;
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let selected = rope.slice(from..to).to_string();
        if selected.is_empty() || selected.contains(['\n', '`']) || is_url(selected.trim()) {
            return None;
        }
        // 行内代码里：这一行选区之前的反引号是单数。
        let (from, line) = (rope.char_to_byte(from), rope.char_to_line(from));
        let before = rope.byte_slice(rope.line_to_byte(line)..from).to_string();
        if before.matches('`').count() % 2 == 1 {
            return None;
        }
        // 判断是否位于代码块、公式、HTML 块中时使用当前版本的解析结果；没有该结果时
        // （未显示预览或尚未解析到当前版本）只检查围栏代码块，统计之前的围栏行。
        if self.markdown.checked == Some(doc.revision)
            && let Some(parsed) = self.markdown.parsed.as_ref()
        {
            let i = parsed.leaves.partition_point(|l| l.range.start <= from);
            if let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i])
                && leaf.range.contains(&from)
                && !leaf.prose
            {
                return None;
            }
        } else if in_fence(rope.lines_at(0).take(line)) {
            return None;
        }
        // 网址里有括号时用尖括号包起来，括号不会被当成链接的结尾。
        let url = if url.contains(['(', ')']) {
            format!("<{url}>")
        } else {
            url.to_owned()
        };
        Some(format!("[{selected}]({url})"))
    }

    /// 正在可编辑的预览上拖着选字。
    fn live_dragging(&self) -> bool {
        self.drag == crate::app::Drag::MarkdownText && self.markdown.live_drag.is_some()
    }

    /// 可编辑的预览中 ↑↓ 按显示的行移动光标（加 ⇧ 扩展选区），`PageUp` / `PageDown` 移动一屏；
    /// 正文里的回车、⇧ 回车见 [`App::live_enter`]。
    ///
    /// 其余的键返回 `false`，交给编辑区按普通编辑处理（打字、删除、←→、⌘↑↓ 等都对着文档）。
    fn live_key(&mut self, key: &Key, cmd: bool, area: winc_render::Rect) -> bool {
        // 表格中 ⌘↩ 在下方插入一行，⌘⇧⌫ 删除当前行。
        if cmd && !self.modifiers.alt_key() && self.live_leaf().is_some_and(|l| l.table) {
            let shift = self.modifiers.shift_key();
            match key {
                Key::Named(NamedKey::Enter) if !shift => return self.live_table_tool("row"),
                Key::Named(NamedKey::Backspace) if shift => {
                    return self.live_table_tool("delete-row");
                }
                _ => {}
            }
        }
        if cmd || self.modifiers.alt_key() {
            return false;
        }
        use winc_preview::markdown::cells::Step;
        let shift = self.modifiers.shift_key();
        match key {
            Key::Named(NamedKey::Tab) => {
                return self.live_table_step(if shift { Step::Previous } else { Step::Next })
                    || self.live_list_shift(shift);
            }
            Key::Named(NamedKey::Enter) if !shift && self.live_table_step(Step::Down) => {
                return true;
            }
            Key::Named(NamedKey::Enter) => return self.live_enter(shift),
            Key::Named(NamedKey::Backspace) => return self.live_backspace(),
            Key::Named(NamedKey::ArrowLeft) => return self.live_horizontal(false),
            Key::Named(NamedKey::ArrowRight) => return self.live_horizontal(true),
            _ => {}
        }
        let page = (area.h * 0.9).max(self.fonts.line_height);
        let (down, by) = match key {
            Key::Named(NamedKey::ArrowUp) => (false, None),
            Key::Named(NamedKey::ArrowDown) => (true, None),
            Key::Named(NamedKey::PageUp) => (false, Some(page)),
            Key::Named(NamedKey::PageDown) => (true, Some(page)),
            _ => return false,
        };
        self.live_vertical(down, by);
        true
    }

    /// 重排后开始过渡动画（见 [`BlockMotion`]）。
    ///
    /// 各块从偏移 `from` 处滑动到新位置，`fading` 中的项淡入。
    ///
    /// 没有需要移动或淡入的内容、或系统开启减弱动态效果时不开始动画，直接绘制新的排版。
    pub(crate) fn start_markdown_motion(&mut self, from: Vec<f32>, fading: Fading) {
        let still = from.iter().all(|d| d.abs() < 0.5);
        if still && fading.is_empty() {
            self.markdown.motion = None;
            return;
        }
        #[cfg(test)]
        if let Some(at) = self.markdown.frozen {
            let clock = crate::app::Clock::frozen(at, crate::app::animation::MARKDOWN_MOTION);
            self.markdown.motion = Some(BlockMotion {
                clock,
                from,
                fading,
            });
            return;
        }
        self.markdown.motion = self
            .clock(crate::app::animation::MARKDOWN_MOTION)
            .map(|clock| BlockMotion {
                clock,
                from,
                fading,
            });
    }

    /// 执行表格工具栏或右键菜单「表格」中的操作 `op`。
    ///
    /// `op` 可用的值见 `view::preview::markdown` 的 `table_tools`。修改规则见
    /// `winc_preview::markdown::cells::change`，`delete` 删除整张表，一次撤销即可恢复。
    /// 光标不在表格中、或操作无效（删除表头、删除最后一列）时返回 `false`。
    pub(crate) fn live_table_tool(&mut self, op: &str) -> bool {
        use winc_preview::markdown::Align;
        use winc_preview::markdown::cells::{self, Change};
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let leaf = live.leaf.start..live.leaf.end.min(rope.len_bytes());
        let text = rope.byte_slice(leaf.clone()).to_string();
        let change = match op {
            "row-above" => Change::RowAbove,
            "row" => Change::RowBelow,
            "column-left" => Change::ColumnLeft,
            "column" => Change::ColumnRight,
            "delete-row" => Change::DeleteRow,
            "delete-column" => Change::DeleteColumn,
            "left" => Change::Align(Align::Left),
            "center" => Change::Align(Align::Center),
            "right" => Change::Align(Align::Right),
            "none" => Change::Align(Align::None),
            "delete" => {
                // 删除表格后面的换行，避免留下空行。
                let end = text.trim_end().len();
                let (new, at) = (text[end..].trim_start_matches('\n').to_owned(), 0);
                return self.replace_live_text(leaf.start, &text, &new, at);
            }
            _ => return false,
        };
        let Some((new, at)) = cells::change(&text, caret.saturating_sub(leaf.start), change) else {
            return false;
        };
        self.replace_live_text(leaf.start, &text, &new, at)
    }

    /// 将文档中从 `start` 起的原文 `old` 替换为 `new`。
    ///
    /// 只替换不同的部分，光标移到 `new` 中的 `caret` 处（字节偏移）。
    fn replace_live_text(&mut self, start: usize, old: &str, new: &str, caret: usize) -> bool {
        let prefix = old
            .char_indices()
            .zip(new.chars())
            .find(|((_, a), b)| a != b)
            .map_or(old.len().min(new.len()), |((i, _), _)| i);
        let suffix = old[prefix..]
            .chars()
            .rev()
            .zip(new[prefix..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum::<usize>();
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(start + prefix),
            rope.byte_to_char(start + old.len() - suffix),
        );
        doc.collapse_selections();
        doc.selection = winc_core::Selection::new(a, b);
        self.run(winc_editor::Command::Insert(
            new[prefix..new.len() - suffix].to_owned(),
        ));
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let at = rope.byte_to_char((start + caret).min(rope.len_bytes()));
            doc.selection = winc_core::Selection::point(at);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 返回光标位于表格中时右键菜单的「表格」子菜单；不在表格中时为 `None`。
    pub(in crate::app) fn live_table_menu(
        &self,
    ) -> Option<Vec<crate::app::input::context_menu::MenuEntry>> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        self.live_leaf().filter(|l| l.table)?;
        let item = |op| MenuEntry::Item(MenuCommand::Table(op), true);
        Some(vec![
            item("row-above"),
            item("row"),
            item("column-left"),
            item("column"),
            MenuEntry::Separator,
            item("left"),
            item("center"),
            item("right"),
            MenuEntry::Separator,
            item("delete-row"),
            item("delete-column"),
            item("delete"),
        ])
    }

    /// 光标在表格的单元格里时 Tab、⇧Tab、回车换格（见 `winc_preview::markdown::cells`）。
    ///
    /// 选中目标单元格的内容，随后的输入将替换该内容；在最后一行继续向下时新增一行。不在表格
    /// 中、或位于表头下方的分隔行时返回 `false`，交给编辑区处理（缩进、换行）。
    fn live_table_step(&mut self, step: winc_preview::markdown::cells::Step) -> bool {
        use winc_preview::markdown::cells;
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        if !(live.leaf.start..=live.leaf.end).contains(&caret) {
            return false;
        }
        let text = rope.byte_slice(live.leaf.clone()).to_string();
        let Some(target) = cells::step(&text, caret - live.leaf.start, step) else {
            return false;
        };
        let mut select = target.select;
        if let Some((at, row)) = target.insert {
            let before = rope.len_bytes();
            let at = live.leaf.start + at;
            if let Some(doc) = self.editor.active_mut()
                && let Some(rope) = doc.text().rope()
            {
                let pos = rope.byte_to_char(at);
                doc.collapse_selections();
                doc.selection = winc_core::Selection::point(pos);
            }
            let len = row.len();
            self.run(winc_editor::Command::Insert(row));
            // 文档的行尾是 `\r\n` 时插进去的比 `row` 长。
            let after = self
                .editor
                .active()
                .and_then(|d| d.text().rope())
                .map_or(before, |r| r.len_bytes());
            let grown = (after - before).saturating_sub(len);
            select = select.start + grown..select.end + grown;
        }
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let char_at = |b: usize| rope.byte_to_char((live.leaf.start + b).min(rope.len_bytes()));
            let (a, b) = (char_at(select.start), char_at(select.end));
            doc.goal_cell = None;
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// ←→ 移出光标所在的块、落到块之间的空行上时，跳过这些空行，到下一块的开头或上一块的
    /// 末尾：排好的预览上看不到空行，光标不在看不见的地方停留。
    ///
    /// 后面（前面）再没有块时停在空行上。其余情形返回 `false`，交给编辑区逐字移动。
    fn live_horizontal(&mut self, right: bool) -> bool {
        let extend = self.modifiers.shift_key();
        let Some(live) = self.live_leaf() else {
            return false;
        };
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (sel, chars) = (doc.selection, rope.len_chars());
        if !sel.is_empty() && !extend {
            return false;
        }
        let next = match right {
            true if sel.head < chars => sel.head + 1,
            false if sel.head > 0 => sel.head - 1,
            _ => return false,
        };
        // 正文里没有空行，落到空行上就是出了这一块；代码块里的空行是代码的一部分。
        let byte = rope.char_to_byte(next);
        let outside = live.prose
            || if right {
                byte >= live.leaf.end
            } else {
                byte < live.leaf.start
            };
        let blank = |line: usize| rope.line(line).chars().all(char::is_whitespace);
        let mut line = rope.char_to_line(next);
        if !outside || !blank(line) {
            return false;
        }
        let target = if right {
            while line < rope.len_lines() && blank(line) {
                line += 1;
            }
            if line >= rope.len_lines() {
                return false;
            }
            rope.line_to_char(line)
        } else {
            while blank(line) {
                let Some(up) = line.checked_sub(1) else {
                    return false;
                };
                line = up;
            }
            let text = rope.line(line);
            let ending = text
                .to_string()
                .chars()
                .rev()
                .take_while(|c| matches!(c, '\n' | '\r'))
                .count();
            rope.line_to_char(line) + text.len_chars() - ending
        };
        if let Some(doc) = self.editor.active_mut() {
            doc.goal_cell = None;
            doc.collapse_selections();
            let anchor = if extend { doc.selection.anchor } else { target };
            doc.selection = winc_core::Selection::new(anchor, target);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 正文（段落、标题、块之间的空行）里按回车：另起一段、列表续一项、空项退出一层；按着
    /// ⇧ 时段内换行（见 `winc_preview::markdown::enter`）。
    ///
    /// 光标在代码块、公式、表格、HTML 块里时返回 `false`，按编辑区的方式换行、保持缩进。有选区
    /// 时先换掉选区：从选区开头算光标之前的文字，从选区末尾算之后的。
    fn live_enter(&mut self, shift: bool) -> bool {
        use winc_preview::markdown::enter;
        if !shift && self.live_fence() {
            return true;
        }
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let (from, to) = (rope.char_to_byte(from), rope.char_to_byte(to));
        let line_of = |at: usize| rope.byte_to_line(at);
        let line_text = |line: usize| {
            let text = rope.line(line).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let start = rope.line_to_byte(line_of(from));
        let before = rope.byte_slice(start..from).to_string();
        let to_line = line_of(to);
        let rest = line_text(to_line);
        let after = rest[(to - rope.line_to_byte(to_line)).min(rest.len())..].to_owned();
        let edit = if shift {
            enter::Edit {
                before: 0,
                after: 0,
                text: enter::line_break(&before),
            }
        } else {
            let next = (to_line + 1 < rope.len_lines()).then(|| line_text(to_line + 1));
            let first = line_of(from);
            let above: Vec<String> = (first.saturating_sub(200)..first)
                .rev()
                .map(line_text)
                .collect();
            enter::enter(
                &before,
                &after,
                next.as_deref(),
                above.iter().map(String::as_str),
            )
        };
        let (a, b) = (from - edit.before, (to + edit.after).min(rope.len_bytes()));
        let (a, b) = (rope.byte_to_char(a), rope.byte_to_char(b));
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 补全闭围栏：在围栏行（```` ```rust ````、`$$`）末尾按回车时，光标移到两行围栏之间的空行。
    ///
    /// 规则见 `winc_preview::markdown::enter::fence`。光标位于代码块中间的行、有选区、
    /// 或当前行不是围栏时返回 `false`。刚输入的围栏可能已解析为未闭合的代码块，
    /// 也可能仍是正文段落，因此两种情况都按原文判断。
    fn live_fence(&mut self) -> bool {
        let leaf = self.live_leaf();
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(caret);
        if leaf.is_some_and(|l| !l.prose && rope.byte_to_line(l.leaf.start) != line) {
            return false;
        }
        let byte = rope.char_to_byte(caret);
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..byte).to_string();
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let after = text(line)[byte - start..].to_owned();
        if !before.contains(['`', '~', '$']) {
            return false;
        }
        let below: Vec<String> = (line + 1..rope.len_lines()).map(text).collect();
        let fence =
            winc_preview::markdown::enter::fence(&before, &after, below.iter().map(String::as_str));
        let Some((head, tail)) = fence else {
            return false;
        };
        // 分两次插入：换行符按文档的行尾风格生成，光标停在两段文字之间。
        self.run(winc_editor::Command::Insert(head));
        let at = self.editor.active().map(|d| d.selection.head);
        self.run(winc_editor::Command::Insert(tail));
        if let Some(doc) = self.editor.active_mut()
            && let Some(at) = at
        {
            doc.selection = winc_core::Selection::point(at);
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 按 Tab、⇧Tab 调整列表项的层级：缩进为上一项的子项，或退出一层。
    ///
    /// 规则见 `winc_preview::markdown::enter::shift`，光标和选区随该行移动。不是列表项、
    /// 有多个光标或选区跨行时返回 `false`，交给编辑区按普通缩进处理。
    fn live_list_shift(&mut self, outdent: bool) -> bool {
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let sel = doc.selection;
        let line = rope.char_to_line(sel.head.min(chars));
        if !doc.extra_selections().is_empty() || rope.char_to_line(sel.anchor.min(chars)) != line {
            return false;
        }
        let line_text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let above: Vec<String> = (line.saturating_sub(200)..line)
            .rev()
            .map(line_text)
            .collect();
        let text = line_text(line);
        let Some((range, indent)) =
            winc_preview::markdown::enter::shift(&text, above.iter().map(String::as_str), outdent)
        else {
            return false;
        };
        if text[range.clone()] == indent {
            return true;
        }
        // 缩进只含空格和 Tab，字节数等于字符数。
        let start = rope.line_to_char(line);
        let (a, b) = (start + range.start, start + range.end);
        let moved = |c: usize| {
            if c >= b {
                (c + indent.len()).saturating_sub(range.len())
            } else {
                c.min(a + indent.len())
            }
        };
        let after = winc_core::Selection::new(moved(sel.anchor), moved(sel.head));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(indent));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = after;
        }
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文中的删除键：移除光标前的一层块标记（列表记号、任务框、引用的 `>`、标题的 `#`）。
    ///
    /// 规则见 `winc_preview::markdown::enter::backspace`。有选区、有多个光标、位于代码块等块中、
    /// 或光标前还有文字时返回 `false`，按普通删除处理。
    fn live_backspace(&mut self) -> bool {
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let start = rope.line_to_byte(rope.byte_to_line(caret));
        let before = rope.byte_slice(start..caret).to_string();
        let Some(edit) = winc_preview::markdown::enter::backspace(&before) else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(caret - edit.before),
            rope.byte_to_char(caret),
        );
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 光标往下（`down`）或往上移一个显示行，或者 `by` 逻辑像素；移出第一行、最后一行时到
    /// 文档的开头、末尾。
    fn live_vertical(&mut self, down: bool, by: Option<f32>) {
        let extend = self.modifiers.shift_key();
        let Some(source) = self.live_caret_source() else {
            return;
        };
        let Some(laid) = self.markdown.layout.as_ref() else {
            return;
        };
        let Some((x, y, h)) = laid.caret_point(source) else {
            return;
        };
        let head = self.editor.active().map_or(0, |d| d.selection.head);
        let goal = match self.markdown.goal {
            Some((goal, at)) if at == head => goal,
            _ => x,
        };
        let step = by.unwrap_or(h);
        let sign = if down { 1.0 } else { -1.0 };
        let height = laid.height();
        let screen = |laid: &crate::view::preview::markdown::Laid, v: f32| {
            (
                laid.left + goal,
                self.markdown.area.y - self.markdown.scroll() + v,
            )
        };
        // 块间留白高于一行时，下一行的目标位置落在留白中，取到的最近一行仍是原来的行；
        // 因此每次再前进半行，直到到达其他行或超出内容范围。
        let mut target = y + h / 2.0 + sign * step;
        let mut moved = None;
        while (0.0..=height).contains(&target) {
            let (sx, sy) = screen(laid, target);
            let found = crate::view::preview::markdown::source_at(self, sx, sy);
            moved = found.filter(|&s| {
                let point = self.markdown.layout.as_ref().and_then(|l| l.caret_point(s));
                point.is_none_or(|(_, py, _)| (py - y).abs() >= 0.5)
            });
            if moved.is_some() || by.is_some() {
                break;
            }
            target += sign * h / 2.0;
        }
        // 仍在原来的行上（已是第一行或最后一行）时，移到文档的开头或末尾。
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = match moved {
            Some(s) => rope.byte_to_char(s.min(rope.len_bytes())),
            None if down => rope.len_chars(),
            None => 0,
        };
        doc.goal_cell = None;
        doc.collapse_selections();
        let anchor = if extend { doc.selection.anchor } else { pos };
        doc.selection = winc_core::Selection::new(anchor, pos);
        self.markdown.goal = Some((goal, pos));
        self.markdown.selection = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
    }

    /// 可编辑的预览上按下鼠标：文档的光标放到点中的字上；双击选一个词，三击选原文的一行，
    /// 按着 ⇧ 时把选区延到这里。
    fn click_live(&mut self, x: f32, y: f32) {
        // 按下之前的光标位置：未开始编辑时为 `None`，按住鼠标期间整篇按排版显示。
        let editing = self.editor.active().map(|d| d.id) == self.markdown.editing;
        let before = self.live_caret_source().filter(|_| editing);
        self.preedit.clear();
        self.markdown.selection = None;
        self.markdown.goal = None;
        self.request_redraw();
        let Some(source) = crate::view::preview::markdown::source_at(self, x, y) else {
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.modifiers.shift_key();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        doc.goal_cell = None;
        doc.record_cursors();
        doc.collapse_selections();
        let (origin, by_word) = match count {
            2 => (winc_core::movement::word_at(doc.text(), pos), true),
            3 => {
                let line = doc.text().pos_to_line_col(pos).0;
                let span = doc.text().line_span(line);
                ((span.start, span.end), true)
            }
            _ if shift => ((doc.selection.anchor, doc.selection.anchor), false),
            _ => ((pos, pos), false),
        };
        doc.selection = if by_word {
            winc_core::Selection::new(origin.0, origin.1)
        } else {
            winc_core::Selection::new(origin.0, pos)
        };
        self.markdown.live_drag = Some((origin, by_word, before));
        self.drag = crate::app::Drag::MarkdownText;
        self.start_live_editing();
        self.reset_blink();
    }

    /// 可编辑的预览上拖着选字：选区从按下处延到鼠标下面的字。
    fn drag_live(&mut self, x: f32, y: f32) {
        let Some((origin, by_word, _)) = self.markdown.live_drag else {
            return;
        };
        let Some(source) = crate::view::preview::markdown::source_at(self, x, y) else {
            return;
        };
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        doc.selection = if !by_word {
            winc_core::Selection::new(origin.0, pos)
        } else if pos < origin.0 {
            winc_core::Selection::new(origin.1, winc_core::movement::word_at(doc.text(), pos).0)
        } else {
            winc_core::Selection::new(origin.0, winc_core::movement::word_at(doc.text(), pos).1)
        };
        self.reset_blink();
    }

    /// 可编辑的预览这一帧是否把光标所在的块显示为原文。
    ///
    /// 在预览上点过、移过光标或者打过字之后才显示；刚打开、只是阅读时整篇都是排好的样子，
    /// 不会有一块突然变成原文。排版之前调用。
    pub(crate) fn live_editing(&mut self) -> bool {
        if !self.markdown_live() {
            return false;
        }
        let (Some(source), Some(doc)) = (self.live_caret_source(), self.editor.active()) else {
            return false;
        };
        let now = (doc.id, source, doc.revision);
        if self
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now)
        {
            self.markdown.editing = Some(doc.id);
        }
        self.markdown.editing == Some(doc.id)
    }

    /// 可编辑的预览中是否显示文字光标：在当前文档的预览上点过、移过光标或者打过字之后（见
    /// [`App::live_editing`]）。
    pub(crate) fn live_caret_shown(&self) -> bool {
        let id = self.editor.active().map(|d| d.id);
        id.is_some() && self.markdown.editing == id
    }

    /// 在可编辑的预览中开始编辑（点击、移动光标）：光标所在的块显示原文。
    fn start_live_editing(&mut self) {
        self.markdown.editing = self.editor.active().map(|d| d.id);
    }

    /// 可编辑的预览中光标挪了（打字、点击、移动、查找）：光标不在屏内时滚到能看见，离边缘留
    /// 一行；光标没动、只是滚动时不管。
    ///
    /// 排好版之后、画之前调用。
    pub(crate) fn follow_live_caret(&mut self) {
        let source = self
            .markdown_live()
            .then(|| self.live_caret_source())
            .flatten();
        let (Some(source), Some(doc)) = (source, self.editor.active()) else {
            self.markdown.followed = None;
            return;
        };
        let now = (doc.id, source, doc.revision);
        let moved = self
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now);
        self.markdown.followed = Some(now);
        if !moved || self.markdown.editing != Some(doc.id) {
            return;
        }
        let Some(laid) = self.markdown.layout.as_ref() else {
            return;
        };
        let Some((_, y, h)) = laid.caret_point(source) else {
            return;
        };
        let md = &mut self.markdown;
        let (scroll, view) = (md.scroll(), md.area.h);
        let margin = h.min(view / 4.0);
        if y - margin < scroll {
            md.scroll_to(y - margin);
        } else if y + h + margin > scroll + view {
            md.scroll_to(y + h + margin - view);
        }
    }

    /// 在预览上按下鼠标：拿到焦点，从这里开始选字；双击选一个词，按着 ⇧ 时把选区延到这里。
    ///
    /// 可编辑的预览中改为放文档的光标，见 [`App::click_live`]。
    pub(in crate::app) fn click_markdown(&mut self, x: f32, y: f32) {
        self.focus = crate::app::Focus::Editor;
        // 内容一栏两侧的空白：分栏时把键盘交给预览，但不放光标、不开始选字，已有的选区保留。
        if !crate::view::preview::markdown::in_content(self, x) {
            if !self.markdown_live() {
                self.markdown.focused = true;
                self.request_redraw();
            }
            return;
        }
        if self.markdown_live() {
            self.click_live(x, y);
            return;
        }
        self.markdown.focused = true;
        self.request_redraw();
        let Some(at) = crate::view::preview::markdown::offset_at(self, x, y) else {
            self.markdown.selection = None;
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.modifiers.shift_key();
        self.markdown.selection = match self.markdown.selection {
            _ if count >= 2 => self.markdown_word(at),
            Some((anchor, _)) if shift => Some((anchor, at)),
            _ => Some((at, at)),
        };
        self.drag = crate::app::Drag::MarkdownText;
        // 分栏时左侧的光标移到点中的字上：随后输入的文字（输入会回到左侧）插入在此处。只看
        // 预览时光标不可见，不移动。
        if count == 1
            && !shift
            && shown_area(self).is_some_and(|a| a != self.layout.editor)
            && let Some(source) = self.current_source_at(x, y)
        {
            self.place_caret_at_source(source);
            // 预览不跟着左边滚：点中的字留在鼠标下面。
            if let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) {
                self.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
            }
        }
    }

    /// 文字光标在原文中的位置（字节偏移）；解析结果不是当前版本时为 `None`。
    pub(crate) fn markdown_caret_source(&self) -> Option<usize> {
        let doc = self.editor.active()?;
        if self.markdown.checked != Some(doc.revision) {
            return None;
        }
        let rope = doc.text().rope()?;
        Some(rope.char_to_byte(doc.selection.head.min(rope.len_chars())))
    }

    /// 预览上 `(x, y)` 处的字在原文中的位置（字节偏移）；解析结果不是当前版本时为 `None`
    /// （刚打的字还没解析，位置可能已经错开）。
    fn current_source_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.editor.active()?;
        // docx 的预览由转换得到，位置不对应文档中的文字。
        if doc.unsupported.is_some() || self.markdown.checked != Some(doc.revision) {
            return None;
        }
        crate::view::preview::markdown::source_at(self, x, y)
    }

    /// 把文字的光标放到原文第 `source` 个字节处；光标所在的行看不到时滚到离顶上三分之一处。
    fn place_caret_at_source(&mut self, source: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        doc.goal_cell = None;
        doc.record_cursors();
        doc.collapse_selections();
        doc.selection = winc_core::Selection::point(pos);
        let line = doc.text().pos_to_line_col(pos).0;
        self.scroll_to_line(line, 1.0 / 3.0, true);
        self.request_redraw();
    }

    /// 「在源代码中显示」：光标放到右键点中的字在原文中的位置，键盘回到文字；只看预览时改为
    /// 分栏，预览留在旁边。
    pub(in crate::app) fn reveal_markdown_source(&mut self) {
        let Some(source) = self.markdown.menu_source.take() else {
            return;
        };
        if self.editor.active().map(|d| d.preview_mode()) == Some(PreviewMode::Preview) {
            self.set_preview_mode(PreviewMode::Split);
        }
        self.markdown.focused = false;
        self.focus = crate::app::Focus::Editor;
        self.place_caret_at_source(source);
    }

    /// 处理预览上不在链接上的右键点击，返回是否点在预览上。
    ///
    /// 预览获得焦点，点击位置在选区外时取消选区。可编辑的预览中改为把文档的光标移到这里
    /// （点在文档的选区里时不动）。
    pub(in crate::app) fn markdown_context_target(&mut self, x: f32, y: f32) -> bool {
        if !self.markdown.area.contains(x, y) || self.markdown_link_at(x, y).is_some() {
            return false;
        }
        self.focus = crate::app::Focus::Editor;
        if self.markdown_live() {
            self.markdown.menu_source = crate::view::preview::markdown::source_at(self, x, y);
            if let Some(source) = self.markdown.menu_source
                && let Some(doc) = self.editor.active_mut()
                && let Some(rope) = doc.text().rope()
            {
                let pos = rope.byte_to_char(source.min(rope.len_bytes()));
                if !(doc.selection.from()..=doc.selection.to()).contains(&pos) {
                    doc.selection = winc_core::Selection::point(pos);
                    doc.goal_cell = None;
                }
            }
            self.request_redraw();
            return true;
        }
        self.markdown.focused = true;
        self.markdown.menu_source = self.current_source_at(x, y);
        let at = crate::view::preview::markdown::offset_at(self, x, y);
        let inside = self
            .markdown
            .selection
            .zip(at)
            .is_some_and(|((a, b), at)| (a.min(b)..=a.max(b)).contains(&at));
        if !inside {
            self.markdown.selection = None;
        }
        self.request_redraw();
        true
    }

    /// 双击处的词：连着的字母数字、连着的中日韩文字，或者单个别的字。
    fn markdown_word(&self, at: usize) -> Option<(usize, usize)> {
        let laid = self.markdown.layout.as_ref()?;
        const REACH: usize = 64;
        let from = at.saturating_sub(REACH);
        let text: Vec<char> = laid
            .text(from, (at + REACH).min(laid.chars()))
            .chars()
            .collect();
        let (start, end) = same_class_span(&text, at - from, word_class)?;
        Some((from + start, from + end))
    }

    /// 拖着选字：拖出上下边缘时跟着滚。
    pub(in crate::app) fn drag_markdown(&mut self, x: f32, y: f32) {
        let area = self.markdown.area;
        if y < area.y {
            self.scroll_markdown((area.y - y).min(60.0));
        } else if y > area.bottom() {
            self.scroll_markdown(-(y - area.bottom()).min(60.0));
        }
        let y = y.clamp(area.y, area.bottom() - 1.0);
        if self.markdown_live() {
            self.drag_live(x, y);
            self.request_redraw();
            return;
        }
        if let Some(at) = crate::view::preview::markdown::offset_at(self, x, y)
            && let Some((anchor, _)) = self.markdown.selection
        {
            self.markdown.selection = Some((anchor, at));
        }
        self.request_redraw();
    }

    /// 预览里选中的文字；没选中时返回 `None`。
    pub(crate) fn markdown_selected_text(&self) -> Option<String> {
        let (a, b) = self.markdown.selection?;
        let text = self.markdown.layout.as_ref()?.text(a.min(b), a.max(b));
        (!text.is_empty()).then_some(text)
    }

    /// 选中预览里的全部文字（⌘A）。
    pub(crate) fn markdown_select_all(&mut self) {
        let chars = self.markdown.layout.as_ref().map_or(0, |l| l.chars());
        self.markdown.selection = Some((0, chars));
        self.request_redraw();
    }

    /// 查找用的预览文字（见 [`MarkdownPreview::find_doc`]），没装时现在装；还没排版时返回
    /// `None`。
    pub(crate) fn markdown_find_doc(&mut self) -> Option<&winc_editor::Document> {
        if self.markdown.find_doc.is_none() {
            let text = crate::view::preview::markdown::full_text(self)?;
            let md = &mut self.markdown;
            md.find_serial += 1;
            // 和真文档、差异用的标识都错开。
            let id = DocumentId(u64::MAX - 1 - md.find_serial);
            md.find_doc = Some(winc_editor::Document::from_text(id, &text));
        }
        self.markdown.find_doc.as_ref()
    }

    /// 选中预览里第 `from` 到 `to` 个字符（查找跳到的匹配），滚到看得见：已经在屏上时不动，
    /// 否则滚到离顶上三分之一屏处。
    pub(crate) fn select_in_markdown(&mut self, from: usize, to: usize) {
        self.markdown.selection = Some((from, to));
        if let Some(y) = crate::view::preview::markdown::offset_top(self, from) {
            let md = &mut self.markdown;
            let scroll = md.scroll();
            if y < scroll || y + 40.0 > scroll + md.area.h {
                let y = y - md.area.h / 3.0;
                self.scroll_markdown_to(y);
            }
        }
        self.request_redraw();
    }

    /// 在预览上滚动 `dy`（逻辑像素，向上为正，和滚轮一致）。
    pub(crate) fn scroll_markdown(&mut self, dy: f32) {
        let y = self.markdown.scroll() - dy;
        self.scroll_markdown_to(y);
    }

    /// 点了标题指示条里的第 `i` 个标题：预览滚到它。
    ///
    /// 滚完后把它记为当前所在的一节（见 `MarkdownPreview::toc_picked`），并在正文里给它铺一层
    /// 底色，慢慢淡出，标出跳到了哪里。
    pub(in crate::app) fn press_toc(&mut self, target: TocTarget) {
        let TocTarget::Heading(i) = target else {
            return;
        };
        let Some(&top) = self.markdown.toc_tops.get(i) else {
            return;
        };
        self.scroll_markdown_to(top);
        self.markdown.toc_picked = Some((i, self.markdown.scroll()));
        let clock = self.clock(crate::app::animation::TOC_FLASH);
        self.markdown.toc_flash = clock.map(|clock| (i, clock));
    }

    /// 预览滚到 `y`；分栏时左边的文字跟着滚到对应的地方。
    pub(crate) fn scroll_markdown_to(&mut self, y: f32) {
        self.markdown.scroll_to(y);
        self.request_redraw();
        if shown_area(self).is_none_or(|a| a == self.layout.editor) {
            return;
        }
        let max = self.max_scroll_line();
        let y = self.markdown.scroll();
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active_mut())
        else {
            return;
        };
        let line = laid.line_of_y(y, doc.text().line_count());
        doc.scroll_line = text_row(doc, line).clamp(0.0, max.max(0.0));
        self.markdown.synced = Some((doc.id, doc.scroll_line, laid.height()));
    }

    /// 换显示方式之前：只看预览时预览顶上是原文的第几行（小数），换成源代码、分栏后文字滚到
    /// 这里（[`App::scroll_text_to`]）；不是只看预览、还没排版时为 `None`。
    ///
    /// 换成只看预览时记下排好版后对齐一次（见 [`App::align_markdown_once`]）；光标所在的块等
    /// 在预览上编辑了才显示原文。
    pub(in crate::app) fn leave_markdown_mode(&mut self, mode: PreviewMode) -> Option<f32> {
        let doc = self.editor.active()?;
        let old = doc.preview_mode();
        self.markdown.followed = None;
        self.markdown.editing = None;
        if old == mode || doc.preview_kind() != Some(PreviewKind::Markdown) {
            return None;
        }
        if mode == PreviewMode::Preview {
            self.markdown.align = Some(doc.id);
            return None;
        }
        let laid = self.markdown.layout.as_ref()?;
        (old == PreviewMode::Preview && self.markdown.doc == Some(doc.id))
            .then(|| laid.line_of_y(self.markdown.scroll(), doc.text().line_count()))
    }

    /// 文字滚到原文第 `line` 行（小数）在顶上。
    pub(in crate::app) fn scroll_text_to(&mut self, line: f32) {
        let max = self.max_scroll_line();
        if let Some(doc) = self.editor.active_mut() {
            doc.scroll_line = text_row(doc, line).clamp(0.0, max.max(0.0));
        }
    }

    /// 将预览滚动到文字顶端那一行对应的位置；仅在刚切换为只显示预览后执行一次。
    ///
    /// 位置的计算使用分栏同步滚动的映射。
    ///
    /// 排好版之后、画之前调用。
    pub(crate) fn align_markdown_once(&mut self) {
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) else {
            return;
        };
        if self.markdown.align != Some(doc.id) {
            return;
        }
        self.markdown.align = None;
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
        self.markdown.scroll_to(y);
    }

    /// 分栏时左边的文字滚了、或者预览重排了：右边滚到对应的地方。
    ///
    /// 画预览之前调用。
    pub(crate) fn sync_markdown_scroll(&mut self) {
        let (Some(laid), Some(doc)) = (self.markdown.layout.as_ref(), self.editor.active()) else {
            return;
        };
        let now = (doc.id, doc.scroll_line, laid.height());
        if self.markdown.synced == Some(now) {
            return;
        }
        let y = laid.y_of_line(text_line(doc), doc.text().line_count());
        self.markdown.scroll_to(y);
        self.markdown.synced = Some(now);
    }
}

/// 文字滚到的第一个显示行 `doc.scroll_line` 是原文的第几行（小数：自动换行折成几段的行
/// 按段数分）。
fn text_line(doc: &winc_editor::Document) -> f32 {
    let row = doc.scroll_line.max(0.0);
    let (line, part) = doc.line_and_part(row as usize);
    let rows = doc.rows_of(line).max(1) as f32;
    line as f32 + (part as f32 + row.fract()) / rows
}

/// [`text_line`] 反过来：原文第 `line` 行（小数）是第几个显示行。
fn text_row(doc: &winc_editor::Document, line: f32) -> f32 {
    let last = doc.text().line_count().saturating_sub(1);
    let whole = (line.max(0.0) as usize).min(last);
    let rows = doc.rows_of(whole).max(1) as f32;
    doc.row_of(whole) as f32 + (line - whole as f32).clamp(0.0, 1.0) * rows
}

/// 网址的图片下载到这里缓存，文件名是网址的哈希。
const REMOTE_CACHE: &str = "winc-markdown-images";

/// 下载一张图片最多等多久（秒，传给 `curl` 的 `--max-time`）。
const REMOTE_TIMEOUT: &str = "20";

/// 下载一张图片最大多少字节（20 MB，传给 `curl` 的 `--max-filesize`）。
const REMOTE_MAX: &str = "20000000";

/// `url` 是不是要联网的地址。
fn is_remote(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// 文档 `doc`（没保存过的文档为 `None`）里写的图片地址 `url` 读哪个文件：本地图片见
/// [`local_target`]；网址的图片是下载到的缓存文件，`remote` 为假（不让联网）时返回 `None`。
pub(crate) fn image_path(doc: Option<&Path>, url: &str, remote: bool) -> Option<PathBuf> {
    if is_remote(url) {
        // FNV-1a：同一个网址每次启动都落到同一个文件，下载过的不用再下。
        let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        // 扩展名统一按位图写，读取时按内容判断真实格式。
        return remote.then(|| {
            std::env::temp_dir()
                .join(REMOTE_CACHE)
                .join(format!("{hash:016x}.png"))
        });
    }
    local_target(doc?, url)
}

/// 双击选词时字的类别：连着的字母数字（中日韩文字以外，含 `_`）算一个词，连着的中日韩文字算
/// 一个词；其他字（类别 0）单独成词。Markdown 预览和 PDF 共用。
pub(in crate::app) fn word_class(c: char) -> u8 {
    if c.is_alphanumeric() && (c as u32) < 0x2E80 || c == '_' {
        1
    } else if c.is_alphanumeric() {
        2
    } else {
        0
    }
}

/// `text` 中第 `i` 个字所在的一段同类字（类别由 `class` 给出）的起止序号；类别为 0 的字只取它
/// 自己。`i` 越界时返回 `None`。
pub(in crate::app) fn same_class_span(
    text: &[char],
    i: usize,
    class: impl Fn(char) -> u8,
) -> Option<(usize, usize)> {
    let kind = class(*text.get(i)?);
    if kind == 0 {
        return Some((i, i + 1));
    }
    let start = text[..i]
        .iter()
        .rposition(|&c| class(c) != kind)
        .map_or(0, |p| p + 1);
    let end = text[i..]
        .iter()
        .position(|&c| class(c) != kind)
        .map_or(text.len(), |p| i + p);
    Some((start, end))
}

/// 缓存中的图片多久未使用时删除（30 天）、整个缓存最大多少字节，超过时从最久没用的删起。
pub(in crate::app) const CACHE_AGE: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 3600);

/// 缓存目录占用的磁盘上限（字节），200 MB。
const CACHE_MAX: u64 = 200 * 1024 * 1024;

/// 把 `url` 下载到 `path`。
///
/// 已经下载过时只更新修改时间（清理缓存时按它计算最久未用）。先写到旁边的临时文件，下完再
/// 改名，中途失败不会留下半个文件。每次启动第一次下载前先清理一遍缓存（[`clean_cache`]）。
fn download(url: &str, path: &Path) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or(t!("preview-markdown-cache-folder-invalid"))?;
    static CLEANED: std::sync::Once = std::sync::Once::new();
    CLEANED.call_once(|| clean_cache(dir, std::time::SystemTime::now()));
    if path.is_file() {
        if let Ok(file) = std::fs::File::options().write(true).open(path) {
            let _ = file.set_modified(std::time::SystemTime::now());
        }
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = path.with_extension(format!("part{}", std::process::id()));
    let mut command = std::process::Command::new("curl");
    command
        .args([
            "-fsSL",
            "--max-time",
            REMOTE_TIMEOUT,
            "--max-filesize",
            REMOTE_MAX,
            "-o",
        ])
        .arg(&part)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Windows 上 `curl.exe` 是命令行程序：不弹出控制台窗口。
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(t!("preview-markdown-download-failed").to_owned());
    }
    winc_config::files::replace_file(&part, path).map_err(|e| e.to_string())
}

/// 删掉 `dir` 中超过 [`CACHE_AGE`] 未用过的图片，以及超过一小时未下完的临时文件；剩下的
/// 超过 [`CACHE_MAX`] 时从最久没用的删起。
pub(in crate::app) fn clean_cache(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        let partial = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.starts_with("part"));
        let stale = if partial {
            age > std::time::Duration::from_secs(3600)
        } else {
            age > CACHE_AGE
        };
        if stale {
            let _ = std::fs::remove_file(&path);
        } else if !partial {
            kept.push((age, meta.len(), path));
        }
    }
    let mut total: u64 = kept.iter().map(|k| k.1).sum();
    // 最久没用的排前面。
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, len, path) in kept {
        if total <= CACHE_MAX {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// 读下载来的图片：内容是 SVG（徽章大多是）时按 SVG 读，否则按位图读。
///
/// SVG 中的文字（徽章上的字）用系统字体绘制，与直接预览 SVG 文件相同。
fn load_downloaded(path: &Path) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(4).any(|w| w == b"<svg") {
        let text = String::from_utf8_lossy(&bytes);
        return svg::load(&text, None, Some(system_glyphs()));
    }
    image::load(path)
}

/// 文档 `doc` 里写的地址 `url` 指向的本地文件：网址、`mailto:` 这类返回 `None`；去掉 `#…`、
/// `?…`，相对路径按文档所在目录算，`%20` 这类转义还原。
///
/// `file://主机/路径` 取其中的路径：本地文件都在编辑区中打开，不交给系统，系统打开 `.app`、
/// `.command` 时会运行它们。
pub(crate) fn local_target(doc: &Path, url: &str) -> Option<PathBuf> {
    let url = match url.strip_prefix("file://") {
        Some(rest) => &rest[rest.find('/')?..],
        None if url.is_empty()
            || url.contains("://")
            || url.starts_with("mailto:")
            || url.starts_with("data:") =>
        {
            return None;
        }
        None => url,
    };
    let path = url.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() {
        return None;
    }
    let path = percent_decode(path);
    let path = Path::new(&path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        doc.parent()?.join(path)
    })
}

/// 还原网址里 `%xx` 形式的转义；不是合法 UTF-8 时原样返回。
pub(crate) fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests;
