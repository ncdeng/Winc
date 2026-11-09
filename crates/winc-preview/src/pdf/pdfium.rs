//! Windows 上用 PDFium（Chromium 的 PDF 引擎）打开和渲染 PDF，并读取文字、读写批注。
//!
//! PDFium 是动态库 `pdfium.dll`：安装包不带它，用户在 winc 中下载后装进安装目录（[`set_dir`]，
//! 见 `install`）；放在 `winc.exe` 旁边也可以。第一次用到时加载；环境变量 `WINC_PDFIUM` 可以指定
//! 动态库的路径。找不到或加载失败时 [`available`] 返回 `false`，调用方改用系统的
//! `Windows.Data.Pdf`，此时只能预览；之后装好了再调用时重新加载，不必重启。
//!
//! 文件整个读进内存后交给 PDFium（`FPDF_LoadMemDocument64`），PDFium 不持有文件句柄：TeX 等
//! 程序重新生成 PDF 时不会因为文件被占用而失败。PDFium 不是线程安全的，进程中的所有调用都在持有
//! [`LOCK`] 时进行；每个窗口的渲染线程各自打开文档，互相等待的时间只有一页的渲染时间。
//!
//! 坐标有两套：页面空间（点，原点在左下角，不含页面的旋转），写入批注时使用；显示坐标
//! （[`PageRect`]，按显示方向，左上角为原点，宽、高各为 1），交给界面做命中测试和绘制，两者的
//! 换算由 PDFium 完成（`FPDF_PageToDevice`），与渲染使用同一套变换。

// 调用 PDFium 的 C 接口需要 `unsafe`：动态库中的函数指针、PDFium 分配的句柄和缓冲区。本模块之外
// 只使用这里提供的安全接口；每个 `unsafe` 块上方的 `// SAFETY:` 注释写明前提。
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use std::ops::Range;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use winc_i18n::t;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
};
use windows::core::{PCSTR, PCWSTR};

use super::{Annotation, AnnotationKind, PageRect, PageSize, TextChar};
use crate::image::Pixels;

/// PDFium 的句柄（文档、页面、文字页、批注、位图）：不透明指针。
type Handle = *mut c_void;

/// 换算显示坐标时使用的设备大小（像素）：`FPDF_PageToDevice` 输出整数，取较大的值保证精度。
const DEVICE: c_int = 1 << 20;
/// `FPDF_ERR_PASSWORD`：文档需要密码。
const ERR_PASSWORD: c_ulong = 4;
/// `FPDFBitmap_BGRA`：每像素 4 字节的位图格式。
const BITMAP_BGRA: c_int = 4;
/// 渲染标志 `FPDF_ANNOT`（画出批注）与 `FPDF_REVERSE_BYTE_ORDER`（按 RGBA 顺序输出像素）。
const RENDER_FLAGS: c_int = 0x01 | 0x10;
/// `FPDF_INCREMENTAL`：增量保存，原有内容不变，修改追加在文件末尾。
const SAVE_INCREMENTAL: c_ulong = 1;
/// `FPDF_ANNOT_FLAG_PRINT`：打印时也画出批注。
const FLAG_PRINT: c_int = 1 << 2;
/// `FPDFANNOT_COLORTYPE_Color`：批注的主颜色。
const COLOR: c_int = 0;

/// 批注子类型（`FPDF_ANNOT_*`）。
const SUBTYPE_TEXT: c_int = 1;
const SUBTYPE_FREE_TEXT: c_int = 3;
const SUBTYPE_SQUARE: c_int = 4;
const SUBTYPE_CIRCLE: c_int = 5;
const SUBTYPE_HIGHLIGHT: c_int = 9;
const SUBTYPE_UNDERLINE: c_int = 10;
const SUBTYPE_SQUIGGLY: c_int = 11;
const SUBTYPE_STRIKEOUT: c_int = 12;
const SUBTYPE_INK: c_int = 15;

/// `FS_RECTF`：页面空间中的矩形。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct RectF {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

/// `FS_SIZEF`：页面大小（点）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct SizeF {
    width: f32,
    height: f32,
}

/// `FS_QUADPOINTSF`：一组四边形顶点，依次为左上、右上、左下、右下。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct QuadPoints {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
}

/// `FPDF_LIBRARY_CONFIG` 的第 2 版。
#[repr(C)]
struct LibraryConfig {
    version: c_int,
    user_font_paths: *const *const c_char,
    isolate: *mut c_void,
    v8_embedder_slot: c_uint,
}

/// `FPDF_FILEWRITE`：保存时 PDFium 逐块调用 `write_block` 输出文件内容。
#[repr(C)]
struct FileWrite {
    version: c_int,
    write_block: unsafe extern "C" fn(*mut FileWrite, *const c_void, c_ulong) -> c_int,
}

/// 保存时收集输出的缓冲区：`base` 必须是第一个字段，回调把收到的 `FileWrite` 指针换回本结构。
#[repr(C)]
struct Writer {
    base: FileWrite,
    out: Vec<u8>,
}

/// 用到的 PDFium 函数。
struct Api {
    init: unsafe extern "system" fn(*const LibraryConfig),
    load_mem_document64: unsafe extern "system" fn(*const c_void, usize, *const c_char) -> Handle,
    get_last_error: unsafe extern "system" fn() -> c_ulong,
    close_document: unsafe extern "system" fn(Handle),
    get_page_count: unsafe extern "system" fn(Handle) -> c_int,
    get_page_size_by_index_f: unsafe extern "system" fn(Handle, c_int, *mut SizeF) -> c_int,
    load_page: unsafe extern "system" fn(Handle, c_int) -> Handle,
    close_page: unsafe extern "system" fn(Handle),
    bitmap_create_ex: unsafe extern "system" fn(c_int, c_int, c_int, *mut c_void, c_int) -> Handle,
    bitmap_fill_rect:
        unsafe extern "system" fn(Handle, c_int, c_int, c_int, c_int, c_ulong) -> c_int,
    bitmap_get_buffer: unsafe extern "system" fn(Handle) -> *mut c_void,
    bitmap_get_stride: unsafe extern "system" fn(Handle) -> c_int,
    bitmap_destroy: unsafe extern "system" fn(Handle),
    render_page_bitmap:
        unsafe extern "system" fn(Handle, Handle, c_int, c_int, c_int, c_int, c_int, c_int),
    page_to_device: unsafe extern "system" fn(
        Handle,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        f64,
        f64,
        *mut c_int,
        *mut c_int,
    ) -> c_int,
    text_load_page: unsafe extern "system" fn(Handle) -> Handle,
    text_close_page: unsafe extern "system" fn(Handle),
    text_count_chars: unsafe extern "system" fn(Handle) -> c_int,
    text_get_unicode: unsafe extern "system" fn(Handle, c_int) -> c_uint,
    text_get_loose_char_box: unsafe extern "system" fn(Handle, c_int, *mut RectF) -> c_int,
    text_is_generated: unsafe extern "system" fn(Handle, c_int) -> c_int,
    page_create_annot: unsafe extern "system" fn(Handle, c_int) -> Handle,
    page_get_annot_count: unsafe extern "system" fn(Handle) -> c_int,
    page_get_annot: unsafe extern "system" fn(Handle, c_int) -> Handle,
    page_close_annot: unsafe extern "system" fn(Handle),
    page_remove_annot: unsafe extern "system" fn(Handle, c_int) -> c_int,
    annot_get_subtype: unsafe extern "system" fn(Handle) -> c_int,
    annot_set_color:
        unsafe extern "system" fn(Handle, c_int, c_uint, c_uint, c_uint, c_uint) -> c_int,
    annot_get_color: unsafe extern "system" fn(
        Handle,
        c_int,
        *mut c_uint,
        *mut c_uint,
        *mut c_uint,
        *mut c_uint,
    ) -> c_int,
    annot_append_attachment_points: unsafe extern "system" fn(Handle, *const QuadPoints) -> c_int,
    annot_count_attachment_points: unsafe extern "system" fn(Handle) -> usize,
    annot_get_attachment_points: unsafe extern "system" fn(Handle, usize, *mut QuadPoints) -> c_int,
    annot_set_rect: unsafe extern "system" fn(Handle, *const RectF) -> c_int,
    annot_get_rect: unsafe extern "system" fn(Handle, *mut RectF) -> c_int,
    annot_set_string_value: unsafe extern "system" fn(Handle, *const c_char, *const u16) -> c_int,
    annot_get_string_value:
        unsafe extern "system" fn(Handle, *const c_char, *mut u16, c_ulong) -> c_ulong,
    annot_set_flags: unsafe extern "system" fn(Handle, c_int) -> c_int,
    save_as_copy: unsafe extern "system" fn(Handle, *mut FileWrite, c_ulong) -> c_int,
}

/// 串行执行所有 PDFium 调用的锁。
static LOCK: Mutex<()> = Mutex::new(());

/// 加载好的 PDFium。加载成功前每次用到都重新查找：用户在运行中安装了 PDFium（见
/// [`install`]）后不必重启。
static API: OnceLock<Api> = OnceLock::new();

/// 加载 PDFium 时持有的锁：几个线程同时首次用到时只加载、初始化一次。
static LOADING: Mutex<()> = Mutex::new(());

/// 安装 PDFium 的目录（见 [`set_dir`]）；没有设置时只在可执行文件旁边查找。
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// 取得 [`LOCK`]；持有锁的线程 panic 过时照常取得，PDFium 的状态不受 Rust 的 panic 影响。
fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 加载好的 PDFium；动态库不存在或缺少用到的函数时返回 `None`。
fn api() -> Option<&'static Api> {
    if let Some(api) = API.get() {
        return Some(api);
    }
    let _loading = LOADING.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(api) = API.get() {
        return Some(api);
    }
    let api = load()?;
    Some(API.get_or_init(|| api))
}

/// PDFium 能否使用：动态库存在并加载成功。
pub(super) fn available() -> bool {
    api().is_some()
}

/// 设置安装 PDFium 的目录（应用的数据目录下）：可执行文件旁边没有 `pdfium.dll` 时在这里
/// 查找，[`install`] 也装到这里。只能设置一次，之后的调用不起作用。
pub(super) fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// 安装 PDFium 的目录；没有设置时为 `None`。
pub(super) fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// 动态库的位置：`WINC_PDFIUM` 指定的路径；否则依次是可执行文件旁边、安装目录（[`set_dir`]）
/// 中的 `pdfium.dll`，取第一个存在的。
fn library_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WINC_PDFIUM") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok();
    let beside = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    [beside, dir().map(Path::to_path_buf)]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("pdfium.dll"))
        .find(|path| path.is_file())
}

/// 核对 PDFium 的安装包 `archive`，把其中的动态库装进安装目录（[`set_dir`]；没有设置时装在
/// 可执行文件旁边）；装好后 [`available`] 即为 `true`，不必重启。
pub(super) fn install(archive: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = dir().or_else(|| exe.parent()).unwrap_or(Path::new("."));
    install::install(archive, dir)
}

/// PDFium 安装包的下载地址（固定版本）。
pub(super) const ARCHIVE_URL: &str = install::ARCHIVE_URL;

/// 取出动态库 `lib` 中名为 `name` 的函数。
///
/// # Safety
///
/// `F` 必须是与该函数的 C 签名一致的函数指针类型。
unsafe fn symbol<F: Copy>(lib: HMODULE, name: &CStr) -> Option<F> {
    // SAFETY: `name` 以 NUL 结尾，`lib` 是已加载的模块。
    let found = unsafe { GetProcAddress(lib, PCSTR(name.as_ptr().cast())) }?;
    debug_assert_eq!(size_of::<F>(), size_of_val(&found));
    // SAFETY: 两者都是函数指针，大小相同；签名由调用方保证一致。
    Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, F>(&found) })
}

/// 加载 PDFium 并初始化。
fn load() -> Option<Api> {
    let path = library_path()?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` 是以 NUL 结尾的完整路径；按该路径的目录解析它依赖的动态库，不搜索当前目录。
    let lib = unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH) }
        .ok()?;
    macro_rules! api {
        ($($field:ident = $name:literal),* $(,)?) => {
            Api {
                // SAFETY: 每个字段的类型与 PDFium 头文件中同名函数的签名一致。
                $($field: unsafe { symbol(lib, $name)? },)*
            }
        };
    }
    let api = api! {
        init = c"FPDF_InitLibraryWithConfig",
        load_mem_document64 = c"FPDF_LoadMemDocument64",
        get_last_error = c"FPDF_GetLastError",
        close_document = c"FPDF_CloseDocument",
        get_page_count = c"FPDF_GetPageCount",
        get_page_size_by_index_f = c"FPDF_GetPageSizeByIndexF",
        load_page = c"FPDF_LoadPage",
        close_page = c"FPDF_ClosePage",
        bitmap_create_ex = c"FPDFBitmap_CreateEx",
        bitmap_fill_rect = c"FPDFBitmap_FillRect",
        bitmap_get_buffer = c"FPDFBitmap_GetBuffer",
        bitmap_get_stride = c"FPDFBitmap_GetStride",
        bitmap_destroy = c"FPDFBitmap_Destroy",
        render_page_bitmap = c"FPDF_RenderPageBitmap",
        page_to_device = c"FPDF_PageToDevice",
        text_load_page = c"FPDFText_LoadPage",
        text_close_page = c"FPDFText_ClosePage",
        text_count_chars = c"FPDFText_CountChars",
        text_get_unicode = c"FPDFText_GetUnicode",
        text_get_loose_char_box = c"FPDFText_GetLooseCharBox",
        text_is_generated = c"FPDFText_IsGenerated",
        page_create_annot = c"FPDFPage_CreateAnnot",
        page_get_annot_count = c"FPDFPage_GetAnnotCount",
        page_get_annot = c"FPDFPage_GetAnnot",
        page_close_annot = c"FPDFPage_CloseAnnot",
        page_remove_annot = c"FPDFPage_RemoveAnnot",
        annot_get_subtype = c"FPDFAnnot_GetSubtype",
        annot_set_color = c"FPDFAnnot_SetColor",
        annot_get_color = c"FPDFAnnot_GetColor",
        annot_append_attachment_points = c"FPDFAnnot_AppendAttachmentPoints",
        annot_count_attachment_points = c"FPDFAnnot_CountAttachmentPoints",
        annot_get_attachment_points = c"FPDFAnnot_GetAttachmentPoints",
        annot_set_rect = c"FPDFAnnot_SetRect",
        annot_get_rect = c"FPDFAnnot_GetRect",
        annot_set_string_value = c"FPDFAnnot_SetStringValue",
        annot_get_string_value = c"FPDFAnnot_GetStringValue",
        annot_set_flags = c"FPDFAnnot_SetFlags",
        save_as_copy = c"FPDF_SaveAsCopy",
    };
    let config = LibraryConfig {
        version: 2,
        user_font_paths: std::ptr::null(),
        isolate: std::ptr::null_mut(),
        v8_embedder_slot: 0,
    };
    let _guard = lock();
    // SAFETY: 配置结构按第 2 版填写；整个进程只初始化一次（`API` 只加载一次）。
    unsafe { (api.init)(&config) };
    Some(api)
}

/// 打开失败时给用户看的原因。
fn invalid() -> String {
    t!("preview-pdf-invalid").to_owned()
}

/// 渲染失败时给用户看的原因。
fn render_failed() -> String {
    t!("preview-pdf-render-failed").to_owned()
}

/// 修改批注失败时给用户看的原因。
fn annotate_failed() -> String {
    t!("preview-pdf-annotate-failed").to_owned()
}

/// 把字符串转成以 NUL 结尾的 UTF-16，`FPDF_WIDESTRING` 用。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// 用 PDFium 打开的 PDF 文档；只能在打开它的线程上使用（句柄是裸指针，不能跨线程）。
pub(super) struct Document {
    api: &'static Api,
    doc: Handle,
    /// 文件内容：PDFium 在文档关闭前一直读取这块内存，因此与文档同生命周期；增量保存后还用来
    /// 认出 PDFium 追加的那一段（见 `indirect`）。
    bytes: Vec<u8>,
}

impl Drop for Document {
    fn drop(&mut self) {
        let _guard = lock();
        // SAFETY: `doc` 是 `FPDF_LoadMemDocument64` 返回的有效句柄，只关闭一次。
        unsafe { (self.api.close_document)(self.doc) };
    }
}

/// PDFium 的一个句柄（页面、文字页、批注或位图），离开作用域时用 `close` 释放；只在持有
/// [`LOCK`] 时创建和丢弃。
struct Owned {
    handle: Handle,
    close: unsafe extern "system" fn(Handle),
}

impl Owned {
    /// 包装 `handle`，由 `close` 释放；PDFium 返回空指针（失败）时为 `None`。
    fn new(handle: Handle, close: unsafe extern "system" fn(Handle)) -> Option<Owned> {
        (!handle.is_null()).then_some(Owned { handle, close })
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `handle` 是与 `close` 配对的 PDFium 函数返回的有效句柄，只释放一次；调用方
        // 持有锁。
        unsafe { (self.close)(self.handle) };
    }
}

/// 页面上的一个字：字符，以及页面空间中的外框（PDFium 补出的空格、换行没有外框）。
struct RawChar {
    ch: char,
    bounds: Option<RectF>,
}

impl Document {
    /// 打开 `path`，返回文档和各页大小。
    pub(super) fn open(path: &Path) -> Result<(Document, Vec<PageSize>), String> {
        let api = api().ok_or_else(invalid)?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let doc = {
            let _guard = lock();
            // SAFETY: `bytes` 在文档关闭前不会释放或移动（保存在 `Document` 中，Vec 的堆内存
            // 不随结构体移动）；没有密码时传空指针。
            let doc = unsafe {
                (api.load_mem_document64)(bytes.as_ptr().cast(), bytes.len(), std::ptr::null())
            };
            if doc.is_null() {
                // SAFETY: 紧接在失败的调用之后读取错误码。
                let code = unsafe { (api.get_last_error)() };
                return Err(if code == ERR_PASSWORD {
                    t!("preview-pdf-encrypted").to_owned()
                } else {
                    invalid()
                });
            }
            doc
        };
        let document = Document { api, doc, bytes };
        let pages = document.page_sizes()?;
        Ok((document, pages))
    }

    /// 各页按显示方向的大小（点）。
    fn page_sizes(&self) -> Result<Vec<PageSize>, String> {
        let _guard = lock();
        // SAFETY: `doc` 是有效的文档句柄。
        let count = unsafe { (self.api.get_page_count)(self.doc) }.max(0);
        let mut pages = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut size = SizeF::default();
            // SAFETY: 页号在范围内，`size` 是可写的输出参数。
            if unsafe { (self.api.get_page_size_by_index_f)(self.doc, index, &mut size) } == 0 {
                return Err(invalid());
            }
            pages.push(PageSize {
                width: f64::from(size.width),
                height: f64::from(size.height),
            });
        }
        Ok(pages)
    }

    /// 加载第 `index` 页；调用方持有锁。
    fn page(&self, index: usize) -> Option<Owned> {
        let index = c_int::try_from(index).ok()?;
        // SAFETY: `doc` 是有效的文档句柄；页号越界时 PDFium 返回空指针。
        let page = unsafe { (self.api.load_page)(self.doc, index) };
        Owned::new(page, self.api.close_page)
    }

    /// 读取一页的文字信息；调用方持有锁。
    fn text_page(&self, page: &Owned) -> Option<Owned> {
        // SAFETY: `page` 是有效的页面句柄。
        let text = unsafe { (self.api.text_load_page)(page.handle) };
        Owned::new(text, self.api.text_close_page)
    }

    /// 页面空间中的点换成显示坐标（0 到 1）；调用方持有锁。
    fn to_display(&self, page: &Owned, x: f32, y: f32) -> (f32, f32) {
        let (mut dx, mut dy) = (0, 0);
        // SAFETY: `page.handle` 是有效的页面句柄，输出参数可写。
        unsafe {
            (self.api.page_to_device)(
                page.handle,
                0,
                0,
                DEVICE,
                DEVICE,
                0,
                f64::from(x),
                f64::from(y),
                &mut dx,
                &mut dy,
            )
        };
        (dx as f32 / DEVICE as f32, dy as f32 / DEVICE as f32)
    }

    /// 页面空间中的矩形换成显示坐标；页面旋转时两个角的先后可能颠倒，取最小、最大值。
    fn rect_to_display(&self, page: &Owned, rect: RectF) -> PageRect {
        let (x1, y1) = self.to_display(page, rect.left, rect.top);
        let (x2, y2) = self.to_display(page, rect.right, rect.bottom);
        PageRect {
            left: x1.min(x2),
            top: y1.min(y2),
            right: x1.max(x2),
            bottom: y1.max(y2),
        }
    }

    /// 一页上的所有字，按 PDFium 的阅读顺序；调用方持有锁。
    ///
    /// PDFium 按 UTF-16 编号字符：数学字母（`𝑥` 等）由两个代理项组成，这里合成一个字。
    fn chars(&self, text: &Owned) -> Vec<RawChar> {
        // SAFETY: `text.handle` 是有效的文字页句柄。
        let count = unsafe { (self.api.text_count_chars)(text.handle) }.max(0);
        let mut chars = Vec::with_capacity(count as usize);
        let mut i = 0;
        while i < count {
            // SAFETY: 编号在范围内。
            let unit = unsafe { (self.api.text_get_unicode)(text.handle, i) };
            let mut next = i + 1;
            let ch = if (0xD800..0xDC00).contains(&unit) && next < count {
                // SAFETY: 编号在范围内。
                let low = unsafe { (self.api.text_get_unicode)(text.handle, next) };
                if (0xDC00..0xE000).contains(&low) {
                    next += 1;
                    char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
                } else {
                    None
                }
            } else {
                char::from_u32(unit)
            }
            .unwrap_or(char::REPLACEMENT_CHARACTER);
            // SAFETY: 编号在范围内。
            let generated = unsafe { (self.api.text_is_generated)(text.handle, i) } == 1;
            let mut rect = RectF::default();
            // SAFETY: 编号在范围内，`rect` 是可写的输出参数。
            let boxed = unsafe { (self.api.text_get_loose_char_box)(text.handle, i, &mut rect) };
            let sized = rect.right > rect.left && rect.top > rect.bottom;
            let bounds = (boxed != 0 && sized && !generated && !ch.is_control()).then_some(rect);
            chars.push(RawChar { ch, bounds });
            i = next;
        }
        chars
    }

    /// 把第 `index` 页画成 `width × height` 像素的位图，画出批注。
    pub(super) fn render(&self, index: usize, width: u32, height: u32) -> Result<Pixels, String> {
        let (w, h) = (
            c_int::try_from(width).map_err(|_| render_failed())?,
            c_int::try_from(height).map_err(|_| render_failed())?,
        );
        let _guard = lock();
        let page = self.page(index).ok_or_else(render_failed)?;
        let bitmap = self.draw(&page, w, h).ok_or_else(render_failed)?;
        // SAFETY: `bitmap` 是有效句柄。
        let (buffer, stride) = unsafe {
            (
                (self.api.bitmap_get_buffer)(bitmap.handle),
                (self.api.bitmap_get_stride)(bitmap.handle),
            )
        };
        let row = width as usize * 4;
        let stride = usize::try_from(stride).map_err(|_| render_failed())?;
        if buffer.is_null() || stride < row {
            return Err(render_failed());
        }
        // SAFETY: PDFium 分配的缓冲区共 `stride × height` 字节，在 `bitmap` 销毁前有效。
        let data =
            unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), stride * height as usize) };
        let mut rgba = Vec::with_capacity(row * height as usize);
        for line in data.chunks_exact(stride) {
            rgba.extend_from_slice(&line[..row]);
        }
        // 底色不透明，透明度一律为 255（PDFium 合成时可能留下其他值）。
        for alpha in rgba.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Ok(Pixels {
            width,
            height,
            rgba,
        })
    }

    /// 把 `page` 画进一张新建的 `w × h` 像素位图：先铺不透明的白色底，再画页面和批注；调用方
    /// 持有锁。
    fn draw(&self, page: &Owned, w: c_int, h: c_int) -> Option<Owned> {
        // SAFETY: 尺寸为正；传空指针由 PDFium 分配缓冲区。
        let bitmap =
            unsafe { (self.api.bitmap_create_ex)(w, h, BITMAP_BGRA, std::ptr::null_mut(), 0) };
        let bitmap = Owned::new(bitmap, self.api.bitmap_destroy)?;
        // SAFETY: `bitmap`、`page` 都是有效句柄。
        unsafe {
            (self.api.bitmap_fill_rect)(bitmap.handle, 0, 0, w, h, 0xFFFF_FFFF);
            (self.api.render_page_bitmap)(bitmap.handle, page.handle, 0, 0, w, h, 0, RENDER_FLAGS);
        }
        Some(bitmap)
    }

    /// 第 `index` 页的文字：每个字及其显示坐标中的外框。
    pub(super) fn text(&self, index: usize) -> Result<Vec<TextChar>, String> {
        let _guard = lock();
        let page = self.page(index).ok_or_else(render_failed)?;
        let Some(text) = self.text_page(&page) else {
            return Ok(Vec::new());
        };
        Ok(self
            .chars(&text)
            .into_iter()
            .map(|c| TextChar {
                ch: c.ch,
                rect: c.bounds.map(|b| self.rect_to_display(&page, b)),
            })
            .collect())
    }

    /// 第 `index` 页上的批注（只列出界面认得的种类，以及带注释文字的其他批注）。
    pub(super) fn annotations(&self, index: usize) -> Result<Vec<Annotation>, String> {
        let _guard = lock();
        let page = self.page(index).ok_or_else(render_failed)?;
        // SAFETY: `page.handle` 是有效的页面句柄。
        let count = unsafe { (self.api.page_get_annot_count)(page.handle) }.max(0);
        let mut list = Vec::new();
        for i in 0..count {
            // SAFETY: 编号在范围内。
            let annot = unsafe { (self.api.page_get_annot)(page.handle, i) };
            let Some(annot) = Owned::new(annot, self.api.page_close_annot) else {
                continue;
            };
            // SAFETY: `annot.handle` 是有效的批注句柄。
            let subtype = unsafe { (self.api.annot_get_subtype)(annot.handle) };
            let comment = self.string_value(&annot, c"Contents");
            let kind = match subtype {
                SUBTYPE_HIGHLIGHT => AnnotationKind::Highlight,
                SUBTYPE_UNDERLINE => AnnotationKind::Underline,
                SUBTYPE_STRIKEOUT => AnnotationKind::StrikeOut,
                SUBTYPE_SQUIGGLY => AnnotationKind::Squiggly,
                SUBTYPE_TEXT => AnnotationKind::Note,
                SUBTYPE_FREE_TEXT | SUBTYPE_SQUARE | SUBTYPE_CIRCLE | SUBTYPE_INK
                    if !comment.is_empty() =>
                {
                    AnnotationKind::Other
                }
                _ => continue,
            };
            let rects = self.annotation_rects(&page, &annot);
            if rects.is_empty() {
                continue;
            }
            let (mut r, mut g, mut b, mut a) = (0, 0, 0, 0);
            // SAFETY: 输出参数可写。
            let colored = unsafe {
                (self.api.annot_get_color)(annot.handle, COLOR, &mut r, &mut g, &mut b, &mut a)
            } != 0;
            list.push(Annotation {
                index: i as usize,
                kind,
                comment,
                rects,
                color: colored.then_some([r as u8, g as u8, b as u8]),
            });
        }
        Ok(list)
    }

    /// 批注占据的区域（显示坐标）：有四边形顶点的按各组顶点，否则按批注的外框。
    fn annotation_rects(&self, page: &Owned, annot: &Owned) -> Vec<PageRect> {
        // SAFETY: `annot.handle` 是有效的批注句柄。
        let quads = unsafe { (self.api.annot_count_attachment_points)(annot.handle) };
        let mut rects = Vec::new();
        for q in 0..quads {
            let mut p = QuadPoints::default();
            // SAFETY: 编号在范围内，`p` 是可写的输出参数。
            if unsafe { (self.api.annot_get_attachment_points)(annot.handle, q, &mut p) } == 0 {
                continue;
            }
            let xs = [p.x1, p.x2, p.x3, p.x4];
            let ys = [p.y1, p.y2, p.y3, p.y4];
            let rect = RectF {
                left: xs.iter().copied().fold(f32::MAX, f32::min),
                right: xs.iter().copied().fold(f32::MIN, f32::max),
                top: ys.iter().copied().fold(f32::MIN, f32::max),
                bottom: ys.iter().copied().fold(f32::MAX, f32::min),
            };
            rects.push(self.rect_to_display(page, rect));
        }
        if rects.is_empty() {
            let mut rect = RectF::default();
            // SAFETY: `rect` 是可写的输出参数。
            if unsafe { (self.api.annot_get_rect)(annot.handle, &mut rect) } != 0 {
                rects.push(self.rect_to_display(page, rect));
            }
        }
        rects
    }

    /// 读取批注字典中键为 `key` 的文字；没有时为空字符串。
    fn string_value(&self, annot: &Owned, key: &CStr) -> String {
        // SAFETY: 缓冲区为空时只返回所需的字节数（含结尾的 NUL）。
        let bytes = unsafe {
            (self.api.annot_get_string_value)(annot.handle, key.as_ptr(), std::ptr::null_mut(), 0)
        };
        let units = (bytes as usize) / 2;
        if units <= 1 {
            return String::new();
        }
        let mut buffer = vec![0u16; units];
        // SAFETY: 缓冲区大小为 `bytes` 字节。
        unsafe {
            (self.api.annot_get_string_value)(
                annot.handle,
                key.as_ptr(),
                buffer.as_mut_ptr(),
                bytes,
            )
        };
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        String::from_utf16_lossy(&buffer)
    }

    /// 给第 `index` 页上序号在 `chars` 范围内的字（[`Document::text`] 中的序号）加一个
    /// `kind` 批注，注释文字为 `comment`（为空时不写）。
    pub(super) fn annotate(
        &mut self,
        index: usize,
        kind: AnnotationKind,
        chars: Range<usize>,
        comment: &str,
    ) -> Result<(), String> {
        let (subtype, [r, g, b]) = match kind {
            AnnotationKind::Highlight => (SUBTYPE_HIGHLIGHT, [255, 226, 64]),
            AnnotationKind::Underline => (SUBTYPE_UNDERLINE, [37, 99, 235]),
            AnnotationKind::StrikeOut => (SUBTYPE_STRIKEOUT, [220, 38, 38]),
            AnnotationKind::Squiggly => (SUBTYPE_SQUIGGLY, [220, 38, 38]),
            AnnotationKind::Note | AnnotationKind::Other => return Err(annotate_failed()),
        };
        let _guard = lock();
        let page = self.page(index).ok_or_else(annotate_failed)?;
        let text = self.text_page(&page).ok_or_else(annotate_failed)?;
        let all = self.chars(&text);
        // 页面空间的 y 轴朝上：取反后按显示坐标的规则分行，再取反换回来。
        let boxes = all
            .get(chars.start.min(all.len())..chars.end.min(all.len()))
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.bounds)
            .map(|b| PageRect {
                top: -b.top,
                bottom: -b.bottom,
                left: b.left,
                right: b.right,
            });
        let lines: Vec<RectF> = super::merge_lines(boxes)
            .into_iter()
            .map(|l| RectF {
                top: -l.top,
                bottom: -l.bottom,
                left: l.left,
                right: l.right,
            })
            .collect();
        if lines.is_empty() {
            return Err(annotate_failed());
        }
        // SAFETY: `page.handle` 是有效的页面句柄。
        let annot = unsafe { (self.api.page_create_annot)(page.handle, subtype) };
        let annot = Owned::new(annot, self.api.page_close_annot).ok_or_else(annotate_failed)?;
        let mut outer = lines[0];
        for line in &lines {
            let quad = QuadPoints {
                x1: line.left,
                y1: line.top,
                x2: line.right,
                y2: line.top,
                x3: line.left,
                y3: line.bottom,
                x4: line.right,
                y4: line.bottom,
            };
            // SAFETY: `quad` 是有效的输入参数。
            if unsafe { (self.api.annot_append_attachment_points)(annot.handle, &quad) } == 0 {
                return Err(annotate_failed());
            }
            outer.left = outer.left.min(line.left);
            outer.right = outer.right.max(line.right);
            outer.top = outer.top.max(line.top);
            outer.bottom = outer.bottom.min(line.bottom);
        }
        // SAFETY: 输入参数有效；字符串以 NUL 结尾。
        let ok = unsafe {
            (self.api.annot_set_color)(annot.handle, COLOR, r, g, b, 255) != 0
                && (self.api.annot_set_rect)(annot.handle, &outer) != 0
                && (self.api.annot_set_flags)(annot.handle, FLAG_PRINT) != 0
                && (self.api.annot_set_string_value)(
                    annot.handle,
                    c"T".as_ptr(),
                    wide("winc").as_ptr(),
                ) != 0
        };
        if !ok {
            return Err(annotate_failed());
        }
        if !comment.is_empty() {
            Self::set_contents(self.api, &annot, comment)?;
        }
        drop(annot);
        self.settle_annotations(&page);
        Ok(())
    }

    /// 让 PDFium 为 `page` 建立批注列表：这时 PDFium 把页面 `/Annots` 中直接写的批注字典
    /// （`FPDFPage_CreateAnnot` 就这样写）转成间接对象，规范要求批注是间接对象。把页面画进
    /// 1 × 1 像素的位图即可触发。保存时这些对象由 PDFium 写出，加密的文件也按加密规则写。
    fn settle_annotations(&self, page: &Owned) {
        let _ = self.draw(page, 1, 1);
    }

    /// 写入批注的注释文字（`Contents`）。
    fn set_contents(api: &Api, annot: &Owned, comment: &str) -> Result<(), String> {
        let text = wide(comment);
        // SAFETY: 键和值都以 NUL 结尾。
        let ok = unsafe {
            (api.annot_set_string_value)(annot.handle, c"Contents".as_ptr(), text.as_ptr())
        };
        if ok == 0 {
            return Err(annotate_failed());
        }
        Ok(())
    }

    /// 改写第 `index` 页上第 `annotation` 个批注（[`Annotation::index`]）的注释文字。
    pub(super) fn set_comment(
        &mut self,
        index: usize,
        annotation: usize,
        comment: &str,
    ) -> Result<(), String> {
        let _guard = lock();
        let page = self.page(index).ok_or_else(annotate_failed)?;
        let i = c_int::try_from(annotation).map_err(|_| annotate_failed())?;
        // SAFETY: `page.handle` 是有效的页面句柄；编号越界时 PDFium 返回空指针。
        let annot = unsafe { (self.api.page_get_annot)(page.handle, i) };
        let annot = Owned::new(annot, self.api.page_close_annot).ok_or_else(annotate_failed)?;
        Self::set_contents(self.api, &annot, comment)?;
        drop(annot);
        self.settle_annotations(&page);
        Ok(())
    }

    /// 删除第 `index` 页上第 `annotation` 个批注（[`Annotation::index`]）。
    pub(super) fn remove_annotation(
        &mut self,
        index: usize,
        annotation: usize,
    ) -> Result<(), String> {
        let _guard = lock();
        let page = self.page(index).ok_or_else(annotate_failed)?;
        let i = c_int::try_from(annotation).map_err(|_| annotate_failed())?;
        // SAFETY: `page.handle` 是有效的页面句柄；编号越界时 PDFium 返回失败。
        if unsafe { (self.api.page_remove_annot)(page.handle, i) } == 0 {
            return Err(annotate_failed());
        }
        Ok(())
    }

    /// 把文档（含修改过的批注）保存成字节：增量保存，原有内容不变，修改追加在后面。
    pub(super) fn save(&self) -> Result<Vec<u8>, String> {
        /// PDFium 的写出回调：把收到的一块追加到 [`Writer::out`]。
        unsafe extern "C" fn write_block(
            this: *mut FileWrite,
            data: *const c_void,
            size: c_ulong,
        ) -> c_int {
            // SAFETY: `this` 指向 `Writer` 的第一个字段（`repr(C)`），保存期间 `Writer` 一直
            // 存活且没有其他引用；`data` 指向 `size` 字节的可读数据。
            let writer = unsafe { &mut *this.cast::<Writer>() };
            // SAFETY: PDFium 保证 `data` 指向 `size` 字节的可读数据，调用期间有效。
            let block = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) };
            writer.out.extend_from_slice(block);
            1
        }

        let mut writer = Writer {
            base: FileWrite {
                version: 1,
                write_block,
            },
            out: Vec::new(),
        };
        let _guard = lock();
        // SAFETY: `doc` 是有效的文档句柄；`writer` 在调用期间存活。
        let ok = unsafe { (self.api.save_as_copy)(self.doc, &mut writer.base, SAVE_INCREMENTAL) };
        if ok == 0 || writer.out.is_empty() {
            return Err(t!("preview-pdf-save-failed").to_owned());
        }
        // 批注改成规范要求的间接对象，没有改动的对象不再重复写入（见 `indirect`）。
        Ok(indirect::tidy_incremental_save(&self.bytes, writer.out))
    }
}

mod indirect;
mod install;

#[cfg(test)]
mod tests;
