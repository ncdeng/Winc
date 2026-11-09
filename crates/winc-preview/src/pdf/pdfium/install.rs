//! 安装 PDFium：从固定版本的安装包（`.tgz`）中取出 `bin/pdfium.dll` 和许可证，核对后放进指定的
//! 目录。
//!
//! 安装包来自 [`ARCHIVE_URL`]（bblanchon/pdfium-binaries 发布的 Windows x64 版本），下载由
//! 调用方完成（本 crate 不联网）。安装包和取出的动态库都按 SHA-256 核对，任何一项不符就不
//! 安装：下载被篡改或版本被替换时不会加载来路不明的代码。
//!
//! 许可证随动态库一起安装：`LICENSE`（pdfium-binaries 的构建脚本）和 `licenses/` 下 PDFium 及其
//! 依赖库的许可证。动态库最后写入：先写进同一目录下的临时文件，写完后再改名，中途失败不会留下
//! 半个文件，`pdfium.dll` 存在就说明安装完整。

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use winc_i18n::t;

/// PDFium 安装包的下载地址：固定为 `chromium/8111` 版本，与 [`ARCHIVE_SHA256`] 对应。
pub(super) const ARCHIVE_URL: &str = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8111/pdfium-win-x64.tgz";

/// 安装包的 SHA-256。
const ARCHIVE_SHA256: &str = "3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e";

/// 安装包中 `bin/pdfium.dll` 的 SHA-256。
const DLL_SHA256: &str = "a318c39c137482dc4687c2973b7ec1c27fc1c20253cf2dc31928f922eab89e8f";

/// 安装包解压后的大小上限（字节）：实际约 20 MB，超过上限说明不是预期的安装包。
const MAX_UNPACKED: usize = 64 << 20;

/// 安装失败时给用户看的原因。
fn bad_archive() -> String {
    t!("preview-pdf-engine-bad-archive").to_owned()
}

/// `data` 的 SHA-256（小写十六进制）。
fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 核对安装包 `archive`，取出其中的 `pdfium.dll` 和许可证写进 `dir`（目录不存在时创建）。
pub(super) fn install(archive: &[u8], dir: &Path) -> Result<(), String> {
    if sha256(archive) != ARCHIVE_SHA256 {
        return Err(bad_archive());
    }
    let tar = gunzip(archive).ok_or_else(bad_archive)?;
    let files = files(&tar).ok_or_else(bad_archive)?;
    let dll = files
        .iter()
        .find(|(path, _)| path == "bin/pdfium.dll")
        .map(|&(_, data)| data)
        .ok_or_else(bad_archive)?;
    if sha256(dll) != DLL_SHA256 {
        return Err(bad_archive());
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir.join("licenses")).map_err(io)?;
    for (path, data) in &files {
        if let Some(name) = license_name(path) {
            std::fs::write(dir.join(name), data).map_err(io)?;
        }
    }
    let target = dir.join("pdfium.dll");
    let part = dir.join(format!("pdfium.dll.part{}", std::process::id()));
    let written = std::fs::File::create(&part)
        .and_then(|mut file| file.write_all(dll).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&part, &target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(io(e));
    }
    Ok(())
}

/// 安装包中的路径 `path` 是许可证时，返回它在安装目录中的相对路径：`LICENSE`，或
/// `licenses` 目录下的文件名。其他文件，以及名字里带目录分隔符或以 `.` 开头的文件，返回
/// `None`。
fn license_name(path: &str) -> Option<PathBuf> {
    if path == "LICENSE" {
        return Some(PathBuf::from("LICENSE"));
    }
    let name = path.strip_prefix("licenses/")?;
    let plain = !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':']);
    plain.then(|| Path::new("licenses").join(name))
}

/// 解压后的 tar 数据 `tar` 中的普通文件：（路径，内容），路径去掉开头的 `./`。格式不对时返回
/// `None`。
fn files(tar: &[u8]) -> Option<Vec<(String, &[u8])>> {
    let mut files = Vec::new();
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            std::str::from_utf8(&raw[..end]).ok()
        };
        let size = usize::from_str_radix(field(124..136)?.trim(), 8).ok()?;
        let mut path = field(0..100)?.to_owned();
        // ustar 格式：长路径的前半段放在 prefix 字段里。
        if &header[257..262] == b"ustar" {
            let prefix = field(345..500)?;
            if !prefix.is_empty() {
                path = format!("{prefix}/{path}");
            }
        }
        let data = at + 512;
        if matches!(header[156], b'0' | 0) {
            let content = tar.get(data..data.checked_add(size)?)?;
            files.push((path.trim_start_matches("./").to_owned(), content));
        }
        at = data + size.div_ceil(512) * 512;
    }
    Some(files)
}

/// 解开 gzip 数据（RFC 1952）：跳过文件头，解压其中的 deflate 数据。
fn gunzip(data: &[u8]) -> Option<Vec<u8>> {
    const FHCRC: u8 = 1 << 1;
    const FEXTRA: u8 = 1 << 2;
    const FNAME: u8 = 1 << 3;
    const FCOMMENT: u8 = 1 << 4;
    if data.len() < 18 || data[0..3] != [0x1f, 0x8b, 8] {
        return None;
    }
    let flags = data[3];
    let mut at = 10;
    if flags & FEXTRA != 0 {
        let len = usize::from(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        at += 2 + len;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            at += data.get(at..)?.iter().position(|&b| b == 0)? + 1;
        }
    }
    if flags & FHCRC != 0 {
        at += 2;
    }
    miniz_oxide::inflate::decompress_to_vec_with_limit(data.get(at..)?, MAX_UNPACKED).ok()
}

#[cfg(test)]
mod tests;
