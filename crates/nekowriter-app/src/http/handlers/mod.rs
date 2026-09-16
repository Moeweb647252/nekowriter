//! 各域 HTTP handler：一个 API 域一个子模块，划分见 `docs/08` 的「Phase 1 的 handler 模块边界」。
//!
//! 这些 handler 是按域从 `app/src/main.rs` 搬过来的（`docs/08` 的分批计划），每一批只移动、
//! 不改行为：`http::router` 用一条 `use crate::http::handlers::<域>::{...}` 把名字接进来，所以
//! 路由表本身一行都不改。
//!
//! 已经搬过来的：P1.6 的系统与提示词面（`system`、`prompts`）。
//! P1.7 的项目/版本/导出/项目设置（`projects`）。
//!
//! 跨域共用的 handler 入口（`ensure_model_ready`、`enqueue_queued_writing_run`）在本文件里，
//! 它们被章节域与准备域的 handler 共用，放在任一域会形成反向依赖。
//! P1.8 的章节域（`chapters`）与准备域（`preparation`）。
//! P1.9 的事件面（`events`）。
//!
//! 两个跨域 helper 就在本文件里（P1.8）：`ensure_model_ready` 与 `enqueue_queued_writing_run`
//! 同时被章节域与准备域调用，放在任一域文件里都会让另一域反向依赖；它们只依赖
//! `AppState`、`nekowriter_core` 与 `nekowriter_store`，所以放这里不需要额外接线。

use nekowriter_core::{ExecutionStatus, ProjectId, WorkflowSnapshot, WorkflowStage};
use nekowriter_store::StoreError;

use crate::http::error::ApiError;
use crate::http::state::AppState;
use crate::model_settings::{
    capability_problem_message, pre_dispatch_capability_problem, resolve_effective_model,
};

pub(crate) mod chapters;
pub(crate) mod events;
pub(crate) mod preparation;
pub(crate) mod projects;
pub(crate) mod prompts;
pub(crate) mod system;

/// 命令前置检查：按项目的**有效模型**判断能力，而不是系统模型。
///
/// 项目可以引用另一个模型，那个模型（而非系统写作模型）才是请求真正发往的对象；
/// 用系统模型的资料放行会让项目引用在真正 dispatch 时才以指向系统模型的文案失败。
/// 这里与调度器共用 [`pre_dispatch_capability_problem`]，不维护第二套规则。
pub(crate) async fn ensure_model_ready(
    state: &AppState,
    project_id: ProjectId,
) -> Result<(), ApiError> {
    let system = state
        .store
        .get_system_model_settings()
        .await?
        .ok_or_else(|| StoreError::InvalidSystemModelSettings("请先保存写作模型设置".to_owned()))?;
    let project = state.store.get_project_settings(project_id).await?;
    let effective = resolve_effective_model(&system, &project);
    if let Some(problem) =
        pre_dispatch_capability_problem(*effective.capabilities, project.thinking_level)
    {
        return Err(
            StoreError::InvalidSystemModelSettings(capability_problem_message(
                problem,
                &effective.describe(),
            ))
            .into(),
        );
    }
    Ok(())
}

/// 审核拒绝或章节细纲通过后，Store 会在同一事务中把原 Run（或新正文 Run）恢复
/// 为 queued。HTTP 命令返回前把它交还给每稿本调度队列，避免工作流停在没有消费者
/// 的队列状态；调度器仍会在实际发送前重新读取最新模型设置和上下文。
pub(crate) async fn enqueue_queued_writing_run(
    state: &AppState,
    workflow: &WorkflowSnapshot,
) -> Result<(), ApiError> {
    if workflow.execution != ExecutionStatus::Queued {
        return Ok(());
    }
    let run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::InvalidPreparationState("排队工作流缺少活跃 Run".to_owned()))?;
    match workflow.stage {
        WorkflowStage::PreparingRevising | WorkflowStage::PreparingGenerating => {
            state
                .scheduler
                .enqueue_preparation(workflow.version_id, run_id)
                .await?;
        }
        WorkflowStage::WritingOutlineGenerating => {
            let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                StoreError::InvalidChapterState("排队细纲任务缺少当前章节".to_owned())
            })?;
            state
                .scheduler
                .enqueue_chapter_outline(workflow.version_id, chapter_id, run_id)
                .await?;
        }
        WorkflowStage::WritingDrafting => {
            let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                StoreError::InvalidChapterState("排队正文任务缺少当前章节".to_owned())
            })?;
            state
                .scheduler
                .enqueue_chapter_body(workflow.version_id, chapter_id, run_id)
                .await?;
        }
        _ => {}
    }
    Ok(())
}
