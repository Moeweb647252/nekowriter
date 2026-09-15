//! 持久事件类型名的唯一清单，以及它与写库点、界面订阅集合的对账。
//!
//! 事件类型名是后端与界面之间的合同：后端写一条事件，界面按名字决定刷新哪些查询。
//! 两侧各写一份时，漏同步不会有编译错误，也不会让任何请求失败——界面只是静默地不再
//! 刷新。这个模块把清单放在一处，并用三条断言钉住它：
//!
//! 1. 清单自身无重复且按字母序；
//! 2. `insert_event` / `insert_app_event` 的写库点不出现清单以外的名字；
//! 3. 清单与 `webui/src/lib/events.tsx` 的 `persistentEventTypes` 逐项相同。
//!
//! 新增一个事件类型时，第 2 条会先失败，把名字逼进清单；补进清单后第 3 条会接着失败，
//! 把订阅面一并补齐。因此清单本身不是运行时输入，而是这两条断言的对照物。

/// 全部持久事件类型名，按字母序排列。
pub const EVENT_TYPES: [&str; 19] = [
    "chapter.changed",
    "document.committed",
    "feedback.changed",
    "knowledge.index_changed",
    "model_request.cancelled",
    "project.changed",
    "prompt_template.changed",
    "question.answered",
    "question.created",
    "recycle.changed",
    "review.changed",
    "run.changed",
    "settings.changed",
    "system.model_operation",
    "tool.included",
    "tool.proposed",
    "tool.resolved",
    "version.completed",
    "workflow.changed",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::EVENT_TYPES;

    /// 前端订阅清单所在的位置，相对本 crate 的根目录。
    const FRONTEND_SUBSCRIPTIONS: &str = "../../webui/src/lib/events.tsx";
    /// 前端订阅清单的数组边界。
    const SUBSCRIPTION_ARRAY_START: &str = "const persistentEventTypes = [";
    const SUBSCRIPTION_ARRAY_END: &str = "] as const";

    /// 取出文本里所有双引号字面量的内容，按出现顺序返回。
    fn quoted_literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            found.push(after[..end].to_owned());
            rest = &after[end + 1..];
        }
        found
    }

    fn store_sources() -> Vec<PathBuf> {
        fn collect(directory: &Path, into: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(directory).expect("读取 store 源码目录") {
                let path = entry.expect("目录项").path();
                if path.is_dir() {
                    collect(&path, into);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    into.push(path);
                }
            }
        }
        let mut sources = Vec::new();
        collect(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut sources,
        );
        sources
    }

    /// 写出事件的那些调用点里，作为事件类型传入的第一个字面量。
    fn written_event_types() -> BTreeSet<String> {
        let mut written = BTreeSet::new();
        for path in store_sources() {
            // 本文件只描述合同，不写事件；它的字面量就是清单本身，不参与扫描。
            if path
                .file_name()
                .is_some_and(|name| name == "event_types.rs")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("读取 store 源文件");
            for marker in ["insert_event(", "insert_app_event("] {
                let mut from = 0;
                while let Some(offset) = source[from..].find(marker) {
                    let start = from + offset;
                    from = start + marker.len();
                    // 定义处的事件类型是形参，不是字面量。
                    if source[..start].trim_end().ends_with("fn") {
                        continue;
                    }
                    // 事件类型是调用里的第一个字面量（前两个实参是事务与稿本，都不是字面量）。
                    let window: String = source[from..].chars().take(400).collect();
                    if let Some(literal) = quoted_literals(&window).first() {
                        written.insert(literal.clone());
                    }
                }
            }
        }
        written
    }

    fn frontend_event_types() -> BTreeSet<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FRONTEND_SUBSCRIPTIONS);
        let source = std::fs::read_to_string(&path).expect("读取前端订阅清单");
        let start = source
            .find(SUBSCRIPTION_ARRAY_START)
            .expect("前端订阅数组起点")
            + SUBSCRIPTION_ARRAY_START.len();
        let end = start
            + source[start..]
                .find(SUBSCRIPTION_ARRAY_END)
                .expect("前端订阅数组终点");
        quoted_literals(&source[start..end]).into_iter().collect()
    }

    #[test]
    fn manifest_is_sorted_and_unique() {
        let unique: BTreeSet<&str> = EVENT_TYPES.into_iter().collect();
        assert_eq!(unique.len(), EVENT_TYPES.len(), "清单出现重复的名字");
        let mut sorted = EVENT_TYPES;
        sorted.sort_unstable();
        assert_eq!(sorted, EVENT_TYPES, "清单需按字母序");
    }

    #[test]
    fn every_written_event_type_is_in_the_manifest() {
        let written = written_event_types();
        assert!(
            written.contains("run.changed"),
            "扫描没有读到写库点，说明扫描规则已经失效"
        );
        let manifest: BTreeSet<String> = EVENT_TYPES
            .into_iter()
            .map(std::borrow::ToOwned::to_owned)
            .collect();
        assert_eq!(
            written, manifest,
            "写库点与清单不一致：新增或改名一个事件类型时，两侧必须一起更新"
        );
    }

    #[test]
    fn frontend_subscribes_to_exactly_the_manifest() {
        let frontend = frontend_event_types();
        let manifest: BTreeSet<String> = EVENT_TYPES
            .into_iter()
            .map(std::borrow::ToOwned::to_owned)
            .collect();
        assert_eq!(
            frontend, manifest,
            "后端写出的事件类型与前端订阅集合必须相同：少一个就会静默不刷新"
        );
    }
}
