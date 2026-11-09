//! PDF 的逐页渲染：用系统的 PDF 渲染器（macOS 的 CoreGraphics、Windows 的
//! `Windows.Data.Pdf`）把一页画成指定大小的位图。
//!
//! Windows 上 `winc.exe` 旁边有 PDFium 的动态库（`pdfium.dll`）时改用 PDFium（见 `pdfium`），
//! 此时还能读取每页的文字（[`Document::text`]）、读写批注（[`Document::annotations`]、
//! [`Document::annotate`]）并保存（[`Document::save`]）；[`Document::editable`] 说明能否这样做。
//! 系统的渲染器只做预览，不解析文字和批注。
//!
//! 打开文档时只读取页数和各页大小；渲染按页、按指定大小进行，调用方只渲染视口附近的页，内存只占
//! 这几页的位图。系统的 PDF 对象不能跨线程使用：[`Document`] 只能在打开它的线程上使用，调用方
//! 通常为每个打开的 PDF 启动一个渲染线程。其他平台不支持，[`supported`] 返回 `false`。

use std::ops::Range;
use std::path::Path;

use crate::image::Pixels;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(windows)]
mod pdfium;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as platform;

/// 一页的大小（点，1/72 英寸）：已按页面的旋转换算，宽高是显示时的方向。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    /// 宽度（点）。
    pub width: f64,
    /// 高度（点）。
    pub height: f64,
}

/// 渲染出的位图每条边的上限（像素）：超过时不渲染。
///
/// 用来限制单页位图的内存：按每像素 4 字节，16384 × 16384 的位图约占 1 GiB。按显示大小渲染时
/// 远达不到该上限。
pub const MAX_SIDE: u32 = 16384;

/// 当前平台能否预览 PDF：macOS 与 Windows 能，其他平台不能。
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

/// `path` 是否是 PDF 文件（按扩展名判断）。
pub fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// 显示坐标中的矩形：按页面的显示方向，左上角为原点，页面的宽、高各为 1。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageRect {
    /// 左边。
    pub left: f32,
    /// 上边。
    pub top: f32,
    /// 右边。
    pub right: f32,
    /// 下边。
    pub bottom: f32,
}

impl PageRect {
    /// 点 `(x, y)`（显示坐标）是否在矩形内。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}

/// 页面上的一个字。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextChar {
    /// 字符；PDF 自身没有、由排版推断出的空格和换行也在其中。
    pub ch: char,
    /// 字的外框（显示坐标）；推断出的空格、换行没有外框。
    pub rect: Option<PageRect>,
}

/// 批注的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnotationKind {
    /// 高亮。
    Highlight,
    /// 下划线。
    Underline,
    /// 删除线。
    StrikeOut,
    /// 波浪线。
    Squiggly,
    /// 附注（页面上的注释图标）。
    Note,
    /// 其他带注释文字的批注（方框、手绘等），只读。
    Other,
}

/// 页面上的一个批注。
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// 在该页批注中的序号，修改、删除时使用；增删批注后会变化，需要重新读取。
    pub index: usize,
    /// 种类。
    pub kind: AnnotationKind,
    /// 注释文字；没有时为空。
    pub comment: String,
    /// 占据的区域（显示坐标）：标记文字的批注每行一个矩形。
    pub rects: Vec<PageRect>,
    /// 颜色（sRGB）；文件中没有写明时为 `None`。
    pub color: Option<[u8; 3]>,
}

/// 实际使用的 PDF 引擎。
#[cfg(any(target_os = "macos", windows))]
enum Backend {
    /// 系统的渲染器：只能预览。
    System(platform::Document),
    /// PDFium：还能读取文字、读写批注。
    #[cfg(windows)]
    Pdfium(pdfium::Document),
}

/// 打开的 PDF 文档；只能在打开它的线程上使用。
pub struct Document {
    #[cfg(any(target_os = "macos", windows))]
    inner: Backend,
    /// 各页的大小，按页序排列。
    pages: Vec<PageSize>,
}

impl Document {
    /// 打开 PDF 文件 `path`，读出页数和各页大小。
    ///
    /// # Errors
    ///
    /// 当前平台不支持、文件无法读取、不是有效的 PDF、加密或没有页时返回给用户看的原因。
    pub fn open(path: &Path) -> Result<Document, String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            #[cfg(windows)]
            let (inner, pages) = if pdfium::available() {
                let (doc, pages) = pdfium::Document::open(path)?;
                (Backend::Pdfium(doc), pages)
            } else {
                let (doc, pages) = platform::Document::open(path)?;
                (Backend::System(doc), pages)
            };
            #[cfg(not(windows))]
            let (inner, pages) = {
                let (doc, pages) = platform::Document::open(path)?;
                (Backend::System(doc), pages)
            };
            if pages.is_empty() {
                return Err(winc_i18n::t!("preview-pdf-empty").to_owned());
            }
            Ok(Document { inner, pages })
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = path;
            Err(winc_i18n::t!("preview-pdf-unsupported").to_owned())
        }
    }

    /// 各页的大小，按页序排列；至少有一页。
    pub fn pages(&self) -> &[PageSize] {
        &self.pages
    }

    /// 把第 `index` 页（从 0 起）画成 `width × height` 像素的位图，白色底。
    ///
    /// 页面按自身比例缩放到位图里，宽高比与 [`PageSize`] 一致时恰好铺满。
    ///
    /// # Errors
    ///
    /// 页号超出范围、尺寸为 0 或超过 [`MAX_SIDE`]，或系统渲染失败时返回原因。
    pub fn render(&self, index: usize, width: u32, height: u32) -> Result<Pixels, String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        #[cfg(any(target_os = "macos", windows))]
        {
            match &self.inner {
                Backend::System(doc) => doc.render(index, width, height),
                #[cfg(windows)]
                Backend::Pdfium(doc) => doc.render(index, width, height),
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Err(winc_i18n::t!("preview-pdf-unsupported").to_owned())
        }
    }

    /// 能否读取文字和读写批注：只有 PDFium 能。
    pub fn editable(&self) -> bool {
        #[cfg(windows)]
        {
            matches!(self.inner, Backend::Pdfium(_))
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 第 `index` 页上的字，按阅读顺序；不能读取文字时为空。
    ///
    /// # Errors
    ///
    /// 页号超出范围或页面无法读取时返回原因。
    pub fn text(&self, index: usize) -> Result<Vec<TextChar>, String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &self.inner {
            return doc.text(index);
        }
        Ok(Vec::new())
    }

    /// 第 `index` 页上的批注；不能读取批注时为空。
    ///
    /// # Errors
    ///
    /// 页号超出范围或页面无法读取时返回原因。
    pub fn annotations(&self, index: usize) -> Result<Vec<Annotation>, String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-render-failed").to_owned());
        }
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &self.inner {
            return doc.annotations(index);
        }
        Ok(Vec::new())
    }

    /// 给第 `index` 页上序号在 `chars` 范围内的字（[`Document::text`] 中的序号）加一个
    /// `kind` 批注，注释文字为 `comment`（为空时不写）。只能加高亮、下划线、删除线、波浪线。
    ///
    /// 只修改内存中的文档，保存见 [`Document::save`]。
    ///
    /// # Errors
    ///
    /// 不能修改批注、种类不支持、范围内没有带外框的字，或 PDF 引擎失败时返回原因。
    pub fn annotate(
        &mut self,
        index: usize,
        kind: AnnotationKind,
        chars: Range<usize>,
        comment: &str,
    ) -> Result<(), String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned());
        }
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &mut self.inner {
            return doc.annotate(index, kind, chars, comment);
        }
        let _ = (kind, chars, comment);
        Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned())
    }

    /// 改写第 `index` 页上第 `annotation` 个批注（[`Annotation::index`]）的注释文字。
    ///
    /// # Errors
    ///
    /// 不能修改批注、批注不存在或 PDF 引擎失败时返回原因。
    pub fn set_comment(
        &mut self,
        index: usize,
        annotation: usize,
        comment: &str,
    ) -> Result<(), String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned());
        }
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &mut self.inner {
            return doc.set_comment(index, annotation, comment);
        }
        let _ = (annotation, comment);
        Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned())
    }

    /// 删除第 `index` 页上第 `annotation` 个批注（[`Annotation::index`]）。
    ///
    /// # Errors
    ///
    /// 不能修改批注、批注不存在或 PDF 引擎失败时返回原因。
    pub fn remove_annotation(&mut self, index: usize, annotation: usize) -> Result<(), String> {
        if index >= self.pages.len() {
            return Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned());
        }
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &mut self.inner {
            return doc.remove_annotation(index, annotation);
        }
        let _ = annotation;
        Err(winc_i18n::t!("preview-pdf-annotate-failed").to_owned())
    }

    /// 把文档（含修改过的批注）保存成字节，由调用方写入文件。
    ///
    /// # Errors
    ///
    /// 不能修改批注或 PDF 引擎保存失败时返回原因。
    pub fn save(&self) -> Result<Vec<u8>, String> {
        #[cfg(windows)]
        if let Backend::Pdfium(doc) = &self.inner {
            return doc.save();
        }
        Err(winc_i18n::t!("preview-pdf-save-failed").to_owned())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
