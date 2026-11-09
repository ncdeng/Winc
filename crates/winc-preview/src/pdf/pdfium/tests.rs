//! `pdfium` 的测试（Windows）：打开手写的 PDF，读出页面大小、文字，加批注、保存后重新打开。
//!
//! 需要 PDFium 的动态库：用 `WINC_PDFIUM` 指定路径，或放在测试可执行文件旁边。找不到时这些测试
//! 直接通过，不做检查（PDFium 是可选的运行时依赖，CI 中没有）；设置了 `WINC_PDFIUM` 却无法加载
//! 时测试失败。

use super::super::{AnnotationKind, Document as Pdf};
use super::available;

/// PDFium 能否使用；设置了 `WINC_PDFIUM` 时必须能加载，否则测试失败而不是跳过。
fn ready() -> bool {
    let ok = available();
    if std::env::var_os("WINC_PDFIUM").is_some() {
        assert!(ok, "WINC_PDFIUM 指定的 PDFium 无法加载");
    }
    ok
}

/// 写一份 PDF：每页一个 `(宽, 高, 旋转)`，左下角 (72, 72) 处有一个 100 × 100 的红色方块，
/// (72, 700) 处用 24 磅的 Helvetica 写着 `Hello world`。交叉引用表按各对象的实际偏移写出。
fn sample_pdf(pages: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages.len())
        .map(|k| format!("{} 0 R", 4 + k * 2))
        .collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_owned());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    let content = "1 0 0 rg 72 72 100 100 re f 0 g BT /F1 24 Tf 72 700 Td (Hello world) Tj ET";
    for (k, (w, h, rotate)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rotate} \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + k * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", k + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// 把 `bytes` 写进临时目录下的 `name`，返回路径。
fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-pdfium-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// 第 0 页文字中 `word` 所在的字序号范围。
fn find(doc: &Pdf, word: &str) -> std::ops::Range<usize> {
    let text: String = doc.text(0).unwrap().iter().map(|c| c.ch).collect();
    let byte = text.find(word).unwrap();
    let start = text[..byte].chars().count();
    start..start + word.chars().count()
}

#[test]
fn pages_report_their_sizes_and_rotation() {
    if !ready() {
        return;
    }
    let path = write("sizes.pdf", &sample_pdf(&[(612, 792, 0), (612, 792, 90)]));
    let doc = Pdf::open(&path).unwrap();
    assert!(doc.editable());
    let pages = doc.pages();
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].width, pages[0].height), (612.0, 792.0));
    // 旋转 90 度的页面按显示方向报告宽高。
    assert_eq!((pages[1].width, pages[1].height), (792.0, 612.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_page_renders_at_the_requested_size_on_white() {
    if !ready() {
        return;
    }
    let path = write("render.pdf", &sample_pdf(&[(200, 200, 0)]));
    let doc = Pdf::open(&path).unwrap();
    // 2 倍：方块占 (144, 144) 到 (344, 344)，位图的第 0 行是页面顶部。
    let pixels = doc.render(0, 400, 400).unwrap();
    assert_eq!((pixels.width, pixels.height), (400, 400));
    let at = |x: usize, y: usize| {
        let i = (y * 400 + x) * 4;
        [
            pixels.rgba[i],
            pixels.rgba[i + 1],
            pixels.rgba[i + 2],
            pixels.rgba[i + 3],
        ]
    };
    assert_eq!(at(10, 390), [255, 255, 255, 255]);
    // 方块的中心在页面坐标 (122, 122)，即位图中 (244, 400 - 244)。
    let [r, g, b, a] = at(244, 156);
    assert!(
        r > 200 && g < 60 && b < 60 && a == 255,
        "{:?}",
        [r, g, b, a]
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn text_comes_with_boxes_in_display_coordinates() {
    if !ready() {
        return;
    }
    let path = write("text.pdf", &sample_pdf(&[(612, 792, 0)]));
    let doc = Pdf::open(&path).unwrap();
    let chars = doc.text(0).unwrap();
    let text: String = chars.iter().map(|c| c.ch).collect();
    assert!(text.contains("Hello world"), "{text:?}");
    let h = chars[find(&doc, "Hello").start].rect.unwrap();
    // `H` 从页面左边 72 点处开始，基线在距底边 700 点处：显示坐标中靠左、靠上。
    assert!((h.left - 72.0 / 612.0).abs() < 0.01, "{h:?}");
    assert!(h.top < 92.0 / 792.0 && h.bottom > 80.0 / 792.0, "{h:?}");
    assert!(h.right > h.left && h.bottom > h.top);
    assert!(doc.text(1).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn annotations_survive_saving_and_can_be_edited_and_removed() {
    if !ready() {
        return;
    }
    let path = write("annotate.pdf", &sample_pdf(&[(612, 792, 0)]));
    let mut doc = Pdf::open(&path).unwrap();
    assert!(doc.annotations(0).unwrap().is_empty());
    let world = find(&doc, "world");
    let word_rect = doc.text(0).unwrap()[world.start].rect.unwrap();
    doc.annotate(0, AnnotationKind::Highlight, world.clone(), "改成 Earth")
        .unwrap();
    doc.annotate(0, AnnotationKind::Underline, find(&doc, "Hello"), "")
        .unwrap();
    // 没有带外框的字：不加批注。
    assert!(
        doc.annotate(0, AnnotationKind::Highlight, 0..0, "")
            .is_err()
    );
    std::fs::write(&path, doc.save().unwrap()).unwrap();
    drop(doc);

    let mut doc = Pdf::open(&path).unwrap();
    let list = doc.annotations(0).unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    let highlight = list
        .iter()
        .find(|a| a.kind == AnnotationKind::Highlight)
        .unwrap();
    assert_eq!(highlight.comment, "改成 Earth");
    assert_eq!(highlight.rects.len(), 1);
    let r = highlight.rects[0];
    assert!(r.contains(
        (word_rect.left + word_rect.right) / 2.0,
        (word_rect.top + word_rect.bottom) / 2.0
    ));
    assert!(
        list.iter()
            .any(|a| a.kind == AnnotationKind::Underline && a.comment.is_empty())
    );

    doc.set_comment(0, highlight.index, "改成 Mars").unwrap();
    let underline = list
        .iter()
        .find(|a| a.kind == AnnotationKind::Underline)
        .unwrap()
        .index;
    doc.remove_annotation(0, underline).unwrap();
    std::fs::write(&path, doc.save().unwrap()).unwrap();
    drop(doc);

    let doc = Pdf::open(&path).unwrap();
    let list = doc.annotations(0).unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].comment, "改成 Mars");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_open_document_does_not_lock_its_file() {
    if !ready() {
        return;
    }
    let path = write("lock.pdf", &sample_pdf(&[(200, 200, 0)]));
    let doc = Pdf::open(&path).unwrap();
    // TeX 重新生成 PDF 时要能覆盖文件。
    std::fs::write(&path, sample_pdf(&[(300, 300, 0)])).unwrap();
    assert_eq!(doc.pages()[0].width, 200.0);
    drop(doc);
    let _ = std::fs::remove_file(&path);
}
