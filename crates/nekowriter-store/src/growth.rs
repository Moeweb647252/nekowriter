//! 本地数据目录的长期增长口径。
//!
//! 事件流有保留期（见 `event_log`），而请求快照、命令回执与消息账本是**审计事实**，
//! 不能自动回收：它们回答「当时到底发了什么、为什么失败」。所以这里不做清理，只把
//! 各表当前的行数与重文本列字节数报出来，让长年使用后的体积是可诊断的，而不是等到
//! 备份或导出变慢才发现。

use sea_orm::{ConnectionTrait, DbBackend, Statement};

use super::{Store, StoreError};

/// 一张会长期增长的表当前占用的行数与重文本字节数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableGrowth {
    /// 表名，同时是对外报出的标识。
    pub table: &'static str,
    pub rows: i64,
    /// 该表登记的重文本列合计字节数；不含索引与页开销。
    pub text_bytes: i64,
}

/// 会被报出的表与它们各自的重文本列。
///
/// `events` 是有保留期的对照组：它随保留期保持在有限规模，其余几张则随使用量线性增长。
const TRACKED_TABLES: &[(&str, &[&str])] = &[
    (
        "model_requests",
        &["prompt_snapshot_json", "context_snapshot_json"],
    ),
    ("command_receipts", &["response_json"]),
    ("agent_messages", &["content_json"]),
    ("document_revisions", &["content", "structured_payload"]),
    ("events", &["payload_json"]),
];

impl Store {
    /// 统计会长期增长的表：行数与重文本列字节数。
    ///
    /// 这是诊断读数，不是清理入口：它不修改任何数据，也不为体积设上限。
    ///
    /// # Errors
    ///
    /// 统计查询失败时返回错误。
    pub async fn storage_growth(&self) -> Result<Vec<TableGrowth>, StoreError> {
        let mut report = Vec::with_capacity(TRACKED_TABLES.len());
        for (table, columns) in TRACKED_TABLES {
            let sizes = columns
                .iter()
                .map(|column| format!("COALESCE(LENGTH({column}), 0)"))
                .collect::<Vec<_>>()
                .join(" + ");
            let sql = format!(
                "SELECT COUNT(*) AS total_rows, COALESCE(SUM({sizes}), 0) AS total_bytes FROM {table}"
            );
            let row = self
                .database
                .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
                .await?
                .ok_or_else(|| {
                    StoreError::CorruptRecord(format!("{table} 的增长统计没有返回结果"))
                })?;
            report.push(TableGrowth {
                table,
                rows: row.try_get::<i64>("", "total_rows")?,
                text_bytes: row.try_get::<i64>("", "total_bytes")?,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// 空库也有完整读数；写入后行数与字节数都跟着涨。
    #[tokio::test]
    async fn growth_report_counts_rows_and_text_bytes() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();

        let empty = store.storage_growth().await.unwrap();
        assert_eq!(empty.len(), TRACKED_TABLES.len());
        assert!(
            empty
                .iter()
                .all(|table| table.rows == 0 && table.text_bytes == 0)
        );

        store
            .create_project(
                uuid::Uuid::new_v4(),
                nekowriter_core::NewProject::new("增长口径", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();

        let after = store.storage_growth().await.unwrap();
        let events = after
            .iter()
            .find(|table| table.table == "events")
            .expect("事件表必须在读数里");
        // 一次创建命令至少落一条事件，且它的正文被计入字节数。
        assert!(events.rows >= 1);
        assert!(events.text_bytes > 0);
        for table in &after {
            assert!(table.rows >= 0 && table.text_bytes >= 0);
        }
    }
}
