//! SeaORM 实体定义。
//!
//! `SeaORM` 实体定义，也是新建本地数据库的 schema 权威。实体注册表负责创建表、
//! 唯一键和外键；仓库约定不引入迁移，已有未知 schema 会被拒绝打开。非空 UUID
//! 主键/外键使用 `TextUuid`，以保持 `SQLite` 中可读、可直接检查的 `TEXT` UUID
//! 表示；允许 SQL `NULL` 的外键则使用 `Option<String>`，并在仓储边界解析为 UUID。
//! 这避开 `SeaORM` 2.0.2 `SQLite` 对 `Option<TextUuid>` 的 NULL 解码缺陷，不让该
//! 细节泄漏到领域层。

pub mod agent_message;
pub mod application_metadata;
pub mod chapter;
pub mod command_receipt;
pub mod document;
pub mod document_revision;
pub mod event;
pub mod feedback;
pub mod model_request;
pub mod novel_version;
pub mod project;
pub mod prompt_override;
pub mod question;
pub mod review;
pub mod revision_dependency;
pub mod run;
pub mod system_model_setting;
pub mod tool_call;
pub mod workflow_instance;
