//! Dashboard 的一次性回算查询。
//!
//! 三项指标都不保存累计值：Token 用量来自已封存模型请求的 `usage_json`，任务
//! 计数来自 `runs` 行与工作流当前执行轴，字数增删来自 `document_revisions` 的
//! 修订链（相邻修订的非空白字符数之差）。因此统计永远与记录一致，重复执行同一
//! 查询会得到相同结果。

use std::collections::HashMap;

use nekowriter_core::{
    DASHBOARD_ROLE_EMBEDDING, Dashboard, DashboardDailyTokens, DashboardDailyWords, DashboardJobs,
    DashboardRange, DashboardReviewJob, DashboardTokenRole, DashboardTokens, DashboardWords,
    PersistedModelUsage, UsageUnknownReason, dashboard_usage_role, utc_now_millis,
};
use sea_orm::{ColumnTrait, DerivePartialModel, EntityTrait, QueryFilter, prelude::TextUuid};
use uuid::Uuid;

use super::{
    CHAPTER_BODY_ROLE, Store, StoreError, db_id,
    entity::document,
    entity::document_revision,
    entity::model_request,
    entity::novel_version,
    entity::run,
    entity::workflow_instance,
    model_requests::{ModelRequestStatus, model_request_status_from_db},
    parse_id,
    system_operations::{add_unknown_usage, usage_reason_order},
};

/// 顶层 Run 中仍处于进行中的状态。
const ACTIVE_RUN_STATUSES: [&str; 5] = ["queued", "running", "waiting", "paused", "interrupted"];

impl Store {
    /// 回算一次 Dashboard 统计。
    ///
    /// # Errors
    ///
    /// 数据库读取失败，或请求、工作流、修订记录损坏时返回错误。
    pub async fn compute_dashboard(
        &self,
        range: DashboardRange,
        project_id: Option<Uuid>,
    ) -> Result<Dashboard, StoreError> {
        let runs = self.dashboard_runs(project_id).await?;
        let tokens = self.dashboard_tokens(&range, &runs).await?;
        let jobs = self.dashboard_jobs(&runs).await?;
        let words = self.dashboard_words(&range, project_id).await?;
        Ok(Dashboard {
            generated_at_ms: utc_now_millis(),
            range,
            project_id,
            tokens,
            jobs,
            words,
        })
    }

    async fn dashboard_runs(
        &self,
        project_id: Option<Uuid>,
    ) -> Result<Vec<run::Model>, StoreError> {
        let mut query = run::Entity::find();
        if let Some(project_id) = project_id {
            query = query.filter(run::Column::ProjectId.eq(db_id(project_id)));
        }
        Ok(query.all(&self.database).await?)
    }

    /// 按角色与本地日汇总已封存请求报告的用量；未知用量的请求按原因单独计数。
    async fn dashboard_tokens(
        &self,
        range: &DashboardRange,
        runs: &[run::Model],
    ) -> Result<DashboardTokens, StoreError> {
        let run_by_id: HashMap<Uuid, &run::Model> =
            runs.iter().map(|run| (run.id.0, run)).collect();
        let requests = model_request::Entity::find()
            .filter(model_request::Column::CreatedAt.gte(range.start_ms))
            .filter(model_request::Column::CreatedAt.lt(range.end_ms))
            .all(&self.database)
            .await?;

        let days = range.day_indexes();
        let day_slot: HashMap<i64, usize> = days
            .iter()
            .enumerate()
            .map(|(index, day)| (*day, index))
            .collect();
        let mut tokens = DashboardTokens {
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            reasoning_tokens: 0,
            request_count: 0,
            unknown_usage_requests: 0,
            unknown_usage: Vec::new(),
            by_role: Vec::new(),
            daily: days
                .iter()
                .map(|day| DashboardDailyTokens {
                    day_start_ms: range.day_start_ms(*day),
                    input_tokens: 0,
                    output_tokens: 0,
                    request_count: 0,
                })
                .collect(),
            system_operations: self.dashboard_system_operations(range).await?,
            embedding: self.dashboard_embedding_usage(range).await?,
        };

        for request in requests {
            let Some(run) = run_by_id.get(&request.run_id.0) else {
                continue;
            };
            let role = dashboard_usage_role(&run.kind);
            if !tokens
                .by_role
                .iter()
                .any(|entry| entry.role == role.as_str())
            {
                tokens.by_role.push(DashboardTokenRole {
                    role: role.as_str().to_owned(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_input_tokens: 0,
                    reasoning_tokens: 0,
                    request_count: 0,
                    unknown_usage_requests: 0,
                });
            }
            let slot = tokens
                .by_role
                .iter_mut()
                .find(|entry| entry.role == role.as_str())
                .expect("角色条目刚刚按需创建");
            slot.request_count += 1;
            tokens.request_count += 1;
            let bucket = day_slot.get(&range.day_index(request.created_at)).copied();
            if let Some(index) = bucket {
                tokens.daily[index].request_count += 1;
            }

            let usage = persisted_usage(request.usage_json.as_deref(), &request.status)?;
            if !usage.is_reported() {
                let reason = usage.reason.unwrap_or(UsageUnknownReason::ProviderSilent);
                add_unknown_usage(&mut tokens.unknown_usage, reason);
                slot.unknown_usage_requests += 1;
                tokens.unknown_usage_requests += 1;
                continue;
            }
            let input = usage.input_tokens.unwrap_or(0);
            let output = usage.output_tokens.unwrap_or(0);
            let cached = usage.cached_input_tokens.unwrap_or(0);
            let reasoning = usage.reasoning_tokens.unwrap_or(0);
            slot.input_tokens += input;
            slot.output_tokens += output;
            slot.cached_input_tokens += cached;
            slot.reasoning_tokens += reasoning;
            tokens.input_tokens += input;
            tokens.output_tokens += output;
            tokens.cached_input_tokens += cached;
            tokens.reasoning_tokens += reasoning;
            if let Some(index) = bucket {
                tokens.daily[index].input_tokens += input;
                tokens.daily[index].output_tokens += output;
            }
        }
        // 规范把 Embedding 列为第三种角色（§12.1），所以角色镜头也要覆盖它：它是
        // 同一批向量用量的另一种镜头，不加进上面的对话总量。
        if tokens.embedding.request_count > 0 {
            let embedding = &tokens.embedding;
            tokens.by_role.push(DashboardTokenRole {
                role: DASHBOARD_ROLE_EMBEDDING.to_owned(),
                input_tokens: embedding.input_tokens,
                output_tokens: 0,
                cached_input_tokens: 0,
                reasoning_tokens: 0,
                request_count: embedding.request_count,
                unknown_usage_requests: embedding.unknown_usage_requests,
            });
        }
        tokens
            .by_role
            .sort_by(|left, right| left.role.cmp(&right.role));
        tokens
            .unknown_usage
            .sort_by_key(|entry| usage_reason_order(entry.reason));
        Ok(tokens)
    }

    /// 统计顶层 Run，并展开仍在进行的审核子 Run。
    async fn dashboard_jobs(&self, runs: &[run::Model]) -> Result<DashboardJobs, StoreError> {
        let mut execution_by_version: HashMap<Uuid, String> = HashMap::new();
        for run in runs {
            if run.parent_run_id.is_some() || run.kind == "review" || run.status != "waiting" {
                continue;
            }
            let version_id = run.version_id.0;
            if execution_by_version.contains_key(&version_id) {
                continue;
            }
            let Some(workflow) = workflow_instance::Entity::find_by_id(run.version_id)
                .one(&self.database)
                .await?
            else {
                continue;
            };
            let state: serde_json::Value =
                serde_json::from_str(&workflow.state_json).map_err(|error| {
                    StoreError::CorruptRecord(format!("工作流状态不是 JSON：{error}"))
                })?;
            if let Some(execution) = state.get("execution").and_then(|value| value.as_str()) {
                execution_by_version.insert(version_id, execution.to_owned());
            }
        }

        let mut jobs = DashboardJobs {
            queued: 0,
            running: 0,
            waiting_user: 0,
            waiting_review: 0,
            paused: 0,
            needs_attention: 0,
            reviews: Vec::new(),
        };
        for run in runs {
            if run.kind == "review" {
                if ACTIVE_RUN_STATUSES.contains(&run.status.as_str()) || run.status == "failed" {
                    jobs.reviews.push(dashboard_review_job(run)?);
                }
                continue;
            }
            if run.parent_run_id.is_some() {
                continue;
            }
            match run.status.as_str() {
                "queued" => jobs.queued += 1,
                "running" => jobs.running += 1,
                "paused" => jobs.paused += 1,
                "interrupted" | "failed" => jobs.needs_attention += 1,
                "waiting" => {
                    let execution = execution_by_version
                        .get(&run.version_id.0)
                        .map_or("waiting_review", String::as_str);
                    if execution == "waiting_answer" {
                        jobs.waiting_user += 1;
                    } else {
                        jobs.waiting_review += 1;
                    }
                }
                _ => {}
            }
        }
        jobs.reviews.sort_by_key(|review| review.run_id);
        Ok(jobs)
    }

    /// 由正文修订链回算净字数增量与当前可见正文总量。
    ///
    /// 只取统计需要的列：修订正文与结构化载荷不参与字数统计，把它们读回来会把整部
    /// 作品的文本搬进内存（实测 6,000 条修订即可让常驻内存多出上百 MB）。可见性也
    /// 交给 SQL，已删除文档与非正文文档的修订不进入结果集。
    async fn dashboard_words(
        &self,
        range: &DashboardRange,
        project_id: Option<Uuid>,
    ) -> Result<DashboardWords, StoreError> {
        let versions = novel_version::Entity::find().all(&self.database).await?;
        let project_by_version: HashMap<Uuid, Uuid> = versions
            .iter()
            .map(|version| (version.id.0, version.project_id.0))
            .collect();
        let documents = document::Entity::find().all(&self.database).await?;
        let visible: HashMap<Uuid, &document::Model> = documents
            .iter()
            .filter(|document| document.deleted_at.is_none())
            .filter(|document| document.role.as_str() == CHAPTER_BODY_ROLE)
            .filter(|document| {
                project_id.is_none_or(|project_id| {
                    project_by_version.get(&document.version_id.0).copied() == Some(project_id)
                })
            })
            .map(|document| (document.id.0, document))
            .collect();

        let visible_ids: Vec<TextUuid> = visible.keys().map(|id| TextUuid::from(*id)).collect();
        let revisions: Vec<RevisionStat> = document_revision::Entity::find()
            .filter(document_revision::Column::DocumentId.is_in(visible_ids))
            .into_partial_model::<RevisionStat>()
            .all(&self.database)
            .await?;
        let word_count_by_revision: HashMap<Uuid, i64> = revisions
            .iter()
            .map(|revision| (revision.id.0, revision.word_count))
            .collect();

        let mut current_total = 0;
        for document in visible.values() {
            let Some(current) = document.current_revision_id.as_deref() else {
                continue;
            };
            current_total += parse_id(current, "修订 ID")
                .ok()
                .and_then(|revision_id| word_count_by_revision.get(&revision_id).copied())
                .unwrap_or(0);
        }

        let days = range.day_indexes();
        let day_slot: HashMap<i64, usize> = days
            .iter()
            .enumerate()
            .map(|(index, day)| (*day, index))
            .collect();
        let mut daily: Vec<DashboardDailyWords> = days
            .iter()
            .map(|day| DashboardDailyWords {
                day_start_ms: range.day_start_ms(*day),
                net_increment: 0,
                revisions: 0,
            })
            .collect();

        // 同一文档的修订按修订号排序，才能得到相邻两次的真实差值。
        let mut per_document: HashMap<Uuid, Vec<&RevisionStat>> = HashMap::new();
        for revision in &revisions {
            per_document
                .entry(revision.document_id.0)
                .or_default()
                .push(revision);
        }
        let mut net_increment = 0;
        for revisions in per_document.values_mut() {
            revisions.sort_by_key(|revision| revision.revision_no);
            let mut previous = 0;
            for revision in revisions.iter() {
                // 复制稿本带来的是内容库存转移，不是一次创作事件。
                let delta = if revision.created_by == "copied" {
                    0
                } else {
                    revision.word_count - previous
                };
                previous = revision.word_count;
                if revision.created_at < range.start_ms || revision.created_at >= range.end_ms {
                    continue;
                }
                net_increment += delta;
                if let Some(index) = day_slot.get(&range.day_index(revision.created_at)) {
                    daily[*index].net_increment += delta;
                    daily[*index].revisions += 1;
                }
            }
        }

        Ok(DashboardWords {
            net_increment,
            current_total,
            daily,
        })
    }
}

/// 字数统计需要的修订列：不含正文与结构化载荷。
#[derive(DerivePartialModel)]
#[sea_orm(entity = "document_revision::Entity")]
struct RevisionStat {
    id: TextUuid,
    document_id: TextUuid,
    revision_no: i64,
    word_count: i64,
    created_by: String,
    created_at: i64,
}

fn dashboard_review_job(run: &run::Model) -> Result<DashboardReviewJob, StoreError> {
    Ok(DashboardReviewJob {
        run_id: run.id.0,
        parent_run_id: run
            .parent_run_id
            .as_deref()
            .map(|value| parse_id(value, "父 Run ID"))
            .transpose()?,
        project_id: run.project_id.0,
        version_id: run.version_id.0,
        stage: run.stage.clone(),
        status: run.status.clone(),
    })
}

/// 读取一条请求的用量口径。
///
/// 已封存的记录自带 `usage_source` / `usage_status`；没有记录的请求按它的终态
/// 解释，因此「用量未知」永远带一个具体原因，而不是无法解释的空缺。
fn persisted_usage(
    usage_json: Option<&str>,
    status: &str,
) -> Result<PersistedModelUsage, StoreError> {
    if let Some(usage_json) = usage_json {
        return Ok(serde_json::from_str(usage_json)
            .unwrap_or_else(|_| PersistedModelUsage::unknown(UsageUnknownReason::InvalidReport)));
    }
    let reason = match model_request_status_from_db(status)? {
        ModelRequestStatus::Prepared | ModelRequestStatus::Dispatched => {
            UsageUnknownReason::Pending
        }
        ModelRequestStatus::Succeeded => UsageUnknownReason::ProviderSilent,
        ModelRequestStatus::Cancelled => UsageUnknownReason::RequestCancelled,
        ModelRequestStatus::Failed | ModelRequestStatus::Unknown => {
            UsageUnknownReason::RequestFailed
        }
    };
    Ok(PersistedModelUsage::unknown(reason))
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{NewProject, non_whitespace_char_count};
    use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};
    use tempfile::TempDir;

    use super::*;

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    fn word_count(content: &str) -> i64 {
        non_whitespace_char_count(content) as i64
    }

    async fn insert_run(
        store: &Store,
        project_id: Uuid,
        version_id: Uuid,
        run_id: Uuid,
        parent_run_id: Option<Uuid>,
        kind: &str,
        status: &str,
        now: i64,
    ) {
        run::ActiveModel {
            id: Set(db_id(run_id)),
            project_id: Set(db_id(project_id)),
            version_id: Set(db_id(version_id)),
            kind: Set(kind.to_owned()),
            parent_run_id: Set(parent_run_id.map(|id| id.to_string())),
            stage: Set("test".to_owned()),
            status: Set(status.to_owned()),
            resume_count: Set(0),
            request_count: Set(0),
            request_allowance_end: Set(0),
            consecutive_protocol_errors: Set(0),
            checkpoint_json: Set("{}".to_owned()),
            quota_scope_id: Set(None),
            started_at: Set(now),
            ended_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&store.database)
        .await
        .unwrap();
    }

    /// 写入一条已封存的模型请求；用量用真实口径序列化，使测试不会撒谎。
    async fn insert_request(
        store: &Store,
        run_id: Uuid,
        attempt_no: i64,
        status: &str,
        usage: Option<PersistedModelUsage>,
        now: i64,
    ) {
        model_request::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            run_id: Set(db_id(run_id)),
            attempt_no: Set(attempt_no),
            status: Set(status.to_owned()),
            settings_snapshot_json: Set("{}".to_owned()),
            prompt_snapshot_json: Set("{}".to_owned()),
            context_snapshot_json: Set("{}".to_owned()),
            provider_request_id: Set(None),
            usage_json: Set(usage.map(|usage| serde_json::to_string(&usage).unwrap())),
            error_json: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            dispatched_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            ..Default::default()
        }
        .insert(&store.database)
        .await
        .unwrap();
    }

    async fn insert_body_document(store: &Store, version_id: Uuid, now: i64) -> Uuid {
        let document_id = Uuid::new_v4();
        document::ActiveModel {
            id: Set(db_id(document_id)),
            version_id: Set(db_id(version_id)),
            chapter_id: Set(None),
            role: Set(CHAPTER_BODY_ROLE.to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(&store.database)
        .await
        .unwrap();
        document_id
    }

    async fn append_body_revision(
        store: &Store,
        document_id: Uuid,
        revision_no: i64,
        content: &str,
        created_by: &str,
        created_at: i64,
    ) {
        let revision_id = Uuid::new_v4();
        document_revision::ActiveModel {
            id: Set(db_id(revision_id)),
            document_id: Set(db_id(document_id)),
            revision_no: Set(revision_no),
            content: Set(content.to_owned()),
            structured_payload: Set(None),
            word_count: Set(word_count(content)),
            created_by: Set(created_by.to_owned()),
            source_revision_id: Set(None),
            created_at: Set(created_at),
        }
        .insert(&store.database)
        .await
        .unwrap();
        let document = document::Entity::find_by_id(db_id(document_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        let mut active = document.into_active_model();
        active.current_revision_id = Set(Some(revision_id.to_string()));
        active.revision = Set(revision_no);
        active.update(&store.database).await.unwrap();
    }

    /// 没有封存记录的请求由终态解释，因此「用量未知」永远带一个具体原因。
    #[test]
    fn missing_usage_records_are_explained_by_the_request_status() {
        assert_eq!(
            persisted_usage(None, "succeeded").unwrap().reason,
            Some(UsageUnknownReason::ProviderSilent)
        );
        assert_eq!(
            persisted_usage(None, "failed").unwrap().reason,
            Some(UsageUnknownReason::RequestFailed)
        );
        assert_eq!(
            persisted_usage(None, "cancelled").unwrap().reason,
            Some(UsageUnknownReason::RequestCancelled)
        );
        assert_eq!(
            persisted_usage(None, "dispatched").unwrap().reason,
            Some(UsageUnknownReason::Pending)
        );
        // 损坏的记录不会让整份统计失败，而是明确标为无法解析。
        let invalid = persisted_usage(Some("{"), "succeeded").unwrap();
        assert!(!invalid.is_reported());
        assert_eq!(invalid.reason, Some(UsageUnknownReason::InvalidReport));
        assert!(persisted_usage(None, "not-a-status").is_err());
    }

    /// 空库也必须给出完整的零值结构，而不是缺字段。
    #[tokio::test]
    async fn empty_store_returns_zeroed_sections() {
        let (_data_dir, store) = open().await;
        let range = DashboardRange::new(utc_now_millis(), 480, 7).unwrap();
        let dashboard = store.compute_dashboard(range.clone(), None).await.unwrap();
        assert_eq!(dashboard.range, range);
        assert_eq!(dashboard.tokens.request_count, 0);
        assert_eq!(
            dashboard.tokens.unknown_usage,
            [] as [nekowriter_core::DashboardUnknownUsage; 0]
        );
        assert_eq!(dashboard.tokens.system_operations.request_count, 0);
        assert_eq!(
            dashboard.tokens.system_operations.recent,
            [] as [nekowriter_core::DashboardSystemOperation; 0]
        );
        assert_eq!(dashboard.tokens.by_role.len(), 0);
        assert_eq!(dashboard.tokens.daily.len(), 7);
        assert_eq!(dashboard.jobs.queued, 0);
        assert_eq!(
            dashboard.jobs.reviews,
            [] as [nekowriter_core::DashboardReviewJob; 0]
        );
        assert_eq!(dashboard.words.current_total, 0);
        assert_eq!(dashboard.words.net_increment, 0);
        assert_eq!(dashboard.words.daily.len(), 7);
    }

    /// Token 与字数都必须从已有记录回算，且同一批数据重复查询结果一致。
    #[tokio::test]
    async fn dashboard_recomputes_tokens_and_word_deltas_from_records() {
        let (_data_dir, store) = open().await;
        let bundle = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("统计项目", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = bundle.version.id;
        let now = utc_now_millis();

        let run_id = Uuid::new_v4();
        insert_run(
            &store,
            bundle.project.id,
            version_id,
            run_id,
            None,
            "chapter_body",
            "succeeded",
            now,
        )
        .await;
        insert_request(
            &store,
            run_id,
            1,
            "succeeded",
            Some(PersistedModelUsage::from_provider_stream(
                10,
                4,
                14,
                Some(2),
                Some(1),
                None,
            )),
            now,
        )
        .await;
        insert_request(&store, run_id, 2, "failed", None, now).await;

        // 正文：一条普通修订链，加上另一个稿本里复制进来的内容。
        let document_id = insert_body_document(&store, version_id, now).await;
        append_body_revision(&store, document_id, 1, "第一段正文", "agent", now).await;
        append_body_revision(&store, document_id, 2, "第一段正文加长很多", "agent", now).await;
        let copied = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("复制来源", "旧报纸", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let copied_document = insert_body_document(&store, copied.version.id, now).await;
        append_body_revision(
            &store,
            copied_document,
            1,
            "来自另一稿本的既有正文内容",
            "copied",
            now,
        )
        .await;

        let range = DashboardRange::new(now, 480, 7).unwrap();
        let first = store.compute_dashboard(range.clone(), None).await.unwrap();
        assert_eq!(first.tokens.request_count, 2);
        assert_eq!(first.tokens.input_tokens, 10);
        assert_eq!(first.tokens.output_tokens, 4);
        assert_eq!(first.tokens.cached_input_tokens, 2);
        assert_eq!(first.tokens.reasoning_tokens, 1);
        assert_eq!(first.tokens.unknown_usage_requests, 1);
        // 没有封存用量的请求由终态解释，因此这个数字能自己说明原因。
        assert_eq!(first.tokens.unknown_usage.len(), 1);
        assert_eq!(
            first.tokens.unknown_usage[0].reason,
            UsageUnknownReason::RequestFailed
        );
        assert_eq!(first.tokens.unknown_usage[0].requests, 1);
        assert_eq!(first.tokens.by_role.len(), 1);
        assert_eq!(first.tokens.by_role[0].role, "writing");
        assert_eq!(
            first
                .tokens
                .daily
                .iter()
                .map(|day| day.request_count)
                .sum::<i64>(),
            2
        );

        // 普通修订链的净增是相邻差值之和；复制内容只增加库存，不增加创作量。
        assert_eq!(first.words.net_increment, 9);
        assert_eq!(
            first.words.current_total,
            word_count("第一段正文加长很多") + word_count("来自另一稿本的既有正文内容")
        );
        assert_eq!(
            first
                .words
                .daily
                .iter()
                .map(|day| day.net_increment)
                .sum::<i64>(),
            9
        );

        // 重复查询同一条记录必须完全一致。
        let second = store.compute_dashboard(range, None).await.unwrap();
        assert_eq!(second.tokens, first.tokens);
        assert_eq!(second.words, first.words);
        assert_eq!(second.jobs, first.jobs);
    }

    /// 顶层 Run 按状态分区，等待用户与等待审核按工作流执行轴区分，审核子 Run 展开。
    #[tokio::test]
    async fn dashboard_jobs_split_waiting_and_expand_review_runs() {
        let (_data_dir, store) = open().await;
        let bundle = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("任务统计", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = bundle.version.id;
        let now = utc_now_millis();

        // 项目创建时已经建立了工作流实例，这里只把它置为“等待用户回答”。
        let workflow = workflow_instance::Entity::find_by_id(db_id(version_id))
            .one(&store.database)
            .await
            .unwrap()
            .expect("项目创建必须建立工作流实例");
        let mut active = workflow.into_active_model();
        active.state_json = Set(serde_json::json!({
            "execution": "waiting_answer",
            "stage": "preparing_reviewing",
            "approvalMode": "auto_approve",
        })
        .to_string());
        active.update(&store.database).await.unwrap();

        let parent_id = Uuid::new_v4();
        insert_run(
            &store,
            bundle.project.id,
            version_id,
            parent_id,
            None,
            "preparation",
            "waiting",
            now,
        )
        .await;
        insert_run(
            &store,
            bundle.project.id,
            version_id,
            Uuid::new_v4(),
            Some(parent_id),
            "review",
            "running",
            now,
        )
        .await;
        insert_run(
            &store,
            bundle.project.id,
            version_id,
            Uuid::new_v4(),
            None,
            "chapter_outline",
            "failed",
            now,
        )
        .await;

        let range = DashboardRange::new(now, 0, 7).unwrap();
        let dashboard = store.compute_dashboard(range, None).await.unwrap();
        assert_eq!(dashboard.jobs.waiting_user, 1);
        assert_eq!(dashboard.jobs.waiting_review, 0);
        assert_eq!(dashboard.jobs.needs_attention, 1);
        assert_eq!(dashboard.jobs.queued, 0);
        // 审核子 Run 展开显示，不进入主计数。
        assert_eq!(dashboard.jobs.reviews.len(), 1);
        assert_eq!(dashboard.jobs.reviews[0].parent_run_id, Some(parent_id));
    }

    /// 项目筛选只汇总该项目的请求与正文。
    #[tokio::test]
    async fn dashboard_project_filter_excludes_other_projects() {
        let (_data_dir, store) = open().await;
        let first = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("项目一", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let second = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("项目二", "雾中的信", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let now = utc_now_millis();
        let run_id = Uuid::new_v4();
        insert_run(
            &store,
            second.project.id,
            second.version.id,
            run_id,
            None,
            "chapter_body",
            "succeeded",
            now,
        )
        .await;
        insert_request(
            &store,
            run_id,
            1,
            "succeeded",
            Some(PersistedModelUsage::from_provider_stream(
                7, 1, 8, None, None, None,
            )),
            now,
        )
        .await;
        let document_id = insert_body_document(&store, second.version.id, now).await;
        append_body_revision(&store, document_id, 1, "另一个项目的正文", "agent", now).await;

        let range = DashboardRange::new(now, 0, 7).unwrap();
        let mine = store
            .compute_dashboard(range.clone(), Some(first.project.id))
            .await
            .unwrap();
        assert_eq!(mine.tokens.request_count, 0);
        assert_eq!(mine.words.current_total, 0);

        let theirs = store
            .compute_dashboard(range, Some(second.project.id))
            .await
            .unwrap();
        assert_eq!(theirs.tokens.input_tokens, 7);
        assert_eq!(theirs.words.current_total, word_count("另一个项目的正文"));
    }
}
