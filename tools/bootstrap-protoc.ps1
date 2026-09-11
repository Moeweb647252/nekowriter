[CmdletBinding()]
param(
    [string]$Version = '35.1'
)

$ErrorActionPreference = 'Stop'
$toolRoot = Join-Path $PSScriptRoot 'protoc'
if (Test-Path -LiteralPath $toolRoot) {
    throw "工具目录已存在：$toolRoot。请先人工确认其内容；脚本不会覆盖已有编译器。"
}

$release = Invoke-RestMethod -Uri "https://api.github.com/repos/protocolbuffers/protobuf/releases/tags/v$Version" -Headers @{ 'User-Agent' = 'NekoWriter-bootstrap' }
$assetName = "protoc-$Version-win64.zip"
$asset = @($release.assets | Where-Object { $_.name -eq $assetName }) | Select-Object -First 1
if ($null -eq $asset -or [string]::IsNullOrWhiteSpace($asset.digest) -or $asset.digest -notmatch '^sha256:') {
    throw "官方发布元数据未提供 $assetName 的 SHA-256。"
}
if ($asset.browser_download_url -notlike 'https://github.com/protocolbuffers/protobuf/releases/download/*') {
    throw '下载地址不是 Protocol Buffers 官方 GitHub 发布地址。'
}

$archive = New-TemporaryFile
try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive.FullName
    $expected = $asset.digest.Substring('sha256:'.Length).ToUpperInvariant()
    $actual = (Get-FileHash -LiteralPath $archive.FullName -Algorithm SHA256).Hash.ToUpperInvariant()
    if ($actual -ne $expected) {
        throw '下载文件的 SHA-256 与官方发布元数据不一致。'
    }

    New-Item -ItemType Directory -Path $toolRoot | Out-Null
    Expand-Archive -LiteralPath $archive.FullName -DestinationPath $toolRoot -ErrorAction Stop
    & (Join-Path $toolRoot 'bin/protoc.exe') --version
} finally {
    Remove-Item -LiteralPath $archive.FullName -Force -ErrorAction SilentlyContinue
}

