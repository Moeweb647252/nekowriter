# 本地构建工具

LanceDB 0.38 的 Windows 构建需要 Protocol Buffers 编译器。首次开发前执行：

```powershell
./tools/bootstrap-protoc.ps1
```

脚本只从 Protocol Buffers 官方 GitHub 发布页下载 Windows x64 的预编译 `protoc`，并使用发布元数据提供的 SHA-256 校验文件。下载的二进制位于 `tools/protoc/`，该目录不纳入版本控制；Cargo 通过 `.cargo/config.toml` 自动使用它。

