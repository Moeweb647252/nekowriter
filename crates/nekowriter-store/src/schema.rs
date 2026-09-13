//! SQLite 连接与首次 schema 初始化。
//!
//! 表、列、普通唯一键和外键由 `SeaORM` Entity 注册表生成。SQLite 特有的 partial
//! index 无法由实体属性完整表达，因此在同一初始化事务中以 `SeaQuery` 的类型化
//! builder 补充；这里不保存任何手写的表或索引 DDL。

use std::path::Path;

use sea_orm::sea_query::{ConditionalStatement, Expr, ExprTrait, Index, IndexOrder};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait, Set,
    Statement, TransactionTrait,
};
use url::Url;

use crate::{
    SCHEMA_VERSION, StoreError,
    entity::{
        application_metadata, chapter, document, document_revision, embedding_setting, event,
        feedback, knowledge_entry, knowledge_index_task, knowledge_space, model_request,
        novel_version, project, prompt_override, prompt_template, question, quota_charge,
        quota_scope, review, run, tool_call,
    },
    transaction_error,
};

pub(crate) async fn open_database(data_dir: &Path) -> Result<DatabaseConnection, StoreError> {
    let database_path = data_dir.join("nekowriter.sqlite3");
    let database_existed = database_path.exists();
    let database = Database::connect(sqlite_url(&database_path)?).await?;
    configure_sqlite(&database).await?;

    if database_existed {
        verify_schema_version(&database).await?;
    } else {
        initialize_schema(&database).await?;
    }

    Ok(database)
}

fn sqlite_url(path: &Path) -> Result<String, StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::CorruptRecord("数据库路径缺少父目录".to_owned()))?;
    let absolute_path = parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| StoreError::CorruptRecord("数据库路径缺少文件名".to_owned()))?,
    );
    let file_url = Url::from_file_path(absolute_path)
        .map_err(|()| StoreError::CorruptRecord("无法构造 SQLite 文件 URL".to_owned()))?;
    let encoded_path = file_url
        .as_str()
        .strip_prefix("file://")
        .ok_or_else(|| StoreError::CorruptRecord("无法解析 SQLite 文件 URL".to_owned()))?
        .to_owned();

    #[cfg(windows)]
    let encoded_path = encoded_path
        .strip_prefix('/')
        .unwrap_or(&encoded_path)
        .to_owned();

    Ok(format!("sqlite://{encoded_path}?mode=rwc"))
}

async fn configure_sqlite(database: &DatabaseConnection) -> Result<(), StoreError> {
    for pragma in [
        "PRAGMA foreign_keys = ON",
        "PRAGMA journal_mode = WAL",
        "PRAGMA synchronous = FULL",
        "PRAGMA busy_timeout = 5000",
    ] {
        database
            .execute_raw(Statement::from_string(DbBackend::Sqlite, pragma.to_owned()))
            .await?;
    }
    Ok(())
}

async fn verify_schema_version(database: &DatabaseConnection) -> Result<(), StoreError> {
    let metadata = application_metadata::Entity::find_by_id("schema_version".to_owned())
        .one(database)
        .await
        .map_err(|error| {
            if error
                .to_string()
                .contains("no such table: application_metadata")
            {
                StoreError::UnknownSchema
            } else {
                StoreError::Database(error)
            }
        })?;
    let found = metadata.ok_or(StoreError::UnknownSchema)?.value;
    if found != SCHEMA_VERSION.to_string() {
        return Err(StoreError::UnsupportedSchema {
            found,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(())
}

async fn initialize_schema(database: &DatabaseConnection) -> Result<(), StoreError> {
    let schema = database.get_schema_registry("nekowriter_store::entity::*");
    database
        .transaction::<_, _, StoreError>(|transaction| {
            Box::pin(async move {
                schema.apply(transaction).await?;
                create_sqlite_indexes(transaction).await?;
                application_metadata::ActiveModel {
                    key: Set("schema_version".to_owned()),
                    value: Set(SCHEMA_VERSION.to_string()),
                }
                .insert(transaction)
                .await?;
                Ok(())
            })
        })
        .await
        .map_err(transaction_error)
}

async fn create_sqlite_indexes<C: ConnectionTrait>(connection: &C) -> Result<(), StoreError> {
    let indexes = [
        Index::create()
            .name("idx_projects_active_updated")
            .table(project::Entity)
            .col((project::Column::UpdatedAt, IndexOrder::Desc))
            .col(project::Column::Id)
            .and_where(Expr::col((project::Entity, project::Column::DeletedAt)).is_null())
            .to_owned(),
        Index::create()
            .name("idx_versions_active_project")
            .table(novel_version::Entity)
            .col(novel_version::Column::ProjectId)
            .col(novel_version::Column::CreatedAt)
            .col(novel_version::Column::Id)
            .and_where(
                Expr::col((novel_version::Entity, novel_version::Column::DeletedAt)).is_null(),
            )
            .to_owned(),
        Index::create()
            .name("idx_chapters_active_order")
            .table(chapter::Entity)
            .col(chapter::Column::VersionId)
            .col(chapter::Column::Kind)
            .col(chapter::Column::OrderIndex)
            .unique()
            .and_where(Expr::col((chapter::Entity, chapter::Column::DeletedAt)).is_null())
            .to_owned(),
        Index::create()
            .name("idx_chapters_active_plan_item")
            .table(chapter::Entity)
            .col(chapter::Column::VersionId)
            .col(chapter::Column::PlanItemId)
            .unique()
            .and_where(Expr::col((chapter::Entity, chapter::Column::DeletedAt)).is_null())
            .and_where(Expr::col((chapter::Entity, chapter::Column::PlanItemId)).is_not_null())
            .to_owned(),
        Index::create()
            .name("idx_documents_active_version_role")
            .table(document::Entity)
            .col(document::Column::VersionId)
            .col(document::Column::Role)
            .unique()
            .and_where(Expr::col((document::Entity, document::Column::ChapterId)).is_null())
            .and_where(Expr::col((document::Entity, document::Column::DeletedAt)).is_null())
            .to_owned(),
        Index::create()
            .name("idx_documents_active_chapter_role")
            .table(document::Entity)
            .col(document::Column::ChapterId)
            .col(document::Column::Role)
            .unique()
            .and_where(Expr::col((document::Entity, document::Column::ChapterId)).is_not_null())
            .and_where(Expr::col((document::Entity, document::Column::DeletedAt)).is_null())
            .to_owned(),
        Index::create()
            .name("idx_document_revisions_document_no")
            .table(document_revision::Entity)
            .col(document_revision::Column::DocumentId)
            .col((document_revision::Column::RevisionNo, IndexOrder::Desc))
            .to_owned(),
        Index::create()
            .name("uq_prompt_overrides_system_key")
            .table(prompt_override::Entity)
            .col(prompt_override::Column::Scope)
            .col(prompt_override::Column::PromptKey)
            .unique()
            .and_where(
                Expr::col((prompt_override::Entity, prompt_override::Column::ProjectId)).is_null(),
            )
            .to_owned(),
        Index::create()
            .name("uq_prompt_overrides_project_key")
            .table(prompt_override::Entity)
            .col(prompt_override::Column::ProjectId)
            .col(prompt_override::Column::PromptKey)
            .unique()
            .and_where(
                Expr::col((prompt_override::Entity, prompt_override::Column::ProjectId))
                    .is_not_null(),
            )
            .to_owned(),
        Index::create()
            .name("idx_runs_active_version")
            .table(run::Entity)
            .col(run::Column::VersionId)
            .col(run::Column::Status)
            .col(run::Column::CreatedAt)
            .to_owned(),
        Index::create()
            .name("idx_model_requests_run_status")
            .table(model_request::Entity)
            .col(model_request::Column::RunId)
            .col(model_request::Column::Status)
            .col((model_request::Column::AttemptNo, IndexOrder::Desc))
            .to_owned(),
        Index::create()
            .name("uq_model_requests_in_flight_run")
            .table(model_request::Entity)
            .col(model_request::Column::RunId)
            .unique()
            .and_where(
                Expr::col((model_request::Entity, model_request::Column::Status))
                    .is_in(["prepared", "dispatched"]),
            )
            .to_owned(),
        Index::create()
            .name("idx_tool_calls_run_sequence")
            .table(tool_call::Entity)
            .col(tool_call::Column::RunId)
            .col(tool_call::Column::Sequence)
            .to_owned(),
        Index::create()
            .name("uq_tool_calls_awaiting_review_run")
            .table(tool_call::Entity)
            .col(tool_call::Column::RunId)
            .unique()
            .and_where(
                Expr::col((tool_call::Entity, tool_call::Column::Status)).eq("awaiting_review"),
            )
            .to_owned(),
        Index::create()
            .name("uq_tool_calls_held_run")
            .table(tool_call::Entity)
            .col(tool_call::Column::RunId)
            .unique()
            .and_where(Expr::col((tool_call::Entity, tool_call::Column::Status)).eq("held"))
            .to_owned(),
        Index::create()
            .name("uq_tool_calls_awaiting_answer_run")
            .table(tool_call::Entity)
            .col(tool_call::Column::RunId)
            .unique()
            .and_where(
                Expr::col((tool_call::Entity, tool_call::Column::Status)).eq("awaiting_answer"),
            )
            .to_owned(),
        Index::create()
            .name("idx_questions_open")
            .table(question::Entity)
            .col(question::Column::Status)
            .col(question::Column::CreatedAt)
            .and_where(Expr::col((question::Entity, question::Column::Status)).eq("open"))
            .to_owned(),
        Index::create()
            .name("idx_feedback_pending_run_sequence")
            .table(feedback::Entity)
            .col(feedback::Column::RunId)
            .col(feedback::Column::Status)
            .col(feedback::Column::Sequence)
            .and_where(
                Expr::col((feedback::Entity, feedback::Column::Status)).is_in(["queued", "bound"]),
            )
            .to_owned(),
        Index::create()
            .name("uq_runs_active_top_level_project")
            .table(run::Entity)
            .col(run::Column::ProjectId)
            .unique()
            .and_where(Expr::col((run::Entity, run::Column::ParentRunId)).is_null())
            .and_where(Expr::col((run::Entity, run::Column::Kind)).is_in([
                "preparation",
                "chapter_outline",
                "chapter_body",
            ]))
            .and_where(Expr::col((run::Entity, run::Column::Status)).is_in([
                "queued",
                "running",
                "waiting",
                "paused",
                "interrupted",
            ]))
            .to_owned(),
        Index::create()
            .name("uq_runs_active_top_level_version")
            .table(run::Entity)
            .col(run::Column::VersionId)
            .unique()
            .and_where(Expr::col((run::Entity, run::Column::ParentRunId)).is_null())
            .and_where(Expr::col((run::Entity, run::Column::Kind)).is_in([
                "preparation",
                "chapter_outline",
                "chapter_body",
            ]))
            .and_where(Expr::col((run::Entity, run::Column::Status)).is_in([
                "queued",
                "running",
                "waiting",
                "paused",
                "interrupted",
            ]))
            .to_owned(),
        Index::create()
            .name("idx_reviews_open_target")
            .table(review::Entity)
            .col(review::Column::VersionId)
            .col(review::Column::TargetRevisionId)
            .and_where(Expr::col((review::Entity, review::Column::Status)).eq("open"))
            .to_owned(),
        Index::create()
            .name("idx_events_version_seq")
            .table(event::Entity)
            .col(event::Column::VersionId)
            .col(event::Column::Seq)
            .to_owned(),
        Index::create()
            .name("uq_embedding_settings_key")
            .table(embedding_setting::Entity)
            .col(embedding_setting::Column::Key)
            .unique()
            .to_owned(),
        Index::create()
            .name("idx_knowledge_spaces_active")
            .table(knowledge_space::Entity)
            .col(knowledge_space::Column::DeletedAt)
            .col(knowledge_space::Column::UpdatedAt)
            .to_owned(),
        Index::create()
            .name("idx_knowledge_entries_active_space")
            .table(knowledge_entry::Entity)
            .col(knowledge_entry::Column::SpaceId)
            .col(knowledge_entry::Column::UpdatedAt)
            .and_where(
                Expr::col((knowledge_entry::Entity, knowledge_entry::Column::DeletedAt)).is_null(),
            )
            .to_owned(),
        Index::create()
            .name("uq_knowledge_index_tasks_target")
            .table(knowledge_index_task::Entity)
            .col(knowledge_index_task::Column::EntryId)
            .col(knowledge_index_task::Column::EntryRevision)
            .col(knowledge_index_task::Column::EmbeddingProfileId)
            .unique()
            .to_owned(),
        Index::create()
            .name("uq_quota_scopes_version_stage")
            .table(quota_scope::Entity)
            .col(quota_scope::Column::VersionId)
            .col(quota_scope::Column::StageKey)
            .unique()
            .to_owned(),
        Index::create()
            .name("uq_quota_charges_scope_tool_call")
            .table(quota_charge::Entity)
            .col(quota_charge::Column::ScopeId)
            .col(quota_charge::Column::ToolCallId)
            .unique()
            .to_owned(),
        Index::create()
            .name("idx_prompt_templates_active")
            .table(prompt_template::Entity)
            .col(prompt_template::Column::DeletedAt)
            .col(prompt_template::Column::UpdatedAt)
            .to_owned(),
        Index::create()
            .name("idx_knowledge_index_tasks_pending")
            .table(knowledge_index_task::Entity)
            .col(knowledge_index_task::Column::Status)
            .col(knowledge_index_task::Column::CreatedAt)
            .and_where(
                Expr::col((
                    knowledge_index_task::Entity,
                    knowledge_index_task::Column::Status,
                ))
                .eq("pending"),
            )
            .to_owned(),
    ];

    for index in indexes {
        connection.execute(&index).await?;
    }
    Ok(())
}
