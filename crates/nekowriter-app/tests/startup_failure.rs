//! 启动失败的现场验证：真正的二进制在每一步失败时都必须给出同一套可诊断说明。
//!
//! 这里跑真正的 `nekowriter`（`CARGO_BIN_EXE_nekowriter`），因为要钉住的正是「哪一步
//! 失败」与「用户看到什么」之间的接线：纯函数测试只能证明那句文案好读，不能证明
//! 数据目录与日志这两步真的接在了同一条出口上。

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// 用 `--data-dir` 指向给定路径启动一次，返回进程输出。
fn start_with_data_dir(data_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nekowriter"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(["--port", "0", "--no-browser"])
        .output()
        .expect("无法启动 nekowriter 二进制")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// 数据目录无法创建（父路径是一个普通文件）：必须说明目录、原因与下一步，并且不能
/// 声称原因已经写进 `logs/`——日志那一层还没落地。
#[test]
fn unusable_data_directory_is_reported_with_cause_and_next_step() {
    let sandbox = tempfile::tempdir().unwrap();
    let blocker = sandbox.path().join("blocker");
    fs::write(&blocker, b"not a directory").unwrap();
    let data_dir = blocker.join("data");

    let output = start_with_data_dir(&data_dir);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("NekoWriter 无法启动。"), "{stderr}");
    assert!(
        stderr.contains(&format!("数据目录：{}", data_dir.display())),
        "{stderr}"
    );
    assert!(stderr.contains("原因：无法准备数据目录（"), "{stderr}");
    assert!(stderr.contains("--data-dir"), "{stderr}");
    assert!(!stderr.contains("只写入了启动日志"), "{stderr}");
}

/// 日志位置被普通文件占住（日志初始化失败）：同样要有一句说明，同样不能声称原因
/// 已经记录在 `logs/`。
#[test]
fn unwritable_log_location_is_reported_too() {
    let sandbox = tempfile::tempdir().unwrap();
    let data_dir = sandbox.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("logs"), b"not a directory").unwrap();
    let output = start_with_data_dir(&data_dir);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("NekoWriter 无法启动。"), "{stderr}");
    // 这一步已经越过数据目录本身，说明里给的是它的物理位置。
    let resolved = fs::canonicalize(&data_dir).unwrap();
    assert!(
        stderr.contains(&format!("数据目录：{}", resolved.display())),
        "{stderr}"
    );
    assert!(
        stderr.contains("原因：无法在该目录下创建日志（"),
        "{stderr}"
    );
    assert!(stderr.contains("logs/ 子目录"), "{stderr}");
    assert!(!stderr.contains("只写入了启动日志"), "{stderr}");
}
