//! 启动链：命令行参数、数据目录与日志、进程锁、数据库、前端产物，以及启动失败的统一说明。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.3），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`。`main.rs` 用一条 `use startup::{Cli, StartupError, prepare};` 把名字接回原位，
//! 因此 `fn main`（二进制入口，必须留在 crate 根）的每一行都保持字面不变。

use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use clap::Parser;
use directories::ProjectDirs;
use fs4::{FileExt, TryLockError};
use nekowriter_store::{Store, StoreError};
use thiserror::Error;
use tracing::error;

use crate::lifecycle;

const DEFAULT_PORT: u16 = 47_821;

#[derive(Debug, Parser)]
#[command(name = "nekowriter", version, about = "NekoWriter 本地写作工具")]
pub(crate) struct Cli {
    /// 本地数据库、LanceDB 和日志所在目录。
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    /// 仅监听回环地址的 HTTP 端口。
    #[arg(long, default_value_t = DEFAULT_PORT)]
    pub(crate) port: u16,
    /// 已构建前端资源目录；默认使用可执行文件旁的 webui/dist。
    #[arg(long)]
    pub(crate) web_dir: Option<PathBuf>,
    /// 不自动打开默认浏览器；无界面环境与自动化探针使用。
    #[arg(long)]
    pub(crate) no_browser: bool,
}

#[derive(Debug, Error)]
pub(crate) enum StartupError {
    #[error("无法确定系统本地数据目录")]
    NoLocalDataDirectory,
    #[error("无法启动 NekoWriter：{0}")]
    Store(#[from] StoreError),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据目录已被另一个 NekoWriter 进程占用：{0}")]
    DataDirectoryLocked(PathBuf),
    #[error("无法在数据目录创建日志文件：{0}")]
    LogFile(#[from] tracing_appender::rolling::InitError),
    #[error("无法定位当前可执行文件所在目录")]
    NoExecutableDirectory,
}

struct ProcessLock {
    _file: File,
}

impl ProcessLock {
    fn acquire(data_dir: &Path) -> Result<Self, StartupError> {
        let lock_path = data_dir.join("nekowriter.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StartupError::DataDirectoryLocked(lock_path)),
            Err(TryLockError::Error(error)) => Err(StartupError::Io(error)),
        }
    }
}

/// 启动过程中的一步：说明里点名是哪一步失败，并给出这一步能走的下一步。
///
/// 启动期的几步各自会以不同方式失败（路径不存在、只读、被占用、来自更早版本），用户
/// 需要的不是 Rust 错误行，而是「哪一步、为什么、现在能做什么」。
#[derive(Clone, Copy)]
pub(crate) enum StartupStep {
    DataDirectory,
    Logging,
    ProcessLock,
    Database,
    WebDirectory,
}

impl StartupStep {
    fn label(self) -> &'static str {
        match self {
            Self::DataDirectory => "无法准备数据目录",
            Self::Logging => "无法在该目录下创建日志",
            Self::ProcessLock => "无法锁定数据目录",
            Self::Database => "无法打开该目录里的数据库",
            Self::WebDirectory => "无法定位前端构建产物目录",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::DataDirectory => {
                "请确认这个路径存在且可写，然后重试；也可以改用另一个数据目录启动，例如：nekowriter --data-dir <新目录>"
            }
            Self::Logging => "请确认这个目录可写（应用需要它下面的 logs/ 子目录），然后重试",
            Self::ProcessLock => {
                "同一时间只允许一个进程打开同一个数据目录；请关闭正在运行的那个 NekoWriter，或改用另一个数据目录启动"
            }
            Self::Database => {
                "请确认这个目录可写且属于 NekoWriter；由更早版本创建的目录请用创建它的旧版本打开，或改用另一个数据目录启动"
            }
            Self::WebDirectory => "请用 --web-dir <目录> 明确指定前端构建产物所在目录",
        }
    }
}

/// 启动失败的统一出口：任何一步失败都经这里给出同一种说明，并保留原始错误细节。
///
/// `logs_dir` 只在该目录下的日志确实可写后给出：日志初始化之前没有可写的日志文件，
/// 说明因此不会声称原因已经记录在 `logs/`。返回值原样透传，进程仍以非零状态退出；
/// stderr 上随后出现的 Rust 原始错误行保留完整细节，供事后核对。
fn startup_failure(
    data_dir: Option<&Path>,
    logs_dir: Option<&Path>,
    step: StartupStep,
    error: StartupError,
) -> StartupError {
    error!(data_dir = ?data_dir, step = step.label(), %error, "启动失败");
    eprintln!(
        "{}",
        startup_failure_message(data_dir, logs_dir, step, &error)
    );
    error
}

/// 一次启动失败的原因与它自己能走的下一步。
///
/// 「旧目录」两种情形自带详细说明与前进路径，因此不再叠加步骤提示。
fn startup_failure_reason(
    step: StartupStep,
    error: &StartupError,
) -> (String, Option<&'static str>) {
    match error {
        StartupError::Store(StoreError::UnsupportedSchema { found, expected }) => (
            format!(
                "这个目录由更早的 NekoWriter 创建，数据库 schema 版本为 {found}，本版本只支持 {expected}。\n\
                 本版本不会读取或迁移其中的作品数据；要继续其中已有的作品，请用创建它的旧版本打开。\n\
                 要用本版本开始创作，请指定另一个数据目录启动，例如：nekowriter --data-dir <新目录>"
            ),
            None,
        ),
        StartupError::Store(StoreError::UnknownSchema) => (
            "这个目录里的数据库不是 NekoWriter 的数据（缺少 schema 标识）。\n\
             请确认 --data-dir 指向正确的数据目录，或换一个空目录启动本版本，例如：nekowriter --data-dir <新目录>"
                .to_owned(),
            None,
        ),
        StartupError::NoLocalDataDirectory => (
            "无法确定系统本地数据目录。".to_owned(),
            Some("请用 --data-dir <目录> 明确指定数据目录，例如：nekowriter --data-dir <新目录>"),
        ),
        other => (format!("{}（{other}）", step.label()), Some(step.hint())),
    }
}

/// 启动失败时给用户的说明：点名数据目录与失败的步骤、可诊断的原因与可以走的下一步。
///
/// 只在日志可写时才声明「往这个目录写了什么」，而且声明的内容就是实际写入的内容
/// （启动日志与进程锁文件），不用「不会改动它」这种与行为不符的说法。
pub(crate) fn startup_failure_message(
    data_dir: Option<&Path>,
    logs_dir: Option<&Path>,
    step: StartupStep,
    error: &StartupError,
) -> String {
    let (reason, hint) = startup_failure_reason(step, error);
    let mut lines = vec!["NekoWriter 无法启动。".to_owned()];
    if let Some(data_dir) = data_dir {
        lines.push(format!("数据目录：{}", data_dir.display()));
    }
    lines.push(format!("原因：{reason}"));
    if let Some(hint) = hint {
        lines.push(hint.to_owned());
    }
    if let Some(logs_dir) = logs_dir {
        lines.push(format!(
            "除了该目录原有的文件，本次启动只写入了启动日志与进程锁文件，作品数据（SQLite 与向量）未被修改；日志：{}。",
            logs_dir.display()
        ));
    }
    lines.join("\n")
}

/// 启动期准备好的东西：数据库句柄、目录，以及必须随进程存活的日志写入器与进程锁。
pub(crate) struct Startup {
    pub(crate) store: Store,
    pub(crate) data_dir: PathBuf,
    pub(crate) web_dir: PathBuf,
    /// 非阻塞日志写入器的守卫：drop 它会丢掉尚未落盘的日志行。
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
    /// 进程锁：drop 它会释放 `nekowriter.lock`。
    _lock: ProcessLock,
}

/// 完成启动期的全部步骤：数据目录、日志、进程锁、数据库、前端产物。
///
/// 每一步失败都经 `startup_failure` 给出同一种可诊断说明，因此这里不出现裸 `?`：
/// 恰恰是这几步最容易因为「路径不存在、只读、被占用、来自更早版本」而失败，而那正是
/// 用户最需要看到「哪一步、为什么、现在能做什么」的时候。日志在拿到数据目录后立即
/// 落地，之后的所有诊断都会同时进入该目录的 `logs/`。
///
/// # Errors
///
/// 任意一步失败时返回原始错误；调用方只需要把它交给 `main` 的非零退出即可。
pub(crate) async fn prepare(cli: &Cli) -> Result<Startup, StartupError> {
    let requested_dir = match cli.data_dir.clone() {
        Some(path) => path,
        None => match default_data_dir() {
            Ok(path) => path,
            Err(error) => {
                return Err(startup_failure(
                    None,
                    None,
                    StartupStep::DataDirectory,
                    error,
                ));
            }
        },
    };
    let data_dir = match resolve_data_dir(&requested_dir) {
        Ok(data_dir) => data_dir,
        Err(error) => {
            return Err(startup_failure(
                Some(&requested_dir),
                None,
                StartupStep::DataDirectory,
                error,
            ));
        }
    };
    let log_guard = match lifecycle::init_logging(&data_dir) {
        Ok(guard) => guard,
        // 日志这一层自己失败时没有可写的日志文件，说明只落到 stderr。
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                None,
                StartupStep::Logging,
                error,
            ));
        }
    };
    let logs_dir = data_dir.join("logs");
    let lock = match ProcessLock::acquire(&data_dir) {
        Ok(lock) => lock,
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                Some(&logs_dir),
                StartupStep::ProcessLock,
                error,
            ));
        }
    };
    let store = match Store::open(&data_dir).await {
        Ok(store) => store,
        Err(error) => {
            return Err(startup_failure(
                Some(&data_dir),
                Some(&logs_dir),
                StartupStep::Database,
                error.into(),
            ));
        }
    };
    let web_dir = match cli.web_dir.clone() {
        Some(path) => path,
        None => match default_web_dir() {
            Ok(path) => path,
            Err(error) => {
                return Err(startup_failure(
                    Some(&data_dir),
                    Some(&logs_dir),
                    StartupStep::WebDirectory,
                    error,
                ));
            }
        },
    };
    Ok(Startup {
        store,
        data_dir,
        web_dir,
        _log_guard: log_guard,
        _lock: lock,
    })
}

fn default_data_dir() -> Result<PathBuf, StartupError> {
    ProjectDirs::from("com", "NekoWriter", "NekoWriter")
        .map(|directories| directories.data_local_dir().to_path_buf())
        .ok_or(StartupError::NoLocalDataDirectory)
}

/// 在取得进程锁、SQLite 和 `LanceDB` 连接之前固定数据目录的物理位置。
///
/// 这样相同的相对 `--data-dir` 不会因启动工作目录不同而指向不同的本地作品库。
fn resolve_data_dir(path: &Path) -> Result<PathBuf, StartupError> {
    std::fs::create_dir_all(path)?;
    Ok(path.canonicalize()?)
}

/// 前端构建产物默认放在可执行文件旁的 `webui/dist`：发行目录整体拷走后从任何
/// 工作目录启动都能打开界面，不依赖源码树。
pub(crate) fn default_web_dir() -> Result<PathBuf, StartupError> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or(StartupError::NoExecutableDirectory)?;
    Ok(directory.join("webui").join("dist"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动失败时，用户看到的必须是一句点明目录、失败步骤、原因与前进路径的说明，
    /// 而不是一个只能看懂的 Rust 错误。
    #[test]
    fn startup_failure_messages_name_the_directory_step_cause_and_way_forward() {
        let data_dir = Path::new("/tmp/nw-schema-probe");
        let logs_dir = data_dir.join("logs");

        let stale = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::Database,
            &StartupError::Store(StoreError::UnsupportedSchema {
                found: "25".to_owned(),
                expected: 26,
            }),
        );
        assert!(stale.contains("数据目录：/tmp/nw-schema-probe"));
        assert!(stale.contains("25") && stale.contains("26"));
        assert!(stale.contains("旧版本"));
        assert!(stale.contains("--data-dir"));
        assert!(stale.contains("/tmp/nw-schema-probe/logs"));
        // 只声明实际写下的东西：日志与进程锁，而不是「不会改动它」。
        assert!(stale.contains("不会读取或迁移其中的作品数据"));
        assert!(!stale.contains("不会改动它"));

        let unknown = startup_failure_message(
            Some(data_dir),
            None,
            StartupStep::Database,
            &StartupError::Store(StoreError::UnknownSchema),
        );
        assert!(unknown.contains("数据目录：/tmp/nw-schema-probe"));
        assert!(unknown.contains("schema"));
        assert!(unknown.contains("--data-dir"));
        // 日志没落地时不能声称原因已记录在 logs/。
        assert!(!unknown.contains("只写入了启动日志"));

        // 早期步骤（数据目录、日志）的 IO 失败：原因用中文点名是哪一步。
        let unusable = startup_failure_message(
            Some(data_dir),
            None,
            StartupStep::DataDirectory,
            &StartupError::Io(std::io::Error::from_raw_os_error(20)),
        );
        assert!(unusable.contains("原因：无法准备数据目录（"));
        assert!(unusable.contains("--data-dir"));

        let locked = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::ProcessLock,
            &StartupError::DataDirectoryLocked(data_dir.join("nekowriter.lock")),
        );
        assert!(locked.contains("原因：无法锁定数据目录（"));
        assert!(locked.contains("关闭正在运行的那个"));

        let no_directory = startup_failure_message(
            None,
            None,
            StartupStep::DataDirectory,
            &StartupError::NoLocalDataDirectory,
        );
        assert!(no_directory.contains("NekoWriter 无法启动。"));
        assert!(!no_directory.contains("数据目录："));
        assert!(no_directory.contains("请用 --data-dir <目录> 明确指定"));

        let web = startup_failure_message(
            Some(data_dir),
            Some(&logs_dir),
            StartupStep::WebDirectory,
            &StartupError::NoExecutableDirectory,
        );
        assert!(web.contains("无法定位前端构建产物目录"));
        assert!(web.contains("--web-dir"));
    }

    #[test]
    fn default_web_dir_lives_next_to_the_executable() {
        let web_dir = default_web_dir().unwrap();
        assert!(web_dir.ends_with(Path::new("webui").join("dist")));
        let executable = std::env::current_exe().unwrap();
        assert_eq!(
            web_dir.parent().unwrap().parent().unwrap(),
            executable.parent().unwrap()
        );
    }
}
