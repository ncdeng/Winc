//! 整理 PDFium 增量保存时追加的那一段：没有改动的对象不再重复写入。
//!
//! PDFium 增量保存时把打开以后读过的所有对象（字体、内容流等）都重新写一遍，文件每保存一次就
//! 大一截（一篇 12 页的论文约 1 MB）。本模块只保留修改批注可能改动的对象（带 `/Annots` 的页面
//! 等字典、批注、数组，以及原文件中没有的新对象），再按新的偏移重新生成这一段的交叉引用（交叉
//! 引用表或交叉引用流，与 PDFium 写出的种类相同）。
//!
//! 保留的对象原样写出、对象号不变：加密的文档中字符串和流按所在对象的对象号加密，照样能解密。
//! 原文件的内容一个字节也不改。追加的那一段里遇到任何不认得的结构（长度写成间接引用的流等），
//! 就原样返回 PDFium 的输出。

use std::fmt::Write as _;

/// 整理 `data`（原文件 `original` 字节之后追加了 PDFium 的增量更新）；无法整理时原样返回。
pub(super) fn tidy_incremental_save(original: &[u8], data: Vec<u8>) -> Vec<u8> {
    if data.len() <= original.len() || !data.starts_with(original) {
        return data;
    }
    match rewrite(original, &data[original.len()..]) {
        Some(section) => {
            let mut out = data;
            out.truncate(original.len());
            out.extend_from_slice(&section);
            out
        }
        None => data,
    }
}

/// 增量更新里的一个对象：对象号、代号和 `obj` 与 `endobj` 之间的内容。
struct Object<'a> {
    number: u32,
    generation: u16,
    body: &'a [u8],
}

/// 增量更新末尾的交叉引用信息：要沿用的尾部字典条目和交叉引用的种类。
struct Trailer<'a> {
    /// `/Root`、`/Info`、`/ID`、`/Encrypt`、`/Prev` 的原文。
    entries: Vec<(&'static str, &'a [u8])>,
    /// 原有的 `/Size`。
    size: u32,
    /// 交叉引用流的对象号；用交叉引用表时为 `None`。
    stream: Option<u32>,
}

/// 原文件最后一段交叉引用中的 `/Size`（原文件中的对象号都小于它）；读不出时为 `None`。
fn original_size(original: &[u8]) -> Option<u32> {
    let at = original.windows(9).rposition(|w| w == b"startxref")?;
    let (offset, _) = integer(original, skip_space(original, at + b"startxref".len()))?;
    let tail = original.get(offset..)?;
    let dict_at = if tail.starts_with(b"xref") {
        skip_space(tail, find(tail, 0, b"trailer")? + b"trailer".len())
    } else {
        skip_space(tail, object_header(tail, 0)?.2)
    };
    let end = skip_value(tail, dict_at)?;
    u32::try_from(dict_number(&tail[dict_at..end], b"Size")?).ok()
}

/// 增量更新中要保留的对象：修改批注可能改动的（带 `/Annots` 的字典、批注、数组），原文件中
/// 没有的新对象（对象号不小于 `original_size`），以及认不出的对象。
fn keep(object: &Object<'_>, original_size: Option<u32>) -> bool {
    if original_size.is_some_and(|size| object.number >= size) || object.body.starts_with(b"[") {
        return true;
    }
    match dict_entries(object.body) {
        Some(entries) => entries
            .iter()
            .any(|(k, v)| *k == b"Annots" || (*k == b"Type" && *v == b"/Annot")),
        None => true,
    }
}

/// 改写增量更新 `section`（接在 `original` 之后）；无法改写时返回 `None`，什么都没改时返回
/// 空的一段。
fn rewrite(original: &[u8], section: &[u8]) -> Option<Vec<u8>> {
    let base = original.len();
    let (objects, trailer) = parse_section(section)?;
    let size = original_size(original);
    let objects: Vec<Object<'_>> = objects.into_iter().filter(|o| keep(o, size)).collect();
    if objects.is_empty() {
        return Some(Vec::new());
    }
    let next = trailer.size.max(
        objects
            .iter()
            .map(|o| o.number + 1)
            .chain(trailer.stream.map(|n| n + 1))
            .max()
            .unwrap_or(0),
    );

    let mut out = Vec::new();
    let mut offsets: Vec<(u32, u16, usize)> = Vec::new();
    // 交叉引用中的偏移指向对象头 `N G obj` 的第一个字节。
    for object in &objects {
        out.extend_from_slice(b"\r\n");
        offsets.push((object.number, object.generation, base + out.len()));
        write!(
            Text(&mut out),
            "{} {} obj\r\n",
            object.number,
            object.generation
        )
        .ok()?;
        out.extend_from_slice(object.body);
        out.extend_from_slice(b"\r\nendobj");
    }
    out.extend_from_slice(b"\r\n");
    match trailer.stream {
        Some(number) => {
            let size = next.max(number + 1);
            offsets.push((number, 0, base + out.len()));
            offsets.sort_by_key(|&(n, _, _)| n);
            let mut data = Vec::with_capacity(offsets.len() * 7);
            for &(_, generation, offset) in &offsets {
                data.push(1);
                data.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
                data.extend_from_slice(&generation.to_be_bytes());
            }
            let start = base + out.len();
            write!(
                Text(&mut out),
                "{number} 0 obj\r\n<</Type/XRef/Size {size}/W[1 4 2]/Index[{}]",
                index(&offsets)
            )
            .ok()?;
            for (key, value) in &trailer.entries {
                out.extend_from_slice(format!("/{key} ").as_bytes());
                out.extend_from_slice(value);
            }
            write!(Text(&mut out), "/Length {}>>stream\r\n", data.len()).ok()?;
            out.extend_from_slice(&data);
            write!(
                Text(&mut out),
                "\r\nendstream\r\nendobj\r\nstartxref\r\n{start}\r\n%%EOF\r\n"
            )
            .ok()?;
        }
        None => {
            offsets.sort_by_key(|&(n, _, _)| n);
            let start = base + out.len();
            out.extend_from_slice(b"xref\r\n");
            for run in runs(&offsets) {
                write!(Text(&mut out), "{} {}\r\n", run[0].0, run.len()).ok()?;
                for &(_, generation, offset) in run {
                    write!(Text(&mut out), "{offset:010} {generation:05} n\r\n").ok()?;
                }
            }
            write!(Text(&mut out), "trailer\r\n<</Size {next}").ok()?;
            for (key, value) in &trailer.entries {
                out.extend_from_slice(format!("/{key} ").as_bytes());
                out.extend_from_slice(value);
            }
            write!(Text(&mut out), ">>\r\nstartxref\r\n{start}\r\n%%EOF\r\n").ok()?;
        }
    }
    Some(out)
}

/// 往字节缓冲里写格式化文字（`write!` 用）。
struct Text<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Text<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// 按对象号排好的条目中连续的几段。
fn runs(sorted: &[(u32, u16, usize)]) -> Vec<&[(u32, u16, usize)]> {
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=sorted.len() {
        if i == sorted.len() || sorted[i].0 != sorted[i - 1].0 + 1 {
            runs.push(&sorted[start..i]);
            start = i;
        }
    }
    runs
}

/// 交叉引用流的 `/Index`：每段连续对象的起始对象号和个数。
fn index(sorted: &[(u32, u16, usize)]) -> String {
    runs(sorted)
        .iter()
        .map(|run| format!("{} {}", run[0].0, run.len()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 解析 PDFium 写出的增量更新：一串对象，最后是交叉引用（表或流）和 `startxref`。
fn parse_section(section: &[u8]) -> Option<(Vec<Object<'_>>, Trailer<'_>)> {
    let mut objects = Vec::new();
    let mut at = skip_space(section, 0);
    loop {
        if section[at..].starts_with(b"xref") {
            let trailer_at = find(section, at, b"trailer")?;
            let dict_at = skip_space(section, trailer_at + b"trailer".len());
            let dict_end = skip_value(section, dict_at)?;
            let trailer = trailer_from(&section[dict_at..dict_end], None)?;
            return Some((objects, trailer));
        }
        let (number, generation, header_end) = object_header(section, at)?;
        let body_start = skip_space(section, header_end);
        let mut end = skip_value(section, body_start)?;
        let value = &section[body_start..end];
        let after = skip_space(section, end);
        if section[after..].starts_with(b"stream") {
            let length = dict_number(value, b"Length")?;
            let mut data = after + b"stream".len();
            if section[data..].starts_with(b"\r\n") {
                data += 2;
            } else if section[data..].starts_with(b"\n") {
                data += 1;
            }
            let data_end = data.checked_add(length)?;
            let close = skip_space(section, data_end);
            if !section.get(close..)?.starts_with(b"endstream") {
                return None;
            }
            end = close + b"endstream".len();
            // 交叉引用流：PDFium 写出的字典里没有 `/Type/XRef`，`endstream` 之后也没有
            // `endobj`，直接是 `startxref`；按有没有 `/W` 认出它。
            let is_xref = dict_name(value, b"Type") == Some(b"XRef".as_slice())
                || dict_entries(value)?.iter().any(|(k, _)| *k == b"W");
            if is_xref {
                let trailer = trailer_from(value, Some(number))?;
                let mut close = skip_space(section, end);
                if section[close..].starts_with(b"endobj") {
                    close = skip_space(section, close + b"endobj".len());
                }
                if !section[close..].starts_with(b"startxref") {
                    return None;
                }
                return Some((objects, trailer));
            }
        }
        let close = skip_space(section, end);
        if !section.get(close..)?.starts_with(b"endobj") {
            return None;
        }
        objects.push(Object {
            number,
            generation,
            body: &section[body_start..end],
        });
        at = skip_space(section, close + b"endobj".len());
        if at >= section.len() {
            return None;
        }
    }
}

/// 从尾部字典（或交叉引用流的字典）`dict` 取出要沿用的条目。
fn trailer_from(dict: &[u8], stream: Option<u32>) -> Option<Trailer<'_>> {
    let entries = dict_entries(dict)?;
    let mut kept = Vec::new();
    for key in ["Root", "Info", "ID", "Encrypt", "Prev"] {
        if let Some((_, value)) = entries.iter().find(|(k, _)| *k == key.as_bytes()) {
            kept.push((key, *value));
        }
    }
    let size = dict_number(dict, b"Size")?;
    Some(Trailer {
        entries: kept,
        size: u32::try_from(size).ok()?,
        stream,
    })
}

/// `at` 处的对象头 `N G obj`：对象号、代号和对象头之后的位置。
fn object_header(data: &[u8], at: usize) -> Option<(u32, u16, usize)> {
    let (number, at) = integer(data, at)?;
    let (generation, at) = integer(data, skip_space(data, at))?;
    let at = skip_space(data, at);
    data[at..].starts_with(b"obj").then_some(())?;
    Some((
        u32::try_from(number).ok()?,
        u16::try_from(generation).ok()?,
        at + b"obj".len(),
    ))
}

/// `at` 处的非负整数和它之后的位置。
fn integer(data: &[u8], at: usize) -> Option<(usize, usize)> {
    let len = data[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    if len == 0 {
        return None;
    }
    let text = std::str::from_utf8(&data[at..at + len]).ok()?;
    Some((text.parse().ok()?, at + len))
}

/// 从 `from` 起第一次出现 `needle` 的位置。
fn find(data: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    data.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// PDF 的空白字符。
fn is_space(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

/// PDF 的分隔字符。
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// 跳过空白和注释。
fn skip_space(data: &[u8], mut at: usize) -> usize {
    while at < data.len() {
        if is_space(data[at]) {
            at += 1;
        } else if data[at] == b'%' {
            while at < data.len() && data[at] != b'\n' && data[at] != b'\r' {
                at += 1;
            }
        } else {
            break;
        }
    }
    at
}

/// 跳过 `at` 处的一个值（字典、数组、字符串、名字、数字、间接引用或关键字），返回它之后的
/// 位置；无法解析时返回 `None`。
fn skip_value(data: &[u8], at: usize) -> Option<usize> {
    let first = *data.get(at)?;
    match first {
        b'<' if data.get(at + 1) == Some(&b'<') => {
            let mut at = skip_space(data, at + 2);
            loop {
                if data.get(at..)?.starts_with(b">>") {
                    return Some(at + 2);
                }
                at = skip_space(data, skip_value(data, at)?);
            }
        }
        b'<' => Some(at + data[at..].iter().position(|&b| b == b'>')? + 1),
        b'[' => {
            let mut at = skip_space(data, at + 1);
            loop {
                if *data.get(at)? == b']' {
                    return Some(at + 1);
                }
                at = skip_space(data, skip_value(data, at)?);
            }
        }
        b'(' => {
            let mut depth = 0usize;
            let mut i = at;
            loop {
                match *data.get(i)? {
                    b'\\' => i += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        b'/' => {
            let len = data[at + 1..]
                .iter()
                .take_while(|&&b| !is_space(b) && !is_delimiter(b))
                .count();
            Some(at + 1 + len)
        }
        b'0'..=b'9' | b'+' | b'-' | b'.' => {
            let end = at
                + data[at..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
                    .count();
            // 间接引用 `N G R` 作为一个值。
            if let Some((_, after)) = integer(data, skip_space(data, end)) {
                let r = skip_space(data, after);
                if data.get(r) == Some(&b'R')
                    && data
                        .get(r + 1)
                        .is_none_or(|&b| is_space(b) || is_delimiter(b))
                {
                    return Some(r + 1);
                }
            }
            Some(end)
        }
        _ => {
            let len = data[at..]
                .iter()
                .take_while(|&&b| !is_space(b) && !is_delimiter(b))
                .count();
            (len > 0).then_some(at + len)
        }
    }
}

/// 字典 `dict` 的条目：（键名，值的原文）。
fn dict_entries(dict: &[u8]) -> Option<Vec<(&[u8], &[u8])>> {
    if !dict.starts_with(b"<<") {
        return None;
    }
    let mut entries = Vec::new();
    let mut at = skip_space(dict, 2);
    loop {
        if dict.get(at..)?.starts_with(b">>") {
            return Some(entries);
        }
        if dict[at] != b'/' {
            return None;
        }
        let key_end = skip_value(dict, at)?;
        let value_start = skip_space(dict, key_end);
        let value_end = skip_value(dict, value_start)?;
        entries.push((&dict[at + 1..key_end], &dict[value_start..value_end]));
        at = skip_space(dict, value_end);
    }
}

/// 字典中键为 `key` 的名字值（不含开头的 `/`）。
fn dict_name<'a>(dict: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let entries = dict_entries(dict)?;
    let (_, value) = entries.into_iter().find(|(k, _)| *k == key)?;
    value.strip_prefix(b"/")
}

/// 字典中键为 `key` 的非负整数值；值是间接引用等其他形式时为 `None`。
fn dict_number(dict: &[u8], key: &[u8]) -> Option<usize> {
    let entries = dict_entries(dict)?;
    let (_, value) = entries.into_iter().find(|(k, _)| *k == key)?;
    let (number, end) = integer(value, 0)?;
    (end == value.len()).then_some(number)
}

#[cfg(test)]
pub(super) mod tests;
