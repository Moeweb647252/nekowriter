use sea_orm_migration::prelude::*;

/// One active relation per ordered pair (spec/06 §1.2, spec/02 §2, D-143).
///
/// The SQL replaces the two `kind`-keyed partial unique indexes with pair-keyed ones. Creating a
/// unique index fails on a database that already violates it, and the author's database is exactly
/// the one this migration exists for; SQLite's own message names no ids, so the offending pairs are
/// read first and reported by id. Nothing is rewritten: choosing a survivor is the author's
/// decision, not a migration's.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0003_relation_pair_unique"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conflicts = active_pair_conflicts(manager.get_connection()).await?;
        if !conflicts.is_empty() {
            return Err(DbErr::Migration(format!(
                "0003_relation_pair_unique: {} pair(s) already hold more than one active relation \
                 — {}. End or delete the redundant rows and retry; this migration never picks a \
                 survivor (spec/06 §1.2).",
                conflicts.len(),
                conflicts
                    .iter()
                    .map(|c| format!(
                        "{}: #{} → #{} ({})",
                        c.scope, c.source_id, c.target_id, c.kinds
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        crate::execute_sql_file(manager, include_str!("sql/0003_relation_pair_unique.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Reverting would re-key the indexes on `kind`, which is the rule this change removes, so a
        // downgraded database would accept the duplicates again. Nothing else needs restoring: the
        // pair-keyed index is strictly stronger for every write the service allows.
        Err(DbErr::Migration(
            "0003_relation_pair_unique has no down migration".to_owned(),
        ))
    }
}

/// One canon that already holds two active relations for one ordered pair.
struct PairConflict {
    /// `shared` or `track #N`: the two canons the two unique indexes cover.
    scope: String,
    source_id: i32,
    target_id: i32,
    /// The colliding rows as `kind #id`, so the author can act on them directly.
    kinds: String,
}

/// The ordered pairs with more than one active relation, split by canon exactly as the two partial
/// indexes are: shared rows separately, each isolated track separately (spec/06 §2).
async fn active_pair_conflicts(
    conn: &impl sea_orm::ConnectionTrait,
) -> Result<Vec<PairConflict>, DbErr> {
    let rows = conn
        .query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT CASE WHEN track_id IS NULL THEN 'shared' ELSE 'track #' || track_id END AS scope, \
                    source_id, target_id, group_concat(kind || ' #' || id, ', ') AS kinds \
             FROM relations WHERE status = 'active' \
             GROUP BY track_id, source_id, target_id HAVING count(*) > 1 \
             ORDER BY track_id, source_id, target_id",
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok(PairConflict {
                scope: row.try_get("", "scope")?,
                source_id: row.try_get("", "source_id")?,
                target_id: row.try_get("", "target_id")?,
                kinds: row.try_get("", "kinds")?,
            })
        })
        .collect()
}
