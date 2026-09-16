//! 本地交付的构建入口：把前端构建产物与后端可执行文件组装成一个脱离源码树的
//! 发行目录。
//!
//! 用法（在仓库任意位置）：
//!
//! ```text
//! cargo xtask package [--target <三元组>] [--debug] [--protoc <路径>]
//! cargo xtask check-move-only --base <基准提交> --from <文件>... --into <文件>...
//! cargo xtask check-move-only --base-state <上一批快照> --head-state <本批快照> ...
//! ```
//!
//! `check-move-only` 是 `docs/08` 那轮「只移动、不改行为」拆分的验收工具：它把
//! 「名称解析不变」与「代码行多重集不变」两条不变量算成可重复执行的机械结论。
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
    collections::{BTreeMap, BTreeSet},
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
const CHECK_USAGE: &str = "用法：cargo xtask check-move-only --base <基准提交> [--head <提交>] \
--from <搬迁前的文件>... --into <搬迁后的文件>... \
[--allow <额外允许改动的路径>...] [--scope <作用域根目录>...] \
[--base-state <上一批的状态目录>] [--head-state <本批的状态目录>] [--write-state <状态目录>] \
[--list-items]";
/// 批次状态快照覆盖的范围：`crates/` 下的全部 Rust 源码。
const STATE_ROOT: &str = "crates";

fn main() -> ExitCode {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|first| first == "check-move-only")
    {
        arguments.remove(0);
        return match run_check_move_only(&arguments) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("搬迁校验失败：{error}");
                ExitCode::FAILURE
            }
        };
    }
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

/// 一次「只移动、不改行为」的拆分的声明。
///
/// 两条不变量各自对应一道机械检查：
///
/// - **I1 名称解析不变**：声明之外的文件一行都不许改。被搬走的项在原位补一行
///   `pub(crate) use`，于是调用方（`crate::ApiError`、`super::insert_app_event` 这些名字的
///   每个使用者）保持字面不变。检查方式是「本批改动、新增、删除的文件必须全部在声明集合里」。
/// - **I2 正文不变**：`--from` 声明的搬迁前文件与 `--into` 声明的搬迁后文件，归一化之后必须
///   给出同一个代码行多重集。归一化只丢掉搬迁必然要改的东西：缩进、空行、`use` / `mod` /
///   `impl` 行、`pub` / `pub(crate)` / `pub(super)` 前缀，以及函数签名的折行（同一个签名
///   因为多了可见性前缀被 rustfmt 折成多行时，两侧合并成一行后相等）。
///
/// 两道都通过才打印 `MOVE-ONLY OK`；任何一处不过都返回错误并列出残留行。
struct MoveCheck {
    base: String,
    /// `None` 表示与工作区比较（不提交也能验收，见 `docs/08` 的提交粒度一节）。
    head: Option<String>,
    /// 上一批通过校验时的文件状态。给了它就对着这份状态算「哪些文件变了」、并从它读基准
    /// 内容——一批一提交时用 `--base <提交>` 就够，但未提交的连续批次（本仓的实际做法）
    /// 无法用提交点隔离单批：`--base HEAD` 会把上一批的搬迁也算进本批。
    base_state: Option<String>,
    /// 「搬迁后」那一侧的另一份批次快照。给了它就不再读工作区，于是一批通过校验之后可以随时
    /// 重跑（回验任意一批，不只是最近一批）；与 `--head <提交>` 二选一。
    head_state: Option<String>,
    /// 本批通过校验后把当前状态写到这个目录，作为下一批的 `--base-state`。
    write_state: Option<String>,
    /// 只把两侧切出来的定义项列出来，不做判定：判据按**项**分组，而项的身份就是签名与
    /// `impl` 头，要人工核对就得能看见这些名字。
    list_items: bool,
    from: Vec<String>,
    into: Vec<String>,
    allow: Vec<String>,
    scope: Vec<String>,
}

fn run_check_move_only(arguments: &[String]) -> Result<(), String> {
    let check = parse_move_check(arguments)?;
    let root = workspace_root()?;
    let head = match (&check.head, &check.head_state) {
        (_, Some(state)) => format!("状态 {state}"),
        (Some(head), None) => head.clone(),
        (None, None) => "工作区".to_owned(),
    };
    match &check.base_state {
        Some(state) => println!("搬迁校验：基准状态 = {state}，head = {head}"),
        None => println!("搬迁校验：base = {}，head = {head}", check.base),
    }
    println!(
        "声明：--from {} 个文件，--into {} 个文件，允许改动 {} 个路径，作用域 {}",
        check.from.len(),
        check.into.len(),
        check.allow.len(),
        check.scope.join("、")
    );

    let changed = changed_paths(&root, &check)?;
    let mut outside_scope = 0;
    let mut violations = Vec::new();
    for path in &changed {
        if !in_scope(path, &check.scope) {
            outside_scope += 1;
        } else if check.allow.contains(path) {
            println!("  声明内改动：{path}");
        } else {
            println!("  ** 声明外改动：{path}");
            violations.push(path.clone());
        }
    }
    if outside_scope > 0 {
        println!("  （{outside_scope} 个路径在本批作用域之外，不参与判定）");
    }

    for path in &check.allow {
        if let Some(state) = &check.base_state {
            let head_side = check
                .head_state
                .as_ref()
                .map_or_else(|| root.clone(), PathBuf::from);
            let (before, after) = state_change_counts(&head_side, Path::new(state), path)?;
            println!("  {path}: {before} → {after} 行");
        } else {
            let (added, removed) = change_counts(&root, &check, path)?;
            println!("  {path}: +{added} −{removed}");
        }
    }

    let mut before = SourceItems::default();
    for path in &check.from {
        before.extend(collect_items(&read_base(&root, &check, path)?));
    }
    let mut after = SourceItems::default();
    for path in &check.into {
        after.extend(collect_items(&read_head(&root, &check, path)?));
    }
    if check.list_items {
        for (side, items) in [("搬迁前", &before), ("搬迁后", &after)] {
            println!("{side} {} 个定义项：", items.items.len());
            for item in &items.items {
                println!("  {} （{} 行）", item.key, item.body.len());
            }
            for line in &items.free {
                println!("  （项外）{line}");
            }
        }
        return Ok(());
    }

    let (comments, residual) = split_residual(side_residuals(&before, &after));
    println!(
        "定义项：搬迁前 {} 个（项内 {} 行），搬迁后 {} 个（项内 {} 行）；项外自由行 {} / {}",
        before.items.len(),
        before.item_lines(),
        after.items.len(),
        after.item_lines(),
        before.free.len(),
        after.free.len()
    );
    println!(
        "残留 {} 行（另有允许的新增注释 {} 行）",
        residual.len(),
        comments.len()
    );
    for residual in residual.iter().take(20) {
        if residual.context.is_empty() {
            println!("  {} {}", residual.direction, residual.line);
        } else {
            println!(
                "  {} [{}] {}",
                residual.direction, residual.context, residual.line
            );
        }
    }
    if residual.len() > 20 {
        println!("  …另有 {} 行未列出", residual.len() - 20);
    }

    if !violations.is_empty() {
        return Err(format!(
            "I1 不成立：声明外被改动的文件 {} 个（{}）",
            violations.len(),
            violations.join("、")
        ));
    }
    if !residual.is_empty() {
        return Err(format!(
            "I2 不成立：{} 行对不上（定义项逐项有序比对有差异）",
            residual.len()
        ));
    }
    if let Some(state) = &check.write_state {
        let written = write_state(&root, Path::new(state))?;
        println!("已写入批次状态：{state}（{written} 个 Rust 源文件）");
    }
    println!("MOVE-ONLY OK：I1（声明外零改动）与 I2（定义项逐项有序比对）都成立");
    Ok(())
}

fn parse_move_check(arguments: &[String]) -> Result<MoveCheck, String> {
    let mut check = MoveCheck {
        base: String::new(),
        head: None,
        base_state: None,
        head_state: None,
        write_state: None,
        list_items: false,
        from: Vec::new(),
        into: Vec::new(),
        allow: Vec::new(),
        scope: Vec::new(),
    };
    let mut index = 0;
    while let Some(flag) = arguments.get(index).cloned() {
        index += 1;
        match flag.as_str() {
            "--base" => check.base = flagged_value(arguments, &mut index, "--base")?,
            "--head" => check.head = Some(flagged_value(arguments, &mut index, "--head")?),
            "--from" => check
                .from
                .push(flagged_value(arguments, &mut index, "--from")?),
            "--into" => check
                .into
                .push(flagged_value(arguments, &mut index, "--into")?),
            "--allow" => check
                .allow
                .push(flagged_value(arguments, &mut index, "--allow")?),
            "--scope" => check
                .scope
                .push(flagged_value(arguments, &mut index, "--scope")?),
            "--base-state" => {
                check.base_state = Some(flagged_value(arguments, &mut index, "--base-state")?);
            }
            "--head-state" => {
                check.head_state = Some(flagged_value(arguments, &mut index, "--head-state")?);
            }
            "--write-state" => {
                check.write_state = Some(flagged_value(arguments, &mut index, "--write-state")?);
            }
            "--list-items" => check.list_items = true,
            other => return Err(format!("未知参数：{other}\n{CHECK_USAGE}")),
        }
    }
    if check.base.is_empty() && check.base_state.is_none() {
        return Err(format!("缺少 --base（或 --base-state）\n{CHECK_USAGE}"));
    }
    if check.from.is_empty() || check.into.is_empty() {
        return Err(format!(
            "--from 与 --into 各至少需要一个路径\n{CHECK_USAGE}"
        ));
    }
    if check.head.is_some() && check.head_state.is_some() {
        return Err(format!(
            "--head 与 --head-state 只能给一个（前者读提交，后者读批次快照）\n{CHECK_USAGE}"
        ));
    }
    if check.head_state.is_some() && check.base_state.is_none() {
        return Err(format!(
            "--head-state 要与 --base-state 一起用：两侧都是批次快照\n{CHECK_USAGE}"
        ));
    }
    if check.allow.is_empty() {
        // 默认只允许声明里的文件改动：被搬空的旧文件与被搬入的新文件（同一路径只算一次，
        // 被拆的文件同时出现在两侧）。
        let mut allow = check.from.clone();
        for path in &check.into {
            if !allow.contains(path) {
                allow.push(path.clone());
            }
        }
        check.allow = allow;
    }
    if check.scope.is_empty() {
        check.scope = scopes_for(&check.allow);
    }
    Ok(check)
}

fn flagged_value(arguments: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let value = arguments
        .get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} 需要参数\n{CHECK_USAGE}"))?;
    *index += 1;
    Ok(value)
}

/// 作用域默认取声明文件所在的 crate 目录（`crates/<名字>`）。仓库是共享检出，别的线程可能
/// 在别的 crate 里工作，默认只对自己相关的部分负责；要更宽的判定就显式给 `--scope`。
fn scopes_for(allowed: &[String]) -> Vec<String> {
    let mut scopes: Vec<String> = allowed
        .iter()
        .filter_map(|path| {
            let mut components = Path::new(path).components();
            let first = components
                .next()?
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            let second = components
                .next()?
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            Some(format!("{first}/{second}"))
        })
        .collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

fn in_scope(path: &str, scopes: &[String]) -> bool {
    scopes
        .iter()
        .any(|scope| Path::new(path).starts_with(scope))
}

/// 本批改动、新增、删除的路径。
///
/// `--head` 给了提交就问 `git diff`；没给就对着工作区，用 `git status` 连未跟踪的新文件
/// 一起算进来——搬迁的新模块文件在提交之前正是未跟踪状态。
fn changed_paths(root: &Path, check: &MoveCheck) -> Result<Vec<String>, String> {
    if let (Some(base), Some(head)) = (&check.base_state, &check.head_state) {
        return changed_between_states(Path::new(base), Path::new(head));
    }
    if let Some(state) = &check.base_state {
        return changed_against_state(root, Path::new(state));
    }
    let output = match &check.head {
        Some(head) => git_output(
            root,
            &["diff", "--name-only", "--no-renames", &check.base, head],
        )?,
        None => git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"])?,
    };
    let mut paths = BTreeSet::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if check.head.is_some() {
            paths.insert(line.to_owned());
            continue;
        }
        // porcelain 的形状是 `XY 路径`；改名写成 `旧 -> 新`，两侧都要算改动。
        let listed = line.get(3..).unwrap_or_default().trim();
        match listed.split_once(" -> ") {
            Some((old, new)) => {
                paths.insert(old.trim_matches('"').to_owned());
                paths.insert(new.trim_matches('"').to_owned());
            }
            None => {
                paths.insert(listed.trim_matches('"').to_owned());
            }
        }
    }
    Ok(paths.into_iter().collect())
}

/// 单个声明文件的增删行数：跟踪中的文件问 `git diff --numstat`，本批新加的文件直接数行。
///
/// 目录之外的路径（反向验证时拿一个改过一行的副本当 `--into`，见 `docs/08`）不走 git，
/// 直接按新文件数行。
fn change_counts(root: &Path, check: &MoveCheck, path: &str) -> Result<(usize, usize), String> {
    if !root.join(path).starts_with(root) {
        return new_file_lines(root, path);
    }
    let range: Vec<String> = match &check.head {
        Some(head) => vec![check.base.clone(), head.clone()],
        None => vec![check.base.clone()],
    };
    let mut arguments: Vec<&str> = vec!["diff", "--numstat", "--no-renames"];
    arguments.extend(range.iter().map(String::as_str));
    arguments.extend(["--", path]);
    let output = git_output(root, &arguments)?;
    if let Some(line) = output.lines().next() {
        let mut fields = line.split('\t');
        let added = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let removed = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        return Ok((added, removed));
    }
    // numstat 为空有两种可能：文件没变，或者它根本不在提交里。跟踪中的文件按「没变」算，
    // 否则会把一个没动过的文件报成「新增 N 行」。
    if !git_output(root, &["ls-files", "--", path])?
        .trim()
        .is_empty()
    {
        return Ok((0, 0));
    }
    new_file_lines(root, path)
}

fn new_file_lines(root: &Path, path: &str) -> Result<(usize, usize), String> {
    let file = root.join(path);
    if file.is_file() {
        let lines = fs::read_to_string(&file)
            .map_err(|error| format!("无法读取 {}：{error}", file.display()))?
            .lines()
            .count();
        Ok((lines, 0))
    } else {
        Ok((0, 0))
    }
}

fn read_at(root: &Path, revision: &str, path: &str) -> Result<String, String> {
    git_output(root, &["show", &format!("{revision}:{path}")])
        .map_err(|error| format!("{error}（路径 {path}，提交 {revision}）"))
}

/// 搬迁前的文件内容：有基准状态就从它读，否则从提交读。
fn read_base(root: &Path, check: &MoveCheck, path: &str) -> Result<String, String> {
    let Some(state) = &check.base_state else {
        return read_at(root, &check.base, path);
    };
    let file = Path::new(state).join(path);
    if file.is_file() {
        return fs::read_to_string(&file)
            .map_err(|error| format!("无法读取 {}：{error}", file.display()));
    }
    Err(format!(
        "基准状态里没有 {path}（{}）：它应当是上一批之后新增的文件，不能出现在 --from 里",
        file.display()
    ))
}

/// 工作区与上一批的状态相比，哪些 Rust 源文件变了。
///
/// 它不读未提交的工作区状态，也不受别的 crate 的并行改动影响：两边列同一批路径，内容不同
/// （含一边不存在，即新增或删除）就是改动。
fn changed_against_state(root: &Path, state: &Path) -> Result<Vec<String>, String> {
    let mut paths = BTreeSet::new();
    for path in state_paths(root)? {
        if fs::read(root.join(&path)).ok() != fs::read(state.join(&path)).ok() {
            paths.insert(path);
        }
    }
    for path in state_paths(state)? {
        if !root.join(&path).is_file() {
            paths.insert(path);
        }
    }
    Ok(paths.into_iter().collect())
}

/// 两份批次快照之间哪些 Rust 源文件变了。
///
/// 有了它，一批的判定不依赖「工作区恰好停在那批之后」：快照就是证据，随时可以重跑任意一批。
fn changed_between_states(base: &Path, head: &Path) -> Result<Vec<String>, String> {
    let mut paths = BTreeSet::new();
    for path in state_paths(base)? {
        if fs::read(base.join(&path)).ok() != fs::read(head.join(&path)).ok() {
            paths.insert(path);
        }
    }
    for path in state_paths(head)? {
        if !base.join(&path).is_file() {
            paths.insert(path);
        }
    }
    Ok(paths.into_iter().collect())
}

/// 基准状态与本批的单文件行数，用来报告搬迁规模（`0` 表示该侧没有这个文件）。
fn state_change_counts(root: &Path, state: &Path, path: &str) -> Result<(usize, usize), String> {
    let count = |base: &Path| -> Result<usize, String> {
        let file = base.join(path);
        if !file.is_file() {
            return Ok(0);
        }
        Ok(fs::read_to_string(&file)
            .map_err(|error| format!("无法读取 {}：{error}", file.display()))?
            .lines()
            .count())
    };
    Ok((count(state)?, count(root)?))
}

/// 把当前状态写成一份快照，作为下一批的基准。目录存在时先清空，否则上一批删掉的文件会
/// 留在里面，下一批会把它们当成「工作区里被删了」。
fn write_state(root: &Path, state: &Path) -> Result<usize, String> {
    if state.exists() {
        fs::remove_dir_all(state)
            .map_err(|error| format!("无法清理 {}：{error}", state.display()))?;
    }
    let paths = state_paths(root)?;
    for path in &paths {
        let destination = state.join(path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建 {}：{error}", parent.display()))?;
        }
        fs::copy(root.join(path), &destination)
            .map_err(|error| format!("无法写入 {}：{error}", destination.display()))?;
    }
    Ok(paths.len())
}

/// 状态快照覆盖的路径：`crates/` 下每个 `.rs`，写成相对仓库根的路径。还没有快照时返回
/// 空列表（「什么都还没记下」，因此每个文件都算新改动）。
fn state_paths(base: &Path) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    if base.join(STATE_ROOT).is_dir() {
        collect_rust_sources(&base.join(STATE_ROOT), STATE_ROOT, &mut paths)?;
    }
    paths.sort();
    Ok(paths)
}

fn collect_rust_sources(
    directory: &Path,
    prefix: &str,
    into: &mut Vec<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("无法读取 {}：{error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("无法读取目录项：{error}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("名字不是有效 UTF-8：{}", directory.display()))?;
        let relative = format!("{prefix}/{name}");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, &relative, into)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        {
            into.push(relative);
        }
    }
    Ok(())
}

/// 搬迁后的文件内容：`--head-state` 给了就从那份快照读，`--head` 给了就读那个提交，否则读
/// 工作区；文件不存在按空内容处理（纯改名时旧路径正是这样，快照里还没出现的新文件同理）。
fn read_head(root: &Path, check: &MoveCheck, path: &str) -> Result<String, String> {
    if let Some(state) = &check.head_state {
        return read_state_file(Path::new(state), path);
    }
    match check.head.as_deref() {
        Some(head) => read_at(root, head, path),
        None => read_state_file(root, path),
    }
}

/// 从一个目录（工作区或批次快照）读文件；不存在按空内容处理。
fn read_state_file(root: &Path, path: &str) -> Result<String, String> {
    let file = root.join(path);
    if file.is_file() {
        return fs::read_to_string(&file)
            .map_err(|error| format!("无法读取 {}：{error}", file.display()));
    }
    Ok(String::new())
}

fn without_visibility(line: &str) -> &str {
    let line = line.trim();
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    line.trim_start()
}

/// 逐行推进的花括号深度，跳过注释、字符串（含 `r#"…"#`）与字符字面量里的括号。
///
/// 朴素地数 `{` / `}` 会被字符串骗到：本仓的 SSE 测试里就有只写了一半括号的断言，一旦深度
/// 失衡，后面所有定义项都会被吞进同一个项里。判据按项分组，所以这是正确性问题。
///
/// 不需要完整词法：只需要知道哪些括号在字符串或注释里。生命周期 `'a` 不能被当成字符字面量，
/// 所以只有 `'x'` / `'\…'` 的形状才算。
#[derive(Default)]
struct Braces {
    depth: i64,
    block_comment: usize,
    raw_string: Option<usize>,
    string: bool,
}

impl Braces {
    /// 吃进一行，返回这一行的括号净增量。
    fn feed(&mut self, line: &str) -> i64 {
        let before = self.depth;
        let characters: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            let character = characters[index];
            if self.block_comment > 0 {
                if character == '/' && characters.get(index + 1) == Some(&'*') {
                    self.block_comment += 1;
                    index += 2;
                } else if character == '*' && characters.get(index + 1) == Some(&'/') {
                    self.block_comment -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }
            if let Some(hashes) = self.raw_string {
                if character == '"' && raw_closes(&characters, index + 1, hashes) {
                    self.raw_string = None;
                    index += hashes + 2;
                } else {
                    index += 1;
                }
                continue;
            }
            if self.string {
                match character {
                    '\\' => index += 2,
                    '"' => {
                        self.string = false;
                        index += 1;
                    }
                    _ => index += 1,
                }
                continue;
            }
            match character {
                '/' if characters.get(index + 1) == Some(&'/') => break,
                '/' if characters.get(index + 1) == Some(&'*') => {
                    self.block_comment = 1;
                    index += 2;
                }
                '"' => {
                    self.string = true;
                    index += 1;
                }
                'r' if raw_starts(&characters, index) => {
                    self.raw_string = Some(raw_hashes(&characters, index));
                    index += raw_hashes(&characters, index) + 2;
                }
                '\'' if is_char_literal(&characters, index) => {
                    index = char_literal_end(&characters, index);
                }
                '{' => {
                    self.depth += 1;
                    index += 1;
                }
                '}' => {
                    self.depth -= 1;
                    index += 1;
                }
                _ => index += 1,
            }
        }
        self.depth - before
    }
}

/// 这一位置看起来是原始字符串的开头（`r"` / `r#"` / `br"`）时，它的 `#` 个数。
fn raw_hashes(characters: &[char], index: usize) -> usize {
    let mut hashes = 0;
    while characters.get(index + 1 + hashes) == Some(&'#') {
        hashes += 1;
    }
    hashes
}

fn raw_starts(characters: &[char], index: usize) -> bool {
    characters.get(index + 1 + raw_hashes(characters, index)) == Some(&'"')
}

/// 原始字符串在这个 `"` 处结束（后面要跟同样多的 `#`）。
fn raw_closes(characters: &[char], index: usize, hashes: usize) -> bool {
    (0..hashes).all(|offset| characters.get(index + offset) == Some(&'#'))
}

/// 这一位置是字符字面量而不是生命周期标注。
fn is_char_literal(characters: &[char], index: usize) -> bool {
    match characters.get(index + 1) {
        Some('\\') => true,
        Some(_) => characters.get(index + 2) == Some(&'\''),
        None => false,
    }
}

fn char_literal_end(characters: &[char], index: usize) -> usize {
    let mut cursor = index + 1;
    while cursor < characters.len() {
        match characters[cursor] {
            '\\' => cursor += 2,
            '\'' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    characters.len()
}

/// 一行的第一个关键字；只用来认出 `use` / `mod` / `impl`，所以按分隔符劈出第一个词就够。
fn first_keyword(line: &str) -> &str {
    line.split([' ', '(', '{']).next().unwrap_or_default()
}

/// 这一行是不是一个函数签名的开头（可见性前缀已经剥掉）。
fn is_signature_start(line: &str) -> bool {
    line.starts_with("fn ")
        || line.starts_with("async fn ")
        || line.starts_with("unsafe fn ")
        || line.starts_with("const fn ")
}

/// 这一行是不是一个定义项的开头（可见性前缀已经剥掉）。`impl` 与 `mod` 是透明容器，不算项。
fn is_item_start(line: &str) -> bool {
    is_signature_start(line)
        || matches!(
            first_keyword(line),
            "struct" | "enum" | "trait" | "union" | "type" | "const" | "static"
        )
}

/// 把一段可能被折行的签名合并成一行。
///
/// rustfmt 按行宽折行：同一个签名，加了 `pub(crate) ` 前缀就可能从一行变成四行（真实例子：
/// P1.8 的 `ensure_model_ready`）。合并时顺手去掉折行留下的空白（`( `、` )`、` ,`、`, `
/// 与尾随逗号 `,)`），于是「同一个签名、不同折行」算同一行；参数名、类型、返回值改了
/// 仍然会在多重集里露出残留。
fn join_signature(text: &str) -> String {
    let mut joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [("( ", "("), (", ", ","), (" )", ")"), (",)", ")")] {
        while joined.contains(from) {
            joined = joined.replace(from, to);
        }
    }
    joined
}

/// 单行归一化：丢缩进与公开性前缀。
///
/// 可见性从私有到 `pub(crate)` 是搬迁必然要改的（模块边界变了），缩进随层级变化，都不算
/// 代码差异。
fn normalized_line(line: &str) -> String {
    without_visibility(line).to_owned()
}

/// 一段代码块（一个定义项的内容）的归一化：丢空行与整条 `use` 语句，把折行的签名合并成一行。
///
/// `use` 整条丢：rustfmt 按名字长度折行，而名字在搬迁里本来就会变，只看行首会把折行位置的
/// 差异当成代码差异。丢掉的接线由编译器与 clippy 接力把关：漏导入编不过，多余导入有
/// `unused_imports`。签名折行同理只算排版（见 `join_signature`）。除此之外每一行都必须原样
/// 出现在另一侧，**且顺序不变**。
fn normalize_block(block: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_use_statement = false;
    let mut signature: Option<String> = None;
    for raw in block {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if in_use_statement {
            in_use_statement = !line.ends_with(';');
            continue;
        }
        let bare = normalized_line(line);
        if let Some(pending) = signature.as_mut() {
            pending.push(' ');
            pending.push_str(&bare);
            if bare.ends_with('{') || bare.ends_with(';') {
                lines.push(join_signature(&signature.take().unwrap_or_default()));
            }
            continue;
        }
        if first_keyword(&bare) == "use" {
            in_use_statement = !bare.ends_with(';');
            continue;
        }
        if is_signature_start(&bare) {
            if bare.ends_with('{') || bare.ends_with(';') {
                lines.push(join_signature(&bare));
            } else {
                signature = Some(bare);
            }
            continue;
        }
        lines.push(bare);
    }
    if let Some(pending) = signature {
        lines.push(join_signature(&pending));
    }
    lines
}

/// 一个定义项：`key` 决定这是「哪一个项」，`body` 是它归一化后的**有序**内容。
///
/// `key` 由容器身份（`impl` 头）与归一化签名/类型头组成，于是「同一个项换到哪个文件」不影响
/// 身份（那正是搬迁要做的），而「两个 `impl` 里的同名方法对调实现」分成两个不同的 key。
struct Item {
    key: String,
    body: Vec<String>,
}

/// 一个源文件切出来的东西：定义项，以及项外的自由行（注释等）。
///
/// 自由行按多重集比对，所以新模块的文档头（新增注释）是允许的，删掉一行旧注释是残留。
#[derive(Default)]
struct SourceItems {
    items: Vec<Item>,
    free: Vec<String>,
}

impl SourceItems {
    /// 把另一个文件切出来的内容并进来：判据是「声明里所有文件合起来」这一层，不是单文件。
    fn extend(&mut self, other: SourceItems) {
        self.items.extend(other.items);
        self.free.extend(other.free);
    }

    fn item_lines(&self) -> usize {
        self.items.iter().map(|item| item.body.len()).sum()
    }
}

/// 把一个文件切成「定义项」与「项外自由行」。
///
/// - `fn`/`struct`/`enum`/`trait`/`union`/`const`/`static`/`type` 是**项**：内容按有序序列
///   比对，因此项内调换两行、把一个项拆到两个文件、两个 `impl` 的同名方法对调，都会留下残留。
/// - `impl` 与 `mod` 块是**透明容器**：头行不参与比对（`impl Store` 可以合法地一分为二，
///   `#[cfg(test)] mod tests` 可以合法地从一个变成多个），但 `impl` 头作为身份前缀进入内部每
///   一项的 key（`impl A for X | fn run(&self) -> u8`），`mod` 的名字不进——搬测试时
///   `mod tests` 会在每个目的文件里重建。
/// - 容器与项上的 cfg 条件都参与比对（见 `split_attributes`），因此「给容器加一条
///   `feature = …` 让里面的测试从编译里消失」不再隐形。
/// - `use` 语句、`mod x;` 声明与空行是接线，不参与。
fn collect_items(source: &str) -> SourceItems {
    let mut items = Vec::new();
    let mut free = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    // 透明容器栈：(容器的花括号深度, 内部项 key 的身份前缀, 内部项继承的 cfg 需求)。
    let mut containers: Vec<(i64, String, Vec<String>)> = Vec::new();
    let mut braces = Braces::default();
    let mut lines = source.lines();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("//") {
            free.append(&mut pending);
            free.push(line.to_owned());
            continue;
        }
        if line.starts_with("#[") {
            pending.push(line.to_owned());
            continue;
        }
        let depth_before = braces.depth;
        let bare = normalized_line(line);
        if first_keyword(&bare) == "use" {
            braces.feed(line);
            if !bare.ends_with(';') {
                for next in lines.by_ref() {
                    let text = next.trim();
                    braces.feed(text);
                    if text.ends_with(';') {
                        break;
                    }
                }
            }
            pending.clear();
            continue;
        }
        if bare == "}"
            && containers
                .last()
                .is_some_and(|(open, _, _)| *open == depth_before)
        {
            braces.feed(line);
            containers.pop();
            continue;
        }
        let keyword = first_keyword(&bare);
        if keyword == "mod" || keyword == "impl" {
            braces.feed(line);
            let (cfg, others) = split_attributes(&pending);
            pending.clear();
            free.extend(others);
            if keyword == "impl" || bare.ends_with('{') {
                let inherited = containers.last().cloned().unwrap_or_default();
                let prefix = if keyword == "impl" {
                    format!("{}{} | ", inherited.1, head_text(&bare))
                } else {
                    inherited.1
                };
                let mut chain = inherited.2;
                chain.extend(cfg);
                containers.push((braces.depth, prefix, chain));
            }
            continue;
        }
        if is_item_start(&bare) {
            let (cfg, others) = split_attributes(&pending);
            pending.clear();
            let inherited = containers.last().cloned().unwrap_or_default();
            let mut body = inherited.2;
            body.extend(cfg);
            body.extend(others);
            let block = item_block(&mut lines, &mut braces, line, depth_before);
            let normalized = normalize_block(&block);
            let key = format!("{}{}", inherited.1, item_key(&normalized));
            body.extend(normalized);
            items.push(Item { key, body });
            continue;
        }
        braces.feed(line);
        free.append(&mut pending);
        free.push(bare);
    }
    free.append(&mut pending);
    SourceItems { items, free }
}

/// 从头部行开始，把这一项的行整段取出来（含折行签名与多行初始化），并推进花括号深度。
///
/// 两种结束方式：头部自身就带闭合括号（`fn a() {}`）或以 `;` 结束（`type X = …;`）时，这一
/// 项就是这一行；否则继续读，直到花括号配平（`fn f() { … }`）或读到 `;`（多行初始化）。
fn item_block<'a, I>(
    lines: &mut I,
    braces: &mut Braces,
    head: &str,
    depth_before: i64,
) -> Vec<String>
where
    I: Iterator<Item = &'a str>,
{
    let mut block = vec![head.to_owned()];
    braces.feed(head);
    let mut opened = braces.depth > depth_before;
    let mut finished = !opened && (head.contains('{') || head.ends_with(';'));
    while !opened && !finished {
        let Some(next) = lines.next() else { break };
        let text = next.trim();
        block.push(text.to_owned());
        braces.feed(text);
        opened = braces.depth > depth_before;
        finished = !opened && (text.contains('{') || text.ends_with(';'));
    }
    while opened && braces.depth > depth_before {
        let Some(next) = lines.next() else { break };
        let text = next.trim();
        block.push(text.to_owned());
        braces.feed(text);
    }
    block
}

/// 待挂到下一个项或容器上的属性行，分成 cfg 需求与别的属性。
///
/// cfg 需求做成 `cfg: …` 行参与比对：`#[cfg(test)]` 什么都不留（搬测试必然要在新位置补上
/// 它），`#[cfg(all(test, feature = "x"))]` 留下 `cfg: all(feature="x")`。别的属性
/// （`#[derive]`、`#[test]`、`#[serde]`…）原样保留：删掉 `#[test]` 是残留。
fn split_attributes(attributes: &[String]) -> (Vec<String>, Vec<String>) {
    let mut cfg = Vec::new();
    let mut others = Vec::new();
    for attribute in attributes {
        let line = normalized_line(attribute);
        if is_cfg_attribute(&line) {
            if let Some(requirement) = cfg_requirement(&line) {
                cfg.push(format!("cfg: {requirement}"));
            }
        } else {
            others.push(line);
        }
    }
    (cfg, others)
}

fn is_cfg_attribute(attribute: &str) -> bool {
    attribute.starts_with("#[cfg(") && attribute.ends_with(")]")
}

/// `#[cfg(...)]` 里去掉 `test` 谓词之后剩下的条件；没有条件就是 `None`。
fn cfg_requirement(attribute: &str) -> Option<String> {
    let predicate = attribute.strip_prefix("#[cfg(")?.strip_suffix(")]")?;
    let requirement = rewrite_cfg(predicate);
    (!requirement.is_empty()).then_some(requirement)
}

/// 只去掉 `test` 谓词与空白：`test` → 空、`all(test, feature = "x")` → `all(feature="x")`，
/// 而 `feature = "test-mode"` 里的字符串原样保留（只在字符串外认标识符）。
fn rewrite_cfg(predicate: &str) -> String {
    let mut kept = String::new();
    let mut characters = predicate.chars().peekable();
    let mut in_string = false;
    while let Some(character) = characters.next() {
        if in_string {
            kept.push(character);
            if character == '\\' {
                if let Some(escaped) = characters.next() {
                    kept.push(escaped);
                }
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            kept.push(character);
            continue;
        }
        if character.is_alphanumeric() || character == '_' {
            let mut word = String::from(character);
            while characters
                .peek()
                .is_some_and(|next| next.is_alphanumeric() || *next == '_')
            {
                word.push(characters.next().unwrap_or_default());
            }
            if word != "test" {
                kept.push_str(&word);
            }
            continue;
        }
        if !character.is_whitespace() {
            kept.push(character);
        }
    }
    tidy_cfg(&kept)
}

/// 收拾 `test` 被拿掉后留下的空分组：`all(,feature="x")` → `all(feature="x")`；
/// `all(test)` 与 `not(test)` 只剩空括号，什么都不留。
fn tidy_cfg(kept: &str) -> String {
    let mut tidied = kept.to_owned();
    for (from, to) in [("(,", "("), (",)", ")"), (",,", ",")] {
        while tidied.contains(from) {
            tidied = tidied.replace(from, to);
        }
    }
    if tidied.contains("()") || !tidied.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    tidied
}

/// 项与容器的身份文本：归一化后的头部行，截到第一个 ` {`、` = ` 或 ` where `。
fn head_text(head: &str) -> String {
    let cut = [" {", " = ", " where "]
        .iter()
        .filter_map(|pattern| head.find(pattern))
        .min()
        .unwrap_or(head.len());
    head[..cut].trim().to_owned()
}

/// 项的身份：归一化内容的第一行再截到 `{` / `=` / ` where ` 之前，于是折行与否不影响身份，
/// 而参数、类型、返回值的变化（同一个名字换了签名）会分成两个 key。
fn item_key(normalized: &[String]) -> String {
    normalized
        .first()
        .map_or_else(String::new, |head| head_text(head))
}

/// 残留：方向（`+` 只在搬迁后出现，`-` 只在搬迁前出现，`~` 内容相同但行序变了）、归一化后的
/// 行内容，以及它所属的定义项（项外自由行与行序标记为空串）。
///
/// 上下文只用于打印与归类，不参与判定。
#[derive(Debug)]
struct Residual {
    direction: char,
    line: String,
    context: String,
}

/// 把残留分成「允许的新增注释」与「真正的残留」。
///
/// 新模块需要自己的文档头，所以新增的注释行是搬迁的自然产物；删除的注释行不算——那是丢
/// 文档，不是搬迁。判定只看方向与行首，不看内容，因此对两侧都存在的注释（搬过去的那些）
/// 没有影响：它们的计数在多重集里已经互相抵消。
fn split_residual(residual: Vec<Residual>) -> (Vec<Residual>, Vec<Residual>) {
    residual
        .into_iter()
        .partition(|residual| residual.direction == '+' && residual.line.starts_with("//"))
}

/// 两侧行多重集的差：`+` 只在搬迁后出现，`-` 只在搬迁前出现。
fn residual_lines(before: &[String], after: &[String]) -> Vec<Residual> {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for line in before {
        *counts.entry(line.as_str()).or_default() -= 1;
    }
    for line in after {
        *counts.entry(line.as_str()).or_default() += 1;
    }
    let mut residual = Vec::new();
    for (line, count) in counts {
        for _ in 0..count.unsigned_abs() {
            residual.push(Residual {
                direction: if count > 0 { '+' } else { '-' },
                line: line.to_owned(),
                context: String::new(),
            });
        }
    }
    residual
}

/// 同一个 key 下，两侧各自的「项内容」列表（内容归一化后是有序行）。
type ItemBodies<'a> = (Vec<&'a [String]>, Vec<&'a [String]>);

/// 逐项比对。同 key 的项按**有序内容**配对：内容完全相同的互相抵消（于是项整体换文件、
/// `impl` 块一分为二都算搬迁成功），一侧多出来的整项点名为新增/删除；成对的行多重集不同就
/// 列出差异行；行多重集相同但**顺序不同**的成对项留下一个 `~` 行序标记。
///
/// 行序标记这一类是池化多重集看不见的：项内调换两条语句、对调两个匹配分支，行集合一字不变，
/// 只有顺序变了。跨文件再分配同理——把项拆到两个文件会让两侧各多出对不上的项。
fn item_residuals(before: &[Item], after: &[Item]) -> Vec<Residual> {
    let mut groups: BTreeMap<&str, ItemBodies<'_>> = BTreeMap::new();
    for item in before {
        groups
            .entry(item.key.as_str())
            .or_default()
            .0
            .push(&item.body);
    }
    for item in after {
        groups
            .entry(item.key.as_str())
            .or_default()
            .1
            .push(&item.body);
    }
    let mut residual = Vec::new();
    for (key, (before, after)) in groups {
        let (mut before, mut after) = (before, after);
        before.sort();
        after.sort();
        let mut before_only = Vec::new();
        let mut after_only = Vec::new();
        let (mut left, mut right) = (0, 0);
        while left < before.len() || right < after.len() {
            match (before.get(left), after.get(right)) {
                (Some(one), Some(other)) => match one.cmp(other) {
                    std::cmp::Ordering::Equal => {
                        left += 1;
                        right += 1;
                    }
                    std::cmp::Ordering::Less => {
                        before_only.push(*one);
                        left += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        after_only.push(*other);
                        right += 1;
                    }
                },
                (Some(one), None) => {
                    before_only.push(*one);
                    left += 1;
                }
                (None, Some(other)) => {
                    after_only.push(*other);
                    right += 1;
                }
                (None, None) => break,
            }
        }
        for (before_body, after_body) in before_only.iter().zip(after_only.iter()) {
            let lines = residual_lines(before_body, after_body);
            if lines.is_empty() {
                residual.push(Residual {
                    direction: '~',
                    line: "行序改变（行多重集相同）".to_owned(),
                    context: key.to_owned(),
                });
            } else {
                residual.extend(lines.into_iter().map(|mut line| {
                    key.clone_into(&mut line.context);
                    line
                }));
            }
        }
        for body in before_only.iter().skip(after_only.len()) {
            residual.extend(body.iter().map(|line| Residual {
                direction: '-',
                line: line.clone(),
                context: key.to_owned(),
            }));
        }
        for body in after_only.iter().skip(before_only.len()) {
            residual.extend(body.iter().map(|line| Residual {
                direction: '+',
                line: line.clone(),
                context: key.to_owned(),
            }));
        }
    }
    residual
}

/// 两侧（声明里所有文件合起来）的完整判据：逐项比对 + 项外自由行的多重集比对。
fn side_residuals(before: &SourceItems, after: &SourceItems) -> Vec<Residual> {
    let mut residual = item_residuals(&before.items, &after.items);
    residual.extend(residual_lines(&before.free, &after.free));
    residual
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| format!("无法运行 git：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} 失败：{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| format!("git {} 的输出不是 UTF-8", arguments.join(" ")))
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

    /// 两侧各是一份源码文本，跑的就是 `check-move-only` 的 I2 判据。
    fn side_check(before: &str, after: &str) -> (Vec<Residual>, Vec<Residual>) {
        split_residual(side_residuals(
            &collect_items(before),
            &collect_items(after),
        ))
    }

    /// 多份源码加起来才算一侧：判据是「声明里所有文件合起来」，不是单文件。
    fn multi_side_check(before: &[&str], after: &[&str]) -> (Vec<Residual>, Vec<Residual>) {
        let mut before_side = SourceItems::default();
        for source in before {
            before_side.extend(collect_items(source));
        }
        let mut after_side = SourceItems::default();
        for source in after {
            after_side.extend(collect_items(source));
        }
        split_residual(side_residuals(&before_side, &after_side))
    }

    /// 归一化只丢缩进、空行、整条 `use` 语句、`mod x;` 声明与公开性前缀：项被切出来，代码一字
    /// 不差，`impl` 头进身份前缀，`mod` 名不进。
    #[test]
    fn move_only_normalization_drops_layout_and_visibility_but_keeps_code() {
        let items = collect_items(
            "use super::Store;\n\n    pub(crate) async fn save() {\n        let now = 1;\n    }\nmod command;\nimpl Store {\n    fn a() {}\n}\n",
        );
        assert!(items.free.is_empty(), "自由行不该有：{:?}", items.free);
        assert_eq!(items.items.len(), 2);
        assert_eq!(items.items[0].key, "async fn save()");
        assert_eq!(
            items.items[0].body,
            vec![
                "async fn save() {".to_owned(),
                "let now = 1;".to_owned(),
                "}".to_owned(),
            ]
        );
        assert_eq!(items.items[1].key, "impl Store | fn a()");
    }

    /// 批次状态快照：空快照下每个文件都算改动，写入后立刻变成「零改动」，改一行、删一个
    /// 文件又各自被认出来。
    #[test]
    fn batch_state_round_trip_detects_edits_additions_and_deletions() {
        let workspace = scratch_dir("state-workspace");
        let sources = workspace.join(STATE_ROOT).join("demo").join("src");
        fs::create_dir_all(&sources).unwrap();
        fs::write(sources.join("lib.rs"), "fn a() {}\n").unwrap();
        fs::write(sources.join("main.rs"), "fn b() {}\n").unwrap();
        let state = scratch_path("batch-state", "dir");

        // 还没有状态：两个文件都算新增。
        assert_eq!(changed_against_state(&workspace, &state).unwrap().len(), 2);

        assert_eq!(write_state(&workspace, &state).unwrap(), 2);
        assert!(matches!(
            changed_against_state(&workspace, &state)
                .unwrap()
                .as_slice(),
            []
        ));

        fs::write(sources.join("lib.rs"), "fn a() { let x = 1; }\n").unwrap();
        assert_eq!(
            changed_against_state(&workspace, &state).unwrap(),
            vec!["crates/demo/src/lib.rs".to_owned()]
        );
        fs::remove_file(sources.join("main.rs")).unwrap();
        assert_eq!(
            changed_against_state(&workspace, &state).unwrap(),
            vec![
                "crates/demo/src/lib.rs".to_owned(),
                "crates/demo/src/main.rs".to_owned()
            ]
        );
    }

    /// `use` 语句被 rustfmt 折行的位置随名字长度变化，整条语句都算接线。
    #[test]
    fn wrapped_use_statements_are_wiring_not_code() {
        let (comments, residual) = side_check(
            "use x::{long_name_a, long_name_b};\nfn save() {}\n",
            "use x::{\n    long_name_a,\n    long_name_c,\n};\nfn save() {}\n",
        );
        assert!(comments.is_empty());
        assert!(residual.is_empty(), "折行的 `use` 不该出残留：{residual:?}");
    }

    /// 同一个签名因为多了可见性前缀而被 rustfmt 折行，也只是排版：合并成一行后两侧相等。
    #[test]
    fn a_wrapped_signature_is_layout_not_code() {
        let (_, residual) = side_check(
            "async fn ensure_model_ready(state: &AppState, project_id: ProjectId) -> Result<(), ApiError> {\n    let ready = true;\n    Ok(())\n}\n",
            "pub(crate) async fn ensure_model_ready(\n    state: &AppState,\n    project_id: ProjectId,\n) -> Result<(), ApiError> {\n    let ready = true;\n    Ok(())\n}\n",
        );
        assert!(residual.is_empty(), "折行不该出残留：{residual:?}");
    }

    /// 合并折行不等于放过签名：参数名或类型改了必须报残留。
    #[test]
    fn a_changed_signature_parameter_still_shows_up() {
        let (comments, residual) = side_check(
            "fn save(a: A) -> B {\n    let ready = true;\n    Ok(())\n}\n",
            "pub(crate) fn save(\n    a: A,\n    c: C,\n) -> B {\n    let ready = true;\n    Ok(())\n}\n",
        );
        assert!(comments.is_empty());
        assert!(!residual.is_empty(), "签名内容变了必须报残留");
    }

    /// 以 `use` / `mod` / `impl` 开头的标识符不是模块接线，不能被归一化吃掉。
    #[test]
    fn move_only_normalization_keeps_identifiers_that_start_like_module_lines() {
        let items = collect_items("use_x();\nmodify();\nimplement();\nuse a::B;\nmod c;\n");
        assert!(items.items.is_empty(), "这里没有定义项");
        assert_eq!(
            items.free,
            vec![
                "use_x();".to_owned(),
                "modify();".to_owned(),
                "implement();".to_owned()
            ]
        );
    }

    /// 搬测试时源文件的一个 `#[cfg(test)] mod tests` 会变成目的文件里的多个：`mod` 是透明容器，
    /// 包装行数与模块名都不参与比对，被搬的测试本身逐项对上。
    #[test]
    fn moved_test_module_wrappers_are_wiring_not_code() {
        let module_a = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {\n        let x = 1;\n    }\n\n    #[test]\n    fn b() {\n        let y = 2;\n    }\n}\n";
        let module_b = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {\n        let x = 1;\n    }\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn b() {\n        let y = 2;\n    }\n}\n";
        let (_, residual) = side_check(module_a, module_b);
        assert!(residual.is_empty(), "模块包装不该出残留：{residual:?}");

        let changed = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {\n        let x = 9;\n    }\n\n    #[test]\n    fn b() {\n        let y = 2;\n    }\n}\n";
        let (_, residual) = side_check(module_a, changed);
        assert!(!residual.is_empty(), "包装内的代码改了必须报残留");
        assert_eq!(residual[0].context, "fn a()");
    }

    /// cfg 条件本身参与比对：`#[cfg(test)]` 算接线（搬测试必然要在新位置补上它），别的条件
    /// 无论在容器上还是项上都不能悄悄改动。
    #[test]
    fn cfg_conditions_other_than_test_participate_in_the_comparison() {
        let plain = "fn helper() {\n    let x = 1;\n}\n";
        let test_only = "#[cfg(test)]\nfn helper() {\n    let x = 1;\n}\n";
        let (_, residual) = side_check(plain, test_only);
        assert!(residual.is_empty(), "`#[cfg(test)]` 是接线：{residual:?}");

        let feature_gated =
            "#[cfg(all(test, feature = \"audit_probe\"))]\nfn helper() {\n    let x = 1;\n}\n";
        let (_, residual) = side_check(plain, feature_gated);
        assert!(!residual.is_empty(), "多出来的 cfg 条件必须报残留");
        assert_eq!(residual[0].line, "cfg: all(feature=\"audit_probe\")");

        // 同一条条件的不同空白写法算同一条件；`feature = "test-mode"` 里的字符串不是 `test` 谓词。
        let spaced =
            "#[cfg(all(test, feature=\"audit_probe\"))]\nfn helper() {\n    let x = 1;\n}\n";
        let (_, residual) = side_check(feature_gated, spaced);
        assert!(
            residual.is_empty(),
            "同一条 cfg 的不同空白写法不该出残留：{residual:?}"
        );
        let test_mode = "#[cfg(feature = \"test-mode\")]\nfn helper() {\n    let x = 1;\n}\n";
        let (_, residual) = side_check(test_mode, plain);
        assert!(!residual.is_empty(), "字符串里的 test 不是 test 谓词");

        // 容器上的 cfg 同理：改了它，内部每一项都对不上。
        let module = "#[cfg(test)]\nmod tests {\n    fn a() {\n        let x = 1;\n    }\n}\n";
        let gated = "#[cfg(all(test, feature = \"audit_probe\"))]\nmod tests {\n    fn a() {\n        let x = 1;\n    }\n}\n";
        let (_, residual) = side_check(module, gated);
        assert!(!residual.is_empty(), "容器上的 cfg 条件改了必须报残留");
        assert_eq!(residual[0].context, "fn a()");
    }

    /// 搬迁的形状（加了接线行、可见性放宽、缩进变了）不该出残留。
    #[test]
    fn a_pure_move_leaves_no_residual() {
        let (_, residual) = side_check(
            "#[derive(Debug)]\n    pub enum ApiError {\n        NotFound,\n    }\nmod http;\npub(crate) use http::ApiError;\n",
            "#[derive(Debug)]\n    pub(crate) enum ApiError {\n        NotFound,\n    }\n",
        );
        assert!(residual.is_empty(), "纯搬迁不该有残留：{residual:?}");
    }

    /// 新模块的文档头是允许的新增注释；删掉一行旧注释仍然是残留。
    #[test]
    fn new_module_comments_are_allowed_but_dropped_comments_are_not() {
        let (comments, residual) = side_check("fn save() {}\n", "//! 新模块文档。\nfn save() {}\n");
        assert_eq!(comments.len(), 1);
        assert!(residual.is_empty());

        let (comments, residual) = side_check("//! 旧模块文档。\nfn save() {}\n", "fn save() {}\n");
        assert!(comments.is_empty());
        assert_eq!(residual.len(), 1);
        assert_eq!(residual[0].direction, '-');
        assert_eq!(residual[0].line, "//! 旧模块文档。");
        assert!(residual[0].context.is_empty(), "项外的注释没有所属项");
    }

    /// 反过来：改一行代码必须真的有反应，而不是两边都归一化成空气。
    #[test]
    fn a_single_changed_code_line_shows_up_as_residual() {
        let (comments, residual) = side_check(
            "fn save() {\n    let now = 1;\n}\n",
            "fn save() {\n    let now = 2;\n}\n",
        );
        assert!(comments.is_empty());
        assert_eq!(residual.len(), 2);
        assert_eq!(residual[0].direction, '-');
        assert_eq!(residual[0].line, "let now = 1;");
        assert_eq!(residual[1].direction, '+');
        assert_eq!(residual[1].line, "let now = 2;");
        assert_eq!(residual[0].context, "fn save()");
    }

    /// 池化多重集看不见的第一类改动：项内调换两条语句。行集合一字不变，只有顺序变了。
    #[test]
    fn reordered_statements_inside_one_item_are_caught() {
        let (_, residual) = side_check(
            "fn save() {\n    let a = 1;\n    let b = 2;\n}\n",
            "fn save() {\n    let b = 2;\n    let a = 1;\n}\n",
        );
        assert_eq!(residual.len(), 1, "{residual:?}");
        assert_eq!(residual[0].direction, '~');
        assert_eq!(residual[0].context, "fn save()");
        assert_eq!(residual[0].line, "行序改变（行多重集相同）");
    }

    /// 对调两个匹配分支的实现（真行为改动）同样要被抓到，并点出是哪个函数的哪两条分支。
    #[test]
    fn swapped_match_arms_are_caught() {
        let (_, residual) = side_check(
            "fn dispatch(kind: Kind) -> Task {\n    match kind {\n        Kind::Outline => outline(),\n        Kind::Body => body(),\n    }\n}\n",
            "fn dispatch(kind: Kind) -> Task {\n    match kind {\n        Kind::Outline => body(),\n        Kind::Body => outline(),\n    }\n}\n",
        );
        assert!(!residual.is_empty());
        assert_eq!(residual[0].context, "fn dispatch(kind: Kind) -> Task");
        assert!(
            residual
                .iter()
                .any(|line| line.direction == '-' && line.line == "Kind::Outline => outline(),")
        );
    }

    /// 项整体换文件是搬迁；把同一个项拆到两个文件不是。
    #[test]
    fn relocating_a_whole_item_is_a_move_but_splitting_one_is_not() {
        let whole = "fn save() {\n    let one = 1;\n    let two = 2;\n}\n";
        let (_, residual) = multi_side_check(&[whole], &["", whole]);
        assert!(residual.is_empty(), "项整体换文件算搬迁：{residual:?}");

        let (_, residual) = multi_side_check(
            &[whole],
            &["fn save() {\n    let one = 1;\n", "    let two = 2;\n}\n"],
        );
        assert!(!residual.is_empty(), "拆到两个文件必须报残留");
    }

    /// `impl` 块可以合法地一分为二：判据的单位是块里的项，不是块本身。
    #[test]
    fn splitting_an_impl_block_is_layout_not_code() {
        let (_, residual) = side_check(
            "impl Store {\n    fn a() {}\n    fn b() {}\n}\n",
            "impl Store {\n    fn a() {}\n}\n\nimpl Store {\n    fn b() {}\n}\n",
        );
        assert!(residual.is_empty(), "`impl` 一分为二算排版：{residual:?}");

        // 但两个 `impl` 里的同名方法对调实现必须被抓到：`impl` 头在 key 里。
        let (_, residual) = side_check(
            "impl A for X {\n    fn run(&self) -> u8 {\n        1\n    }\n}\nimpl B for X {\n    fn run(&self) -> u8 {\n        2\n    }\n}\n",
            "impl A for X {\n    fn run(&self) -> u8 {\n        2\n    }\n}\nimpl B for X {\n    fn run(&self) -> u8 {\n        1\n    }\n}\n",
        );
        assert!(!residual.is_empty(), "同签名方法对调实现必须报残留");
        assert!(
            residual
                .iter()
                .any(|line| line.context == "impl A for X | fn run(&self) -> u8")
        );
    }

    /// 声明里没写的文件改动必须被抓出来；默认允许集合就是 `--from` 与 `--into`。
    #[test]
    fn move_check_defaults_declare_the_files_and_the_crate_scope() {
        let arguments: Vec<String> = [
            "--base",
            "HEAD",
            "--from",
            "crates/nekowriter-app/src/main.rs",
            "--into",
            "crates/nekowriter-app/src/http/error.rs",
        ]
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
        let check = parse_move_check(&arguments).unwrap();
        assert_eq!(check.base, "HEAD");
        assert!(check.head.is_none());
        assert_eq!(
            check.allow,
            vec![
                "crates/nekowriter-app/src/main.rs".to_owned(),
                "crates/nekowriter-app/src/http/error.rs".to_owned(),
            ]
        );
        assert_eq!(check.scope, vec!["crates/nekowriter-app".to_owned()]);
        assert!(in_scope(
            "crates/nekowriter-app/src/http/error.rs",
            &check.scope
        ));
        assert!(!in_scope(
            "crates/nekowriter-store/src/lib.rs",
            &check.scope
        ));

        assert!(parse_move_check(&[String::new()]).is_err());
        assert!(parse_move_check(&["--base".to_owned()]).is_err());
        assert!(
            parse_move_check(&["--base".to_owned(), "HEAD".to_owned(), "--from".to_owned(),])
                .is_err()
        );
    }
}
