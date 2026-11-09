//! `install` 的测试：手工拼出 tgz，取出其中的文件；核对不符的安装包不安装。

use super::*;

/// 拼一个 gzip 压缩的 tar 包，内含 `files`（路径，内容）。
fn tgz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = Vec::new();
    for (name, data) in files {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        let size = format!("{:011o}", data.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        tar.extend_from_slice(&header);
        tar.extend_from_slice(data);
        tar.resize(tar.len().div_ceil(512) * 512, 0);
    }
    tar.extend_from_slice(&[0u8; 1024]);
    // 文件头带 FNAME 标志和文件名，检查跳过文件头的逻辑；尾部的校验和不读，写 0。
    let mut out = vec![0x1f, 0x8b, 8, 1 << 3, 0, 0, 0, 0, 0, 3];
    out.extend_from_slice(b"pdfium.tar\0");
    out.extend(miniz_oxide::deflate::compress_to_vec(&tar, 6));
    out.extend_from_slice(&[0; 8]);
    out
}

#[test]
fn files_come_out_of_a_tgz() {
    let archive = tgz(&[
        ("LICENSE", b"license text".as_slice()),
        ("./bin/pdfium.dll", b"MZ fake dll".as_slice()),
        ("lib/pdfium.dll.lib", b"lib".as_slice()),
    ]);
    assert_eq!(
        extract(&archive, "bin/pdfium.dll").as_deref(),
        Some(b"MZ fake dll".as_slice())
    );
    assert_eq!(
        extract(&archive, "LICENSE").as_deref(),
        Some(b"license text".as_slice())
    );
    assert_eq!(extract(&archive, "bin/missing.dll"), None);
    assert_eq!(extract(b"not gzip data at all", "LICENSE"), None);
}

#[test]
fn archives_that_do_not_match_are_not_installed() {
    let dir = std::env::temp_dir().join(format!("winc-pdfium-install-{}", std::process::id()));
    let archive = tgz(&[("bin/pdfium.dll", b"MZ fake dll".as_slice())]);
    assert_eq!(install(&archive, &dir), Err(bad_archive()));
    assert!(!dir.join("pdfium.dll").exists());
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// 用真实的安装包安装（`WINC_PDFIUM_ARCHIVE` 指定 `pdfium-win-x64.tgz` 的路径；没有设置时
/// 跳过）。
#[test]
fn the_pinned_archive_installs() {
    let Some(path) = std::env::var_os("WINC_PDFIUM_ARCHIVE") else {
        return;
    };
    let archive = std::fs::read(path).unwrap();
    let dir = std::env::temp_dir().join(format!("winc-pdfium-real-{}", std::process::id()));
    install(&archive, &dir).unwrap();
    let dll = std::fs::read(dir.join("pdfium.dll")).unwrap();
    assert_eq!(sha256(&dll), DLL_SHA256);
    let _ = std::fs::remove_dir_all(&dir);
}
