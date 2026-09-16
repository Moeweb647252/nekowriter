//! `LanceDB` 冒烟：写入并查询一张表。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn lancedb_smoke_can_write_and_query_a_table() {
    let data_dir = TempDir::new().unwrap();
    let lance_dir = data_dir.path().join("lancedb");
    let connection = lancedb::connect(lance_dir.to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let rows = record_batch!(("id", Int32, [1, 2])).unwrap();
    let table = connection
        .create_table("m0_smoke", rows)
        .execute()
        .await
        .unwrap();
    let batches = table
        .query()
        .limit(1)
        .execute()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(
        batches
            .iter()
            .map(arrow_array::RecordBatch::num_rows)
            .sum::<usize>(),
        1
    );
}
