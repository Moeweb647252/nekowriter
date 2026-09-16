//! 源码形状：事务外的回执预查只有一个所有者。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。
//! `include_str!` 的路径改成相对本文件所在的 `tests/`（P2.4）。
//! `preparation.rs` 变成目录模块后路径跟着改成 `../preparation/mod.rs`（P3.2）；P3.3–P3.17
//! 会把它拆成多个层文件，那时这里的 `files` 数组要按层文件逐个补（`docs/08` 的 P2.4 备注）。

use super::*;

/// 事务外的回执预查只有一个所有者：`Command::commit`。
///
/// 这条用例数的是源码形状，不是运行期行为。站点多查的那一次会把同一行的结果原样丢掉，
/// 没有任何断言看得出差别，所以运行期次数由 `docs/03` 的 SQL trace 现场核对：一次成功的
/// 写命令在事务前只查一次 `command_receipts`。形状在这里钉两侧：既预查又 `commit` 的函数
/// 只允许下面这五个（它们靠早退省掉 `embedding_profile_with_id()` 的读库，`reindex`
/// 还省掉整张 `knowledge_entry` 表的扫描）；而这五个必须仍然预查。
#[test]
fn transaction_external_recheck_has_a_single_owner() {
    const ALLOWED: &[&str] = &[
        "create_knowledge_entry",
        "delete_knowledge_entry",
        "insert_accumulated_entries",
        "reindex_all_knowledge",
        "update_knowledge_entry",
    ];
    // 只数生产代码：`lib.rs` 从这个文件的 `#[cfg(test)]` 起截掉，否则这条用例自己
    // 会把自己的断言文本数进去。
    let lib_source = include_str!("../lib.rs");
    let lib_source = lib_source
        .split("#[cfg(test)]")
        .next()
        .unwrap_or(lib_source);
    let files = [
        ("preparation/mod.rs", include_str!("../preparation/mod.rs")),
        ("knowledge.rs", include_str!("../knowledge.rs")),
        ("recycle.rs", include_str!("../recycle.rs")),
        ("lib.rs", lib_source),
        ("prompts.rs", include_str!("../prompts.rs")),
        (
            "prompt_templates.rs",
            include_str!("../prompt_templates.rs"),
        ),
        ("settings.rs", include_str!("../settings.rs")),
        (
            "project_settings.rs",
            include_str!("../project_settings.rs"),
        ),
        ("version_copy.rs", include_str!("../version_copy.rs")),
    ];

    let mut both = Vec::new();
    for (file, source) in files {
        for (function, body) in functions(source) {
            let prechecks = body.matches(".replay_if_recorded(").count();
            let commits = body.matches(".commit(self, Notify::").count();
            if prechecks == 0 || commits == 0 {
                continue;
            }
            assert_eq!(
                (prechecks, commits),
                (1, 1),
                "{file}::{function} 同时出现预查与 commit，但数量不是各一次"
            );
            both.push(function);
        }
    }
    both.sort_unstable();
    assert_eq!(
        both, ALLOWED,
        "既有事务外预查又调 commit 的函数，只允许白名单里那五个"
    );
}
