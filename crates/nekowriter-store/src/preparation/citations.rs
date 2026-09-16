//! 引用校验、正文自由意见的锚点与未封存批次。
//!
//! 从 `preparation/mod.rs` 的引用/意见层原样搬出（`docs/08` 的 P3.3），只移动：函数体逐字不变，
//! 只改了可见性前缀（被块外引用的项改 `pub(crate)`）。`mod.rs` 用
//! `pub(crate) use citations::{…}` 把这些名字接回，所以 mod.rs 自己与兄弟模块的既有引用
//! （`super::preparation::…`）一行都不用改。

use nekowriter_core::{
    ChapterId, ExecutionStatus, FeedbackCitation, FeedbackStatus, HumanReviewDecision, RevisionId,
    RunId,
};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use crate::entity::{document, document_revision, feedback, run, tool_call};
use crate::{StoreError, VersionId, db_id, parse_optional_id};
// 只被 `queue_chapter_body_feedback` 的文档链接 [`Store::submit_run_feedback`] 使用。
#[allow(unused_imports)]
use crate::Store;

use super::{
    candidate_content_hash, held_chapter_body_tool_payload_from_model,
    pending_chapter_body_proposal_from_tool_call,
};

/// 正文自由意见只能引用当前稿本中的正式文档，或当前 Run 尚未落库的候选正文。
///
/// 前端提交的字符位置始终被当作 Unicode scalar value 序号处理；这里先把它们映射
/// 回服务端快照中的 UTF-8 边界，再逐字校验 quote。这样 emoji、组合字符和多字节
/// 中文都不会因为把 UTF-16/UTF-8 偏移混用而引用到错误位置。
#[derive(Clone, Copy)]
pub(crate) enum CandidateCitationScope {
    HeldCurrentRun,
    Proposal { tool_call_id: Uuid },
}

pub(crate) async fn validate_chapter_body_feedback_citations(
    transaction: &DatabaseTransaction,
    citations: &[FeedbackCitation],
    version_id: VersionId,
    active_chapter_id: ChapterId,
    run_id: RunId,
    candidate_scope: CandidateCitationScope,
) -> Result<(), StoreError> {
    for citation in citations {
        match citation {
            FeedbackCitation::Document {
                document_id,
                revision_id,
                range,
                chapter_id,
            } => {
                validate_document_citation(
                    transaction,
                    *document_id,
                    *revision_id,
                    range,
                    *chapter_id,
                    version_id,
                    Some(active_chapter_id),
                )
                .await?;
            }
            FeedbackCitation::Candidate {
                tool_call_id,
                candidate_hash,
                candidate_range,
            } => {
                let (candidate_content, actual_hash) = match candidate_scope {
                    CandidateCitationScope::HeldCurrentRun => {
                        held_candidate_for_feedback_citation(transaction, run_id, *tool_call_id)
                            .await?
                    }
                    CandidateCitationScope::Proposal {
                        tool_call_id: proposal_tool_call_id,
                    } => {
                        if *tool_call_id != proposal_tool_call_id {
                            return Err(StoreError::InvalidCitation(
                                "待审编辑的引用必须指向当前待审工具调用".to_owned(),
                            ));
                        }
                        proposal_candidate_for_feedback_citation(
                            transaction,
                            run_id,
                            proposal_tool_call_id,
                        )
                        .await?
                    }
                };
                if candidate_hash != &actual_hash {
                    return Err(StoreError::InvalidCitation(
                        "候选正文已变化，请重新选择要引用的文本".to_owned(),
                    ));
                }
                validate_text_citation_range(&candidate_content, candidate_range)?;
            }
        }
    }
    Ok(())
}

/// 校验一条正式文档引用是否逐字锚定在本稿本中的不可变修订上。
///
/// 正文意见、委托审核与人工审核决定共用同一套判定：修订存在、`document_id` 与
/// `revision_id` 匹配、文档未被删除、属于当前稿本、`chapter_id` 与文档归属一致，
/// 最后把引用范围映射回 UTF-8 边界逐字复核 quote。`required_chapter_id` 为 `Some`
/// 时表示本次操作只允许引用该章节的文档；为 `None` 时允许本稿本任意文档（准备
/// 阶段审核）。
pub(crate) async fn validate_document_citation(
    transaction: &DatabaseTransaction,
    document_id: nekowriter_core::DocumentId,
    revision_id: RevisionId,
    range: &nekowriter_core::TextCitationRange,
    cited_chapter_id: Option<ChapterId>,
    version_id: VersionId,
    required_chapter_id: Option<ChapterId>,
) -> Result<(), StoreError> {
    let revision_model = document_revision::Entity::find_by_id(db_id(revision_id))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("被引用的修订不存在".to_owned()))?;
    if revision_model.document_id != db_id(document_id) {
        return Err(StoreError::InvalidCitation(
            "引用中的 document_id 与 revision_id 不匹配".to_owned(),
        ));
    }
    let document_model = document::Entity::find_by_id(db_id(document_id))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("被引用的文档不存在或已删除".to_owned()))?;
    if document_model.version_id != db_id(version_id) {
        return Err(StoreError::InvalidCitation(
            "引用必须属于当前稿本".to_owned(),
        ));
    }
    let actual_chapter_id = parse_optional_id(
        document_model.chapter_id.as_deref(),
        "citation.document.chapter_id",
    )?;
    if actual_chapter_id != cited_chapter_id {
        return Err(StoreError::InvalidCitation(
            "引用中的 chapter_id 与文档所属章节不匹配".to_owned(),
        ));
    }
    if required_chapter_id.is_some_and(|required| actual_chapter_id != Some(required)) {
        return Err(StoreError::InvalidCitation(
            "引用必须指向本次操作所在章节的文档".to_owned(),
        ));
    }
    validate_text_citation_range(&revision_model.content, range)
}

/// 校验一批人工审核决定携带的引用。
///
/// 决定针对的是已经进入审核的产物或正文修订，因此只能引用已持久化的正式文档；
/// 尚未落库的正文候选不属于审核目标，不能借决定绕过候选哈希校验。
pub(crate) async fn validate_review_decision_citations(
    transaction: &DatabaseTransaction,
    citations: &[FeedbackCitation],
    version_id: VersionId,
    required_chapter_id: Option<ChapterId>,
) -> Result<(), StoreError> {
    for citation in citations {
        match citation {
            FeedbackCitation::Document {
                document_id,
                revision_id,
                range,
                chapter_id,
            } => {
                validate_document_citation(
                    transaction,
                    *document_id,
                    *revision_id,
                    range,
                    *chapter_id,
                    version_id,
                    required_chapter_id,
                )
                .await?;
            }
            FeedbackCitation::Candidate { .. } => {
                return Err(StoreError::InvalidCitation(
                    "人工审核决定只能引用已持久化的修订".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// 退回审核可以由文字或已逐字验证的引用构成；这项判断必须留在引用校验之后，
/// 否则一个伪造引用会错误地让空反馈通过。
pub(crate) fn validate_review_decision_feedback(
    decision: &HumanReviewDecision,
    citations: &[FeedbackCitation],
) -> Result<(), StoreError> {
    decision
        .validate_feedback(!citations.is_empty())
        .map_err(|error| StoreError::InvalidReviewDecision(error.to_string()))
}

async fn held_candidate_for_feedback_citation(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
) -> Result<(String, String), StoreError> {
    let model = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .one(transaction)
        .await?
        .ok_or_else(|| {
            StoreError::InvalidCitation("候选引用必须来自当前 Run 中已暂停的工具调用".to_owned())
        })?;
    let held = held_chapter_body_tool_payload_from_model(&model)?.held;
    let candidate_content = held.candidate_content.ok_or_else(|| {
        StoreError::InvalidCitation("该暂停工具调用没有可引用的正文候选".to_owned())
    })?;
    let candidate_hash = held
        .candidate_hash
        .ok_or_else(|| StoreError::CorruptRecord("暂停正文候选缺少 content hash".to_owned()))?;
    Ok((candidate_content, candidate_hash))
}

async fn proposal_candidate_for_feedback_citation(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
) -> Result<(String, String), StoreError> {
    let model = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("awaiting_review"))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("候选引用必须来自当前待审编辑".to_owned()))?;
    let proposal = pending_chapter_body_proposal_from_tool_call(&model)?;
    Ok((proposal.candidate_content, proposal.candidate_hash))
}

pub(crate) fn validate_text_citation_range(
    content: &str,
    range: &nekowriter_core::TextCitationRange,
) -> Result<(), StoreError> {
    if range.start_char < 0 || range.end_char < range.start_char {
        return Err(StoreError::InvalidCitation(
            "引用范围必须是非负的左闭右开区间".to_owned(),
        ));
    }
    let start = usize::try_from(range.start_char)
        .map_err(|_| StoreError::InvalidCitation("引用起点超出当前平台可表示范围".to_owned()))?;
    let end = usize::try_from(range.end_char)
        .map_err(|_| StoreError::InvalidCitation("引用终点超出当前平台可表示范围".to_owned()))?;
    let scalar_count = content.chars().count();
    if start >= end || end > scalar_count {
        return Err(StoreError::InvalidCitation(
            "引用范围不在原文的 Unicode 字符边界内".to_owned(),
        ));
    }
    let start_byte = byte_offset_for_unicode_scalar(content, start).ok_or_else(|| {
        StoreError::InvalidCitation("引用起点不在原文的 Unicode 字符边界内".to_owned())
    })?;
    let end_byte = byte_offset_for_unicode_scalar(content, end).ok_or_else(|| {
        StoreError::InvalidCitation("引用终点不在原文的 Unicode 字符边界内".to_owned())
    })?;
    if content[start_byte..end_byte] != range.quote {
        return Err(StoreError::InvalidCitation(
            "引用文字与原始修订不一致，请重新选择".to_owned(),
        ));
    }
    Ok(())
}

fn byte_offset_for_unicode_scalar(content: &str, scalar_offset: usize) -> Option<usize> {
    if scalar_offset == content.chars().count() {
        Some(content.len())
    } else {
        content
            .char_indices()
            .nth(scalar_offset)
            .map(|(byte_offset, _)| byte_offset)
    }
}

pub(crate) fn feedback_citations_from_json(
    value: &str,
) -> Result<Vec<FeedbackCitation>, StoreError> {
    serde_json::from_str(value).map_err(|error| {
        StoreError::CorruptRecord(format!("自由意见 citations_json 无法解析：{error}"))
    })
}

/// 候选正文被接受为正式修订前，把仍未封存的候选引用重定位到该修订。
///
/// 已封存意见不在这里改写：它们已经进入某个不可变请求快照。当前调用点只会处理
/// 等待投递的同一批意见，因此后续模型消息得到稳定的正式修订锚点，同时不会回写
/// 已发送的历史。
pub(crate) async fn resolve_pending_candidate_feedback_citations(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
    candidate_content: &str,
    document_id: nekowriter_core::DocumentId,
    revision_id: RevisionId,
    chapter_id: ChapterId,
    now: i64,
) -> Result<(), StoreError> {
    let expected_hash = candidate_content_hash(candidate_content);
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .all(transaction)
        .await?;
    for model in models {
        let mut citations = feedback_citations_from_json(&model.citations_json)?;
        let mut changed = false;
        for citation in &mut citations {
            let FeedbackCitation::Candidate {
                tool_call_id: cited_tool_call_id,
                candidate_hash,
                candidate_range,
            } = citation
            else {
                continue;
            };
            if *cited_tool_call_id != tool_call_id {
                continue;
            }
            if *candidate_hash != expected_hash {
                return Err(StoreError::CorruptRecord(
                    "待投递意见的候选引用 hash 与被接受候选不一致".to_owned(),
                ));
            }
            validate_text_citation_range(candidate_content, candidate_range).map_err(|error| {
                StoreError::CorruptRecord(format!("待投递意见的候选引用无效：{error}"))
            })?;
            *citation = FeedbackCitation::Document {
                document_id,
                revision_id,
                range: candidate_range.clone(),
                chapter_id: Some(chapter_id),
            };
            changed = true;
        }
        if changed {
            let mut active = model.into_active_model();
            active.citations_json = Set(serde_json::to_string(&citations)?);
            active.updated_at = Set(now);
            active.update(transaction).await?;
        }
    }
    Ok(())
}

struct FeedbackBatchAnchors {
    after_tool_sequence: i64,
    preferred_tool_call_id: Option<String>,
    bound_tool_call_id: Option<String>,
    status: FeedbackStatus,
}

pub(crate) fn feedback_status_name(status: FeedbackStatus) -> &'static str {
    match status {
        FeedbackStatus::Queued => "queued",
        FeedbackStatus::Bound => "bound",
        FeedbackStatus::Included => "included",
    }
}

pub(crate) fn feedback_status_from_db(status: &str) -> Result<FeedbackStatus, StoreError> {
    match status {
        "queued" => Ok(FeedbackStatus::Queued),
        "bound" => Ok(FeedbackStatus::Bound),
        "included" => Ok(FeedbackStatus::Included),
        _ => Err(StoreError::CorruptRecord(format!(
            "未知的自由意见状态：{status}"
        ))),
    }
}

fn feedback_batch_anchors(
    models: &[feedback::Model],
) -> Result<Option<FeedbackBatchAnchors>, StoreError> {
    let Some(first) = models.first() else {
        return Ok(None);
    };
    let status = feedback_status_from_db(&first.status)?;
    if status == FeedbackStatus::Included {
        return Err(StoreError::CorruptRecord(
            "未封存自由意见批次包含 included 记录".to_owned(),
        ));
    }
    if (status == FeedbackStatus::Queued && first.bound_tool_call_id.is_some())
        || (status == FeedbackStatus::Bound && first.bound_tool_call_id.is_none())
    {
        return Err(StoreError::CorruptRecord(
            "自由意见批次的状态与工具绑定不一致".to_owned(),
        ));
    }
    let anchors = FeedbackBatchAnchors {
        after_tool_sequence: first.after_tool_sequence,
        preferred_tool_call_id: first.preferred_tool_call_id.clone(),
        bound_tool_call_id: first.bound_tool_call_id.clone(),
        status,
    };
    for model in models.iter().skip(1) {
        if feedback_status_from_db(&model.status)? != anchors.status
            || model.after_tool_sequence != anchors.after_tool_sequence
            || model.preferred_tool_call_id != anchors.preferred_tool_call_id
            || model.bound_tool_call_id != anchors.bound_tool_call_id
        {
            return Err(StoreError::CorruptRecord(
                "同一 Run 的未封存自由意见没有共享批次锚点".to_owned(),
            ));
        }
    }
    Ok(Some(anchors))
}

/// 一条刚排进未封存批次的自由意见。
pub(crate) struct QueuedChapterBodyFeedback {
    pub(crate) id: Uuid,
    pub(crate) sequence: i64,
    pub(crate) status: FeedbackStatus,
    /// 意见已归属的工具投放（同一批次共享），未归属时为 `None`。
    pub(crate) bound_tool_call_id: Option<String>,
}

/// 把一条自由意见排进正文 Run 的未封存批次。
///
/// 与 [`Store::submit_run_feedback`] 共用同一套锚点规则（见 [`feedback_batch_anchors`]）：
/// 已经有未封存批次时继承它的锚点，让同一批意见随同一个工具结果一起交付；否则挂到
/// 最后一条已有结果的工具调用之后，等模型产生序号更大的下一条工具结果时由
/// [`bind_pending_feedback_to_tool`] 绑定，随该结果的 `feedback` 数组在下一次请求里
/// 交给模型。未封存期间这条意见同时挡住直接终稿提交（见
/// `commit_chapter_body_model_submission`），因此模型必须先产生一条正文编辑。
///
/// 调用方负责把工作流留在能继续写正文的状态，并写 `feedback.changed` 事件。
pub(crate) async fn queue_chapter_body_feedback(
    transaction: &DatabaseTransaction,
    body_run: &run::Model,
    text: String,
    citations: &[FeedbackCitation],
    now: i64,
) -> Result<QueuedChapterBodyFeedback, StoreError> {
    let existing_batch = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(transaction)
        .await?;
    let anchors = feedback_batch_anchors(&existing_batch)?;
    let (after_tool_sequence, preferred_tool_call_id, bound_tool_call_id, status) =
        if let Some(anchors) = anchors {
            (
                anchors.after_tool_sequence,
                anchors.preferred_tool_call_id,
                anchors.bound_tool_call_id,
                anchors.status,
            )
        } else {
            let after_tool_sequence = tool_call::Entity::find()
                .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
                .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
                .order_by_desc(tool_call::Entity::COLUMN.sequence)
                .one(transaction)
                .await?
                .map_or(0, |tool| tool.sequence);
            (after_tool_sequence, None, None, FeedbackStatus::Queued)
        };
    let sequence = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .order_by_desc(feedback::Entity::COLUMN.sequence)
        .one(transaction)
        .await?
        .map_or(Ok(1), |previous| {
            previous.sequence.checked_add(1).ok_or_else(|| {
                StoreError::InvalidChapterState("自由意见 sequence 已达到 i64 上限".to_owned())
            })
        })?;
    let id = Uuid::new_v4();
    feedback::ActiveModel {
        id: Set(db_id(id)),
        run_id: Set(body_run.id),
        version_id: Set(body_run.version_id),
        sequence: Set(sequence),
        text: Set(text),
        citations_json: Set(serde_json::to_string(citations)?),
        after_tool_sequence: Set(after_tool_sequence),
        preferred_tool_call_id: Set(preferred_tool_call_id),
        bound_tool_call_id: Set(bound_tool_call_id.clone()),
        included_request_id: Set(None),
        status: Set(feedback_status_name(status).to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(transaction)
    .await?;
    Ok(QueuedChapterBodyFeedback {
        id,
        sequence,
        status,
        bound_tool_call_id,
    })
}

pub(crate) fn chapter_body_run_status_matches_execution(
    run_status: &str,
    execution: &ExecutionStatus,
) -> bool {
    matches!(
        (run_status, execution),
        ("queued", ExecutionStatus::Queued)
            | (
                "running",
                ExecutionStatus::RequestInFlight | ExecutionStatus::ToolBoundary
            )
            | (
                "waiting",
                ExecutionStatus::WaitingAnswer | ExecutionStatus::WaitingReview
            )
            | ("paused", ExecutionStatus::Paused)
    )
}

pub(crate) fn writing_run_status_for_execution(
    execution: &ExecutionStatus,
) -> Option<&'static str> {
    match execution {
        ExecutionStatus::Queued => Some("queued"),
        ExecutionStatus::RequestInFlight | ExecutionStatus::ToolBoundary => Some("running"),
        ExecutionStatus::WaitingAnswer | ExecutionStatus::WaitingReview => Some("waiting"),
        ExecutionStatus::Paused => Some("paused"),
        _ => None,
    }
}

/// 在一个新工具结果落库时，为唯一未封存的自由意见批次选择该结果。
///
/// 工具基础结果保持不可变；这里只保存绑定关系。下一次模型请求会在封存快照时
/// 把同一批次的所有意见按序物化到该结果的 `feedback` 数组，故已绑定批次绝不能
/// 被之后的工具调用悄悄改绑。
pub(crate) async fn bind_pending_feedback_to_tool<C: ConnectionTrait>(
    connection: &C,
    run_id: RunId,
    tool_call_id: Uuid,
    tool_sequence: i64,
    now: i64,
) -> Result<Vec<Uuid>, StoreError> {
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?;
    let Some(anchors) = feedback_batch_anchors(&models)? else {
        return Ok(Vec::new());
    };
    let tool_call_id_text = tool_call_id.to_string();
    match anchors.status {
        FeedbackStatus::Included => Err(StoreError::CorruptRecord(
            "未封存自由意见批次不应为 included".to_owned(),
        )),
        FeedbackStatus::Bound => {
            if anchors.bound_tool_call_id.as_deref() != Some(tool_call_id_text.as_str()) {
                return Err(StoreError::CorruptRecord(
                    "存在尚未封存的自由意见批次时又产生了另一条正文工具结果".to_owned(),
                ));
            }
            Ok(Vec::new())
        }
        FeedbackStatus::Queued if tool_sequence <= anchors.after_tool_sequence => Ok(Vec::new()),
        FeedbackStatus::Queued => {
            let bound_tool_call_id = tool_call_id_text;
            let mut changed_ids = Vec::with_capacity(models.len());
            for model in models {
                let id: Uuid = model.id.into();
                let mut active = model.into_active_model();
                active.status = Set("bound".to_owned());
                active.preferred_tool_call_id = Set(Some(bound_tool_call_id.clone()));
                active.bound_tool_call_id = Set(Some(bound_tool_call_id.clone()));
                active.updated_at = Set(now);
                active.update(connection).await?;
                changed_ids.push(id);
            }
            Ok(changed_ids)
        }
    }
}

/// 找到尚未随任何请求封存的最近正文工具结果。
///
/// 这是“暂停尚未命中时提交意见”的唯一可回填目标；已封存的旧结果绝不能被新的
/// 反馈改写。`error` 也是完整、可交付的工具结果，因此与 `resolved` 同等处理。
pub(crate) async fn latest_unsealed_chapter_body_tool_result<C: ConnectionTrait>(
    connection: &C,
    run_id: &sea_orm::prelude::TextUuid,
) -> Result<Option<(Uuid, i64)>, StoreError> {
    tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(*run_id))
        .filter(
            tool_call::Entity::COLUMN
                .status
                .is_in(["resolved", "error"]),
        )
        .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
        .filter(tool_call::Entity::COLUMN.included_request_id.is_null())
        .order_by_desc(tool_call::Entity::COLUMN.sequence)
        .one(connection)
        .await?
        .map(|tool| Ok((tool.id.into(), tool.sequence)))
        .transpose()
}

/// 暂停尚未命中工具边界时，把整个未封存意见批次回填到最近的可交付结果。
///
/// 正常的“下一条工具结果”规则要求序号严格大于 `after_tool_sequence`；暂停特例
/// 则刻意允许等于它，因为意见抵达时最近结果已经形成但尚未随请求发送。所有
/// 记录同步写入相同的 preferred/bound 锚点，保证随后追加的意见不会拆批。
pub(crate) async fn bind_pending_feedback_to_existing_tool_for_pause<C: ConnectionTrait>(
    connection: &C,
    run_id: RunId,
    tool_call_id: Uuid,
    now: i64,
) -> Result<Vec<Uuid>, StoreError> {
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?;
    let Some(anchors) = feedback_batch_anchors(&models)? else {
        return Ok(Vec::new());
    };
    let tool_call_id_text = tool_call_id.to_string();
    match anchors.status {
        FeedbackStatus::Included => Err(StoreError::CorruptRecord(
            "未封存自由意见批次不应为 included".to_owned(),
        )),
        FeedbackStatus::Bound => {
            if anchors.bound_tool_call_id.as_deref() != Some(tool_call_id_text.as_str()) {
                return Err(StoreError::CorruptRecord(
                    "暂停回填意见时批次已经绑定到另一条工具结果".to_owned(),
                ));
            }
            Ok(Vec::new())
        }
        FeedbackStatus::Queued => {
            let mut changed_ids = Vec::with_capacity(models.len());
            for model in models {
                let id: Uuid = model.id.into();
                let mut active = model.into_active_model();
                active.status = Set("bound".to_owned());
                active.preferred_tool_call_id = Set(Some(tool_call_id_text.clone()));
                active.bound_tool_call_id = Set(Some(tool_call_id_text.clone()));
                active.updated_at = Set(now);
                active.update(connection).await?;
                changed_ids.push(id);
            }
            Ok(changed_ids)
        }
    }
}
