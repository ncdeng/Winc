# Places the pinned PDFium build in a directory: pdfium.dll, and the archive it came from as
# pdfium-win-x64.tgz. winc does not ship PDFium; users download this same archive from inside winc
# (crates/winc-preview/src/pdf/pdfium/install.rs, which holds the same version and SHA-256). CI uses
# it to run the PDF tests that select text and write annotations.
param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
$version = 'chromium%2F8111'
$sha256 = '3452d11031050ec65e57f180215b57b9e6d465e3fbedafa8a77ad90885c7d23e'
$url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$version/pdfium-win-x64.tgz"
New-Item -ItemType Directory -Force $Destination | Out-Null
$directory = (Resolve-Path $Destination).Path
$archive = Join-Path $directory 'pdfium-win-x64.tgz'
$unpacked = Join-Path ([IO.Path]::GetTempPath()) "pdfium.$PID"
try {
    Invoke-WebRequest -Uri $url -OutFile $archive -MaximumRetryCount 3 -RetryIntervalSec 5
    if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $sha256) { throw "unexpected SHA-256 for $url" }
    New-Item -ItemType Directory -Force $unpacked | Out-Null
    # Windows' own bsdtar: a GNU tar earlier on PATH (Git's) reads "D:" in a path as a remote host.
    & (Join-Path $env:SystemRoot 'System32/tar.exe') -xzf $archive -C $unpacked bin/pdfium.dll
    if ($LASTEXITCODE -ne 0) { throw "$url has no bin/pdfium.dll" }
    Move-Item -Force (Join-Path $unpacked 'bin/pdfium.dll') (Join-Path $directory 'pdfium.dll')
} finally {
    Remove-Item -Recurse $unpacked -ErrorAction SilentlyContinue
}
