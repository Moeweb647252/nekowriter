//! 本地交付的构建入口：把前端构建产物与后端可执行文件组装成一个脱离源码树的
//! 发行目录。
//!
//! 用法（在仓库任意位置）：
//!
//! ```text
//! cargo xtask package [--target <三元组>] [--debug] [--protoc <路径>]
//! ```
//!
//! 默认按本机三元组打包；`--target` 指定其它平台时用同一套布局组装该平台的产物
//! （例如 `--target x86_64-pc-windows-msvc`）。所有平台的发行目录形状一致，因为
//! 后端按「可执行文件旁的 `webui/dist`」解析前端资源。
//!
//! `.cargo/config.toml` 把 `PROTOC` 钉在 Windows 工具目录上（`force = true`），
//! 其它平台（含从非 Windows 主机交叉打包）用 `--protoc` 指定本机 protoc。
//!
//! 产物是 `target/package/<三元组>/nekowriter/`：
//!
//! - `nekowriter`（Windows 上是 `nekowriter.exe`）
//! - `webui/dist` 前端资源
//! - `README.txt` 面向使用者的说明
//! - `release-manifest.txt` 交付前核对用：目标三元组、构建配置、可执行文件的容器
//!   格式与各文件字节数 / SHA-256
//!
//! 打包时会读可执行文件的头部，确认它真的是目标平台的容器格式（PE / Mach-O /
//! ELF）**与架构**都与三元组一致：拷错、改名或交叉编译错架构的二进制都不会被当成
//! 成功。目标平台的标准库没装时打包会在任何构建之前停下，不会白写一次前端产物。

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

const PACKAGE_DIR_NAME: &str = "nekowriter";
const MANIFEST_FILE_NAME: &str = "release-manifest.txt";
const README_FILE_NAME: &str = "README.txt";
const USAGE: &str =
    "用法：cargo xtask package [--target <三元组>] [--debug] [--release] [--protoc <路径>]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("打包失败：{error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    release: bool,
    target: Option<String>,
    protoc: Option<String>,
}

fn run() -> Result<(), String> {
    let options = parse_arguments()?;
    let root = workspace_root()?;
    let host = host_triple()?;
    let target = options.target.clone().unwrap_or_else(|| host.clone());
    preflight_target(&target, &host, &sysroot()?)?;
    let profile = if options.release { "release" } else { "debug" };

    let mut build_arguments = vec!["build", "-p", "nekowriter-app", "--target", &target];
    if options.release {
        build_arguments.push("--release");
    }
    let protoc_config;
    if let Some(path) = &options.protoc {
        protoc_config = format!("env.PROTOC.value=\"{path}\"");
        build_arguments.push("--config");
        build_arguments.push(&protoc_config);
    }
    // 先编译后端：它的失败最可能发生（工具链、目标支持），而前端构建会改写
    // `webui/dist`，不该被一次注定失败的编译白白重做。
    run_command("cargo", &build_arguments, &root)?;
    build_webui(&root)?;

    let binary_name = binary_name(&target);
    let compiled = root
        .join("target")
        .join(&target)
        .join(profile)
        .join(binary_name);
    if !compiled.is_file() {
        return Err(format!("没有找到编译产物 {}", compiled.display()));
    }
    let format = describe_binary(&compiled)?;
    format.ensure_matches(&target)?;

    let package = root
        .join("target")
        .join("package")
        .join(&target)
        .join(PACKAGE_DIR_NAME);
    if package.exists() {
        fs::remove_dir_all(&package).map_err(|error| format!("无法清理旧发行目录：{error}"))?;
    }
    let web_dist = package.join("webui").join("dist");
    let web_files: Vec<FileEntry> = copy_directory(&root.join("webui").join("dist"), &web_dist)?
        .into_iter()
        .map(|entry| FileEntry {
            name: format!("webui/dist/{}", entry.name),
            ..entry
        })
        .collect();
    fs::copy(&compiled, package.join(binary_name))
        .map_err(|error| format!("无法拷贝可执行文件：{error}"))?;
    fs::write(package.join(README_FILE_NAME), readme(&target))
        .map_err(|error| format!("无法写入说明文件：{error}"))?;

    let binary = file_entry(&package.join(binary_name), binary_name)?;
    let binary_bytes = binary.bytes;
    let mut listed = vec![
        binary,
        file_entry(&package.join(README_FILE_NAME), README_FILE_NAME)?,
    ];
    listed.extend(web_files);
    let manifest = manifest(&target, profile, &format.description, &listed);
    fs::write(package.join(MANIFEST_FILE_NAME), &manifest)
        .map_err(|error| format!("无法写入发行清单：{error}"))?;

    println!("发行目录：{}", package.display());
    println!(
        "可执行文件：{binary_name}（{}，{binary_bytes} 字节）",
        format.description
    );
    println!();
    print!("{manifest}");
    Ok(())
}

fn parse_arguments() -> Result<Options, String> {
    let mut options = Options {
        release: true,
        target: None,
        protoc: None,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "package" => {}
            "--debug" => options.release = false,
            "--release" => options.release = true,
            "--target" => {
                let target = arguments
                    .next()
                    .ok_or_else(|| format!("--target 需要三元组\n{USAGE}"))?;
                options.target = Some(target);
            }
            "--protoc" => {
                let path = arguments
                    .next()
                    .ok_or_else(|| format!("--protoc 需要路径参数\n{USAGE}"))?;
                if !Path::new(&path).is_file() {
                    return Err(format!("protoc 不存在：{path}"));
                }
                options.protoc = Some(path);
            }
            other => return Err(format!("未知参数：{other}\n{USAGE}")),
        }
    }
    Ok(options)
}

/// 本机三元组：直接问编译器，避免自己拼 `arch-os` 猜错。
fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|error| format!("无法运行 rustc：{error}"))?;
    if !output.status.success() {
        return Err("rustc -vV 失败".to_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(ToOwned::to_owned))
        .ok_or_else(|| "无法从 rustc -vV 读出本机三元组".to_owned())
}

/// 构建前端资源：发行目录里的 `webui/dist` 必须是本次构建的结果，而不是仓库里
/// 碰巧留下的旧产物。
fn build_webui(root: &Path) -> Result<(), String> {
    run_command("pnpm", &["--dir", "webui", "build"], root)
}

fn binary_name(target: &str) -> &'static str {
    if target.contains("windows") {
        "nekowriter.exe"
    } else {
        "nekowriter"
    }
}

/// 交叉打包前先确认目标平台的标准库真的装了。
///
/// 缺标准库时 `cargo build` 会在编译中途才失败，那时前端已经白构建一次。这里先看
/// 编译器自己的 sysroot，把注定失败的打包提前到任何构建之前。本机目标无需检查。
fn preflight_target(target: &str, host: &str, sysroot: &Path) -> Result<(), String> {
    if target == host {
        return Ok(());
    }
    let standard_library = sysroot.join("lib").join("rustlib").join(target).join("lib");
    if standard_library.is_dir() {
        return Ok(());
    }
    Err(format!(
        "目标 {target} 的标准库未安装：请先运行 `rustup target add {target}`，或改用本机三元组"
    ))
}

/// 编译器自己的 sysroot；目标平台的标准库就在它下面。
fn sysroot() -> Result<PathBuf, String> {
    let output = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()))
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| format!("无法运行 rustc：{error}"))?;
    if !output.status.success() {
        return Err("rustc --print sysroot 失败".to_owned());
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// 目标三元组要求的容器格式。
fn expected_container(target: &str) -> &'static str {
    if target.contains("windows") {
        "PE32+"
    } else if target.contains("apple") {
        "Mach-O"
    } else {
        "ELF"
    }
}

/// 目标三元组的架构分量，写成与可执行文件头部一致的规范名。
fn expected_arch(target: &str) -> Result<&'static str, String> {
    match target.split('-').next().unwrap_or_default() {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("aarch64"),
        other => Err(format!(
            "不支持的目标架构 {other}：目前只校验 x86_64 与 aarch64\n{USAGE}"
        )),
    }
}

/// 可执行文件的平台描述：容器格式与架构。
struct BinaryFormat {
    container: &'static str,
    arch: &'static str,
    /// 给人看的一行，例如 `Mach-O 64 位（aarch64）`。
    description: String,
}

impl BinaryFormat {
    /// 容器格式与架构都必须与目标三元组一致。
    ///
    /// 只比容器家族是不够的：`aarch64-unknown-linux-gnu` 配一个 x86-64 的 ELF 也是
    /// ELF，但装到目标机器上跑不起来。
    fn ensure_matches(&self, target: &str) -> Result<(), String> {
        let arch = expected_arch(target)?;
        if self.container != expected_container(target) || self.arch != arch {
            return Err(format!(
                "编译产物是 {}，与目标平台 {target} 不符；请确认 --target 与工具链",
                self.description
            ));
        }
        Ok(())
    }
}

/// 读取可执行文件的头部，识别它的容器格式与架构。
///
/// 这一步是「别拿别的平台的二进制充数」的硬检查：改名、拷错与交叉编译错架构都会在
/// 与目标三元组比对时失败。
fn describe_binary(path: &Path) -> Result<BinaryFormat, String> {
    let mut file = File::open(path).map_err(|error| format!("无法读取编译产物：{error}"))?;
    let mut header = vec![0_u8; 64];
    let read = file
        .read(&mut header)
        .map_err(|error| format!("无法读取编译产物：{error}"))?;
    header.truncate(read);
    let get =
        |offset: usize, length: usize| -> Option<&[u8]> { header.get(offset..offset + length) };
    let u16_at = |offset: usize| -> Option<u16> {
        get(offset, 2).map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let u32_at = |offset: usize| -> Option<u32> {
        get(offset, 4).map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };

    let (container, arch) = if get(0, 2) == Some(b"MZ") {
        // PE：e_lfanew 指向 PE 签名，签名的下两个字节是机器类型。
        let offset = usize::try_from(u32_at(0x3C).ok_or("PE 头部不完整")?)
            .map_err(|_| "PE 头部偏移超出范围".to_owned())?;
        file.seek(SeekFrom::Start(u64::try_from(offset).unwrap_or(u64::MAX)))
            .map_err(|error| format!("无法读取编译产物：{error}"))?;
        let mut signature = [0_u8; 6];
        file.read_exact(&mut signature)
            .map_err(|error| format!("无法读取 PE 签名：{error}"))?;
        if &signature[..4] != b"PE\0\0" {
            return Err("文件以 MZ 开头但不是 PE 镜像".to_owned());
        }
        let machine = u16::from_le_bytes([signature[4], signature[5]]);
        let arch = match machine {
            0x8664 => "x86_64",
            0xAA64 => "aarch64",
            other => return Err(format!("未知的 PE 机器类型 0x{other:x}")),
        };
        ("PE32+", arch)
    } else if get(0, 4) == Some(&[0xCF, 0xFA, 0xED, 0xFE]) {
        let cpu = u32_at(4).ok_or("Mach-O 头部不完整")?;
        let arch = match cpu {
            0x0100_0007 => "x86_64",
            0x0100_000C => "aarch64",
            other => return Err(format!("未知的 Mach-O CPU 类型 0x{other:x}")),
        };
        ("Mach-O", arch)
    } else if get(0, 4) == Some(b"\x7fELF") {
        let machine = u16_at(18).ok_or("ELF 头部不完整")?;
        let arch = match machine {
            0x3E => "x86_64",
            0xB7 => "aarch64",
            other => return Err(format!("未知的 ELF 机器类型 0x{other:x}")),
        };
        ("ELF", arch)
    } else {
        return Err("无法识别的可执行文件格式（既不是 PE、Mach-O 也不是 ELF）".to_owned());
    };

    let description = match container {
        // PE32+ 这个名字本身就说明是 64 位，不再重复。
        "PE32+" => format!("PE32+（{arch}）"),
        other => format!("{other} 64 位（{arch}）"),
    };
    Ok(BinaryFormat {
        container,
        arch,
        description,
    })
}

struct FileEntry {
    /// 相对发行目录的路径，目录分隔符统一为 `/`。
    name: String,
    bytes: u64,
    sha256: String,
}

fn file_entry(path: &Path, name: &str) -> Result<FileEntry, String> {
    let bytes =
        fs::metadata(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    Ok(FileEntry {
        name: name.to_owned(),
        bytes: bytes.len(),
        sha256: sha256(path)?,
    })
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 发行清单：人读的头部 + 机器可核对的「路径 字节数 SHA-256」逐行表。
///
/// 统一用 CRLF 换行，方便在 Windows 记事本里直接打开。
fn manifest(target: &str, profile: &str, format: &str, files: &[FileEntry]) -> String {
    let generated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let total: u64 = files.iter().map(|file| file.bytes).sum();
    let mut lines = vec![
        "NekoWriter 发行清单".to_owned(),
        String::new(),
        format!("目标三元组：{target}"),
        format!("构建配置：{profile}"),
        format!("可执行文件容器格式：{format}"),
        format!("生成时间（UTC 毫秒）：{generated_at_ms}"),
        format!("文件：{} 个，共 {total} 字节", files.len()),
        String::new(),
    ];
    lines.extend(
        files
            .iter()
            .map(|file| format!("{}\t{}\t{}", file.name, file.bytes, file.sha256)),
    );
    format!("{}\r\n", lines.join("\r\n"))
}

fn readme(target: &str) -> String {
    format!(
        "NekoWriter 本地写作工具\r\n\
         本次发行：{target}\r\n\r\n\
         用法：\r\n\
         \x20 nekowriter                  启动本地服务并打开默认浏览器（默认端口 47821）\r\n\
         \x20 nekowriter --no-browser     不自动打开浏览器\r\n\
         \x20 nekowriter --port <端口>    改用其它回环端口\r\n\
         \x20 nekowriter --data-dir <目录> 指定作品库目录\r\n\r\n\
         数据位置：默认使用系统本地应用数据目录下的 NekoWriter，包含 nekowriter.sqlite3、\r\n\
         lancedb/、logs/ 与进程锁文件。工作台右下角「本地服务」面板会显示当前实际目录。\r\n\r\n\
         模型密钥存放在操作系统提供的凭据存储里（macOS 钥匙串 / Windows 凭据管理器 /\r\n\
         Linux Secret Service），数据库与请求快照都不保存密钥正文。\r\n\r\n\
         退出：在工作台「本地服务」面板点击退出，或在运行窗口按 Ctrl+C。退出前会停止接受\r\n\
         新请求、取消在途模型调用、把执行中的 Run 收束为「已中断」，然后落盘并关闭数据库。\r\n"
    )
}

/// 仓库根目录：本 crate 位于 `crates/xtask`，因此向上两级。
fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "无法确定仓库根目录".to_owned())
}

fn run_command(program: &str, arguments: &[&str], cwd: &Path) -> Result<(), String> {
    println!("> {program} {}", arguments.join(" "));
    let status = Command::new(program)
        .args(arguments)
        .current_dir(cwd)
        .status()
        .map_err(|error| format!("无法运行 {program}：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} 以 {status} 结束"))
    }
}

/// 拷贝目录，返回其中每个文件相对于 `from` 的路径、字节数与 SHA-256。
///
/// 清单记录的是**拷贝到发行目录之后的那一份**（大小与摘要都从目标文件算），
/// 所以清单描述的就是交付物本身。
fn copy_directory(from: &Path, to: &Path) -> Result<Vec<FileEntry>, String> {
    if !from.is_dir() {
        return Err(format!("没有找到前端构建产物 {}", from.display()));
    }
    fs::create_dir_all(to).map_err(|error| format!("无法创建目录：{error}"))?;
    let mut files = Vec::new();
    for entry in fs::read_dir(from).map_err(|error| format!("无法读取目录：{error}"))? {
        let entry = entry.map_err(|error| format!("无法读取目录项：{error}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "前端资源名不是有效 UTF-8".to_owned())?;
        if entry.path().is_dir() {
            for nested in copy_directory(&entry.path(), &to.join(&name))? {
                files.push(FileEntry {
                    name: format!("{name}/{}", nested.name),
                    ..nested
                });
            }
        } else {
            fs::copy(entry.path(), to.join(&name))
                .map_err(|error| format!("无法拷贝文件：{error}"))?;
            files.push(file_entry(&to.join(&name), &name)?);
        }
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个临时文件，用后即删；名字带进程号与纳秒时间戳，避免并发测试撞名。
    fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
        let path = scratch_path(name, "bin");
        fs::write(&path, bytes).unwrap();
        path
    }

    /// 造一个空的临时目录，用后即删。
    fn scratch_dir(name: &str) -> PathBuf {
        let path = scratch_path(name, "dir");
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// 带进程号与纳秒时间戳的临时路径，避免并发测试撞名。
    fn scratch_path(name: &str, suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!(
            "xtask-{name}-{}-{unique}.{suffix}",
            std::process::id()
        ))
    }

    fn macho_arm64() -> Vec<u8> {
        let mut bytes = vec![0xCF, 0xFA, 0xED, 0xFE];
        bytes.extend_from_slice(&0x0100_000Cu32.to_le_bytes());
        bytes.resize(64, 0);
        bytes
    }

    fn pe_x86_64() -> Vec<u8> {
        let mut bytes = vec![0_u8; 64];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3C..0x40].copy_from_slice(&64_u32.to_le_bytes());
        bytes.extend_from_slice(b"PE\0\0");
        bytes.extend_from_slice(&0x8664_u16.to_le_bytes());
        bytes
    }

    #[test]
    fn container_and_arch_must_both_match_the_requested_target() {
        let macho = scratch("macho", &macho_arm64());
        let described = describe_binary(&macho).unwrap();
        assert_eq!(described.description, "Mach-O 64 位（aarch64）");
        assert!(described.ensure_matches("aarch64-apple-darwin").is_ok());
        // 把一个 macOS 产物当 Windows 目标打包必须失败，而不是安静地打出来。
        let wrong_os = described
            .ensure_matches("x86_64-pc-windows-msvc")
            .unwrap_err();
        assert!(wrong_os.contains("与目标平台 x86_64-pc-windows-msvc 不符"));
        // 容器对、架构不对同样必须失败。
        let wrong_arch = described.ensure_matches("x86_64-apple-darwin").unwrap_err();
        assert!(wrong_arch.contains("与目标平台 x86_64-apple-darwin 不符"));
        fs::remove_file(&macho).unwrap();
    }

    #[test]
    fn pe_and_elf_headers_are_described_by_container_and_arch() {
        let pe = scratch("pe", &pe_x86_64());
        let described = describe_binary(&pe).unwrap();
        assert_eq!(described.description, "PE32+（x86_64）");
        assert!(described.ensure_matches("x86_64-pc-windows-msvc").is_ok());
        assert!(described.ensure_matches("aarch64-pc-windows-msvc").is_err());
        fs::remove_file(&pe).unwrap();

        let mut elf = vec![0x7F, b'E', b'L', b'F', 2];
        elf.resize(20, 0);
        elf[18..20].copy_from_slice(&0x3E_u16.to_le_bytes());
        let elf = scratch("elf", &elf);
        let described = describe_binary(&elf).unwrap();
        assert_eq!(described.description, "ELF 64 位（x86_64）");
        assert!(described.ensure_matches("x86_64-unknown-linux-gnu").is_ok());
        // 一个 x86-64 的 ELF 不能当 aarch64 目标的产物。
        let wrong_arch = described
            .ensure_matches("aarch64-unknown-linux-gnu")
            .unwrap_err();
        assert!(wrong_arch.contains("与目标平台 aarch64-unknown-linux-gnu 不符"));
        fs::remove_file(&elf).unwrap();

        let junk = scratch("junk", b"not an executable");
        assert!(describe_binary(&junk).is_err());
        fs::remove_file(&junk).unwrap();
    }

    #[test]
    fn unsupported_target_architectures_are_named_in_the_error() {
        assert_eq!(expected_arch("aarch64-apple-darwin").unwrap(), "aarch64");
        assert_eq!(expected_arch("x86_64-unknown-linux-gnu").unwrap(), "x86_64");
        assert!(expected_arch("wasm32-unknown-unknown").is_err());
        assert_eq!(expected_container("x86_64-pc-windows-msvc"), "PE32+");
        assert_eq!(expected_container("aarch64-unknown-linux-gnu"), "ELF");
    }

    #[test]
    fn targets_without_an_installed_standard_library_fail_before_building() {
        let root = scratch_dir("sysroot");
        fs::create_dir_all(root.join("lib/rustlib/aarch64-apple-darwin/lib")).unwrap();
        assert!(
            preflight_target("aarch64-apple-darwin", "x86_64-unknown-linux-gnu", &root).is_ok()
        );
        let error =
            preflight_target("x86_64-pc-windows-gnu", "aarch64-apple-darwin", &root).unwrap_err();
        assert!(error.contains("rustup target add x86_64-pc-windows-gnu"));
        // 本机目标不看 sysroot 布局。
        assert!(preflight_target("aarch64-apple-darwin", "aarch64-apple-darwin", &root).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }
}
