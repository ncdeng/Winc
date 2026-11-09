//! 安装 PDFium：从固定版本的安装包（`.tgz`）中取出 `bin/pdfium.dll`，核对后放进指定的目录。
//!
//! 安装包来自 [`ARCHIVE_URL`]（bblanchon/pdfium-binaries 发布的 Windows x64 版本），下载由
//! 调用方完成（本 crate 不联网）。安装包和取出的动态库都按 SHA-256 核对，任何一项不符就不
//! 安装：下载被篡改或版本被替换时不会加载来路不明的代码。动态库先写进同一目录下的临时文件，
//! 核对、写完后再改名，中途失败不会留下半个文件。

use std::io::Write;
use std::path::Path;

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

/// 核对安装包 `archive`，取出其中的 `pdfium.dll` 写进 `dir`（目录不存在时创建）。
pub(super) fn install(archive: &[u8], dir: &Path) -> Result<(), String> {
    if sha256(archive) != ARCHIVE_SHA256 {
        return Err(bad_archive());
    }
    let dll = extract(archive, "bin/pdfium.dll").ok_or_else(bad_archive)?;
    if sha256(&dll) != DLL_SHA256 {
        return Err(bad_archive());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let target = dir.join("pdfium.dll");
    let part = dir.join(format!("pdfium.dll.part{}", std::process::id()));
    let written = std::fs::File::create(&part)
        .and_then(|mut file| file.write_all(&dll).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&part, &target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e.to_string());
    }
    Ok(())
}

/// 从 gzip 压缩的 tar 包 `archive` 中取出名为 `name` 的普通文件（名字前可以有 `./`）。
fn extract(archive: &[u8], name: &str) -> Option<Vec<u8>> {
    let tar = gunzip(archive)?;
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            return None;
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
        let regular = matches!(header[156], b'0' | 0);
        if regular && path.trim_start_matches("./") == name {
            return tar.get(data..data.checked_add(size)?).map(<[u8]>::to_vec);
        }
        at = data + size.div_ceil(512) * 512;
    }
    None
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
