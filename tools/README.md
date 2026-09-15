# 本地构建工具

LanceDB 0.38 的构建脚本需要 Protocol Buffers 编译器（`protoc`）。

## Windows

首次开发前执行一次引导脚本，它只从 Protocol Buffers 官方 GitHub 发布页下载
Windows x64 的预编译 `protoc`，并使用发布元数据提供的 SHA-256 校验文件：

```powershell
./tools/bootstrap-protoc.ps1
```

下载的二进制位于 `tools/protoc/`（不纳入版本控制），`.cargo/config.toml` 里的
Windows 专属配置会让 Cargo 自动使用它。

## macOS / Linux

交给系统包管理器，Cargo 会在 PATH 上找到 `protoc`：

```bash
brew install protobuf              # macOS
apt install protobuf-compiler      # Debian / Ubuntu（需要管理员权限）
```

`.cargo/config.toml` 不为这些平台指定 `PROTOC`，本机安装的版本可以直接参与构建。
