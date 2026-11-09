//! `indirect` 的测试：手写 PDFium 风格的增量更新，检查保留下来的对象和重写的交叉引用。

use super::*;

/// 原文件：随便几个字节，以 `%%EOF` 结尾（改写只看追加的那一段）。
const ORIGINAL: &[u8] = b"%PDF-1.7\n1 0 obj\n<</Type/Catalog>>\nendobj\nstartxref\n0\n%%EOF\n";

/// 最后一段交叉引用中每个对象的（对象号，偏移）：只认本模块写出的格式（表，或不压缩、
/// `/W[1 4 2]` 的交叉引用流）。
pub(crate) fn last_xref(data: &[u8]) -> Vec<(u32, usize)> {
    let text = String::from_utf8_lossy(data);
    let start: usize = text
        .rsplit("startxref")
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let section = &data[start..];
    let mut entries = Vec::new();
    if section.starts_with(b"xref") {
        let body = String::from_utf8_lossy(section);
        let mut lines = body.lines().skip(1);
        while let Some(line) = lines.next() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 2 {
                break;
            }
            let (first, count): (u32, u32) = (parts[0].parse().unwrap(), parts[1].parse().unwrap());
            for k in 0..count {
                let entry = lines.next().unwrap();
                // 空闲的条目（`f`，例如删掉的批注）不指向对象。
                if entry.trim_end().ends_with('n') {
                    entries.push((first + k, entry[..10].parse().unwrap()));
                }
            }
        }
        return entries;
    }
    let dict_end = find(section, 0, b">>stream").unwrap();
    let dict = &section[..dict_end + 2];
    let dict = &dict[find(dict, 0, b"<<").unwrap()..];
    let index = dict_entries(dict)
        .unwrap()
        .into_iter()
        .find(|(k, _)| *k == b"Index")
        .unwrap()
        .1;
    let numbers: Vec<u32> = String::from_utf8_lossy(&index[1..index.len() - 1])
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let mut data_at = dict_end + b">>stream\r\n".len();
    for pair in numbers.chunks(2) {
        for k in 0..pair[1] {
            let row = &section[data_at..data_at + 7];
            assert_eq!(row[0], 1);
            let offset = u32::from_be_bytes([row[1], row[2], row[3], row[4]]) as usize;
            entries.push((pair[0] + k, offset));
            data_at += 7;
        }
    }
    entries
}

/// 每个交叉引用条目都指向 `N G obj` 的第一个字节。
pub(crate) fn assert_offsets_point_at_objects(data: &[u8]) {
    let entries = last_xref(data);
    assert!(!entries.is_empty());
    for (number, offset) in entries {
        let head = format!("{number} ");
        assert!(
            data[offset..].starts_with(head.as_bytes())
                && find(&data[offset..offset + 20], 0, b"obj").is_some(),
            "对象 {number} 的偏移 {offset} 处是 {:?}",
            String::from_utf8_lossy(&data[offset..offset + 20])
        );
    }
}

#[test]
fn an_xref_stream_is_written_again_for_the_kept_objects() {
    let mut data = ORIGINAL.to_vec();
    // 照 PDFium 的写法：交叉引用流的字典里没有 `/Type/XRef`，`endstream` 后面没有 `endobj`。
    let rows = [0u8, 0, 0, 70, 0, 0, 0, 0, 200, 0];
    data.extend_from_slice(
        b"\r\n5 0 obj\r\n[ 9 0 R ]\r\nendobj\
          \r\n3 0 obj\r\n<</Annots 5 0 R /Type/Page>>\r\nendobj\
          \r\n12 0 obj <</Root 1 0 R /Size 13/Info 7 0 R/Prev 40/W[0 4 1]/Index[5 1 12 1]\
          /Length 10>>stream\r\n",
    );
    data.extend_from_slice(&rows);
    data.extend_from_slice(b"\r\nendstream\r\nstartxref\r\n200\r\n%%EOF\r\n");
    let out = tidy_incremental_save(ORIGINAL, data);
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("5 0 obj\r\n[ 9 0 R ]"), "{text}");
    assert!(
        text.contains("/Size 13/W[1 4 2]/Index[3 1 5 1 12 1]"),
        "{text}"
    );
    assert!(
        text.contains("/Root 1 0 R/Info 7 0 R/Prev 40/Length"),
        "{text}"
    );
    let numbers: Vec<u32> = last_xref(&out).into_iter().map(|(n, _)| n).collect();
    assert_eq!(numbers, [3, 5, 12]);
    assert_offsets_point_at_objects(&out);
}

#[test]
fn unchanged_objects_are_not_written_again() {
    // PDFium 把读过的字体、内容流、目录都重新写了一遍：只留下带 `/Annots` 的页面和批注。
    let mut data = ORIGINAL.to_vec();
    data.extend_from_slice(
        b"\r\n1 0 obj\r\n<</Pages 2 0 R /Type/Catalog>>\r\nendobj\
          \r\n4 0 obj\r\n<</BaseFont/SimHei/Subtype/Type0/Type/Font>>\r\nendobj\
          \r\n5 0 obj\r\n<</Filter/FlateDecode/Length 4>>stream\r\nabcd\r\nendstream\r\nendobj\
          \r\n3 0 obj\r\n<</Annots[9 0 R]/Type/Page>>\r\nendobj\
          \r\n9 0 obj\r\n<</Contents(new)/Subtype/Highlight/Type/Annot>>\r\nendobj\
          \r\nxref\r\n1 1\r\n0000000070 00000 n\r\ntrailer\r\n<</Size 10/Root 1 0 R/Prev 40\
          /ID[<01><02>]>>\r\nstartxref\r\n70\r\n%%EOF\r\n",
    );
    let out = tidy_incremental_save(ORIGINAL, data);
    assert!(out.starts_with(ORIGINAL));
    let text = String::from_utf8_lossy(&out[ORIGINAL.len()..]);
    assert!(
        text.contains("3 0 obj\r\n<</Annots[9 0 R]/Type/Page>>"),
        "{text}"
    );
    assert!(text.contains("9 0 obj\r\n<</Contents(new)"), "{text}");
    for dropped in ["Catalog", "SimHei", "FlateDecode"] {
        assert!(!text.contains(dropped), "{text}");
    }
    assert!(
        text.contains("trailer\r\n<</Size 10/Root 1 0 R/ID [<01><02>]/Prev 40>>"),
        "{text}"
    );
    let numbers: Vec<u32> = last_xref(&out).into_iter().map(|(n, _)| n).collect();
    assert_eq!(numbers, [3, 9]);
    assert_offsets_point_at_objects(&out);

    // 什么都没改（只有没改动的对象）：只剩原文件。
    let mut data = ORIGINAL.to_vec();
    data.extend_from_slice(
        b"\r\n1 0 obj\r\n<</Pages 2 0 R /Type/Catalog>>\r\nendobj\r\nxref\r\n1 1\r\n\
          0000000070 00000 n\r\ntrailer\r\n<</Size 10/Root 1 0 R>>\r\nstartxref\r\n70\r\n%%EOF\r\n",
    );
    assert_eq!(tidy_incremental_save(ORIGINAL, data), ORIGINAL);
}

#[test]
fn encrypted_documents_drop_unchanged_objects_and_keep_the_key() {
    // 加密的文档：没改动的对象照样去掉；留下的对象原样写出（对象号不变，加密的字符串仍能
    // 解密），尾部字典保留 `/Encrypt`。
    let mut data = ORIGINAL.to_vec();
    data.extend_from_slice(
        b"\r\n4 0 obj\r\n<</BaseFont/SimHei/Subtype/Type0/Type/Font>>\r\nendobj\
          \r\n3 0 obj\r\n<</Annots[9 0 R]/Type/Page>>\r\nendobj\
          \r\n9 0 obj\r\n<</Contents<8f1e2a>/Subtype/Highlight/Type/Annot>>\r\nendobj\
          \r\nxref\r\n3 1\r\n0000000070 00000 n\r\ntrailer\r\n<</Size 10/Root 1 0 R\
          /Encrypt 8 0 R/ID[<01><02>]>>\r\nstartxref\r\n70\r\n%%EOF\r\n",
    );
    let out = tidy_incremental_save(ORIGINAL, data);
    let text = String::from_utf8_lossy(&out[ORIGINAL.len()..]);
    assert!(!text.contains("SimHei"), "{text}");
    assert!(text.contains("9 0 obj\r\n<</Contents<8f1e2a>"), "{text}");
    assert!(text.contains("/Encrypt 8 0 R"), "{text}");
    assert_offsets_point_at_objects(&out);
}

#[test]
fn unknown_structure_keeps_the_output() {
    // 不是在原文件后面追加的内容。
    let other = b"%PDF-1.7 something else".to_vec();
    assert_eq!(tidy_incremental_save(ORIGINAL, other.clone()), other);
    // 结构不完整（缺少交叉引用）。
    let mut broken = ORIGINAL.to_vec();
    broken.extend_from_slice(b"\r\n3 0 obj\r\n<</Annots[<</Type/Annot>>]>>\r\nendobj\r\n");
    assert_eq!(tidy_incremental_save(ORIGINAL, broken.clone()), broken);
}
