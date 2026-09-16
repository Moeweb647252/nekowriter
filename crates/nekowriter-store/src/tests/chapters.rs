//! 章节：提纲提交的形状、终审驳回把问题交给正文 Agent、正文编辑失败的码与形状。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn outline_submission_requires_a_structured_chapter_plan() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("章节计划校验", "港城雨夜", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部在港城旧码头展开的悬疑小说。 ").unwrap(),
        )
        .await
        .unwrap();
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();

    let error = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            started.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(PreparationArtifactKind::Background, "背景").unwrap(),
                PreparationArtifactInput::new(PreparationArtifactKind::Research, "资料").unwrap(),
                PreparationArtifactInput::new(PreparationArtifactKind::Characters, "人物").unwrap(),
                PreparationArtifactInput::new(PreparationArtifactKind::Outline, "粗纲").unwrap(),
            ],
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::InvalidPreparationArtifacts(message)
            if message.contains("章节计划")
    ));
    assert_eq!(
        store
            .get_preparation_snapshot(version_id)
            .await
            .unwrap()
            .workflow
            .stage,
        WorkflowStage::PreparingGenerating
    );
}

/// 整章审核「退回修订」写下的意见必须到达正文 Agent。
///
/// 它与面板里的自由意见走同一条通道：退回时排进未封存批次（`queued`），模型产生
/// 下一条正文编辑的结果时绑定（`bound`），随该结果进入下一次请求（`included`）。
/// 只把意见写进 `reviews.issues_json` 会让退回后的原稿空转回到审核
/// （docs/03 §43.4 a），而界面标签与正文提示词都承诺这份意见会到模型。
#[tokio::test]
async fn chapter_final_rejection_hands_its_issues_to_the_body_agent() {
    use crate::entity::{feedback, tool_call};

    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("退回修订", "雨夜车站", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部围绕雨夜车站和失踪信件展开的悬疑短篇。").unwrap(),
        )
        .await
        .unwrap();
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();
    let mut snapshot = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            started.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Background,
                    "雨夜的旧车站即将拆除，末班车仍在运行。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "用户提供了一封没有署名的旧信，出处尚待核验。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Characters,
                    "沈宁是追查失踪姐姐下落的摄影记者。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Outline,
                    "沈宁在末班车上收到旧信，决定进入废弃候车室寻找寄信人。",
                )
                .unwrap(),
            ],
            Some(vec![
                ChapterPlanDraft::new("末班车", "沈宁在雨夜车站收到一封旧信。").unwrap(),
            ]),
        )
        .await
        .unwrap();
    for kind in PreparationArtifactKind::ALL {
        let review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == kind)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        snapshot = store
            .decide_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
    }
    let chapter_id = snapshot.workflow.active_chapter_id.unwrap();
    store
        .save_chapter_guidance(
            chapter_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            NewChapterGuidance::new("让沈宁先辨认旧信上的车站印章。").unwrap(),
        )
        .await
        .unwrap();
    let ready = store.get_preparation_snapshot(version_id).await.unwrap();
    let outline_started = store
        .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
        .await
        .unwrap();
    let outline = store
        .submit_chapter_outline(
            chapter_id,
            Uuid::new_v4(),
            outline_started.state_revision,
            NewChapterOutline::new(
                "1. 沈宁辨认旧信上的车站印章。\n2. 末班车鸣笛，迫使她决定是否追上寄信人。",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let outline_review_id = outline.outline.review.as_ref().unwrap().id;
    let drafting = store
        .decide_review(
            outline_review_id,
            Uuid::new_v4(),
            outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(drafting.workflow.stage, WorkflowStage::WritingDrafting);
    let body_run_id = drafting.workflow.active_run_id.unwrap();

    // 模型写出正文，人工提交整章审核。
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: body_run_id,
                settings_snapshot: json!({ "model": "writing-test" }),
                prompt_snapshot: json!({ "key": "chapter.write" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let body_content = "沈宁攥紧旧信，踏上刚刚进站的末班车。";
    let applied = store
        .execute_chapter_body_agent_tool_call(
            request.id,
            chapter_id,
            Some("writing-provider-body".to_owned()),
            json!({}),
            ChapterBodyAgentToolCall {
                provider_call_id: "body-call".to_owned(),
                name: "append_content".to_owned(),
                arguments: json!({ "content": body_content }),
            },
        )
        .await
        .unwrap();
    assert!(applied.newly_resolved);
    let written = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(written.body.content.as_deref(), Some(body_content));
    let submitted = store
        .submit_chapter_final_review(chapter_id, Uuid::new_v4(), written.workflow.state_revision)
        .await
        .unwrap();
    let review = submitted.body.review.clone().unwrap();
    assert_eq!(review.status, ReviewStatus::Open);
    let review_id = review.id;
    let rejected_revision_id = review.target_revision_id;

    // 退回：意见必须排进正文 Run 的自由意见批次，等着随下一条正文编辑投递。
    let issues = "让沈宁主动留下查清寄信人；第二段的雨声描写与结尾重复。";
    let rejected = store
        .decide_review(
            review_id,
            Uuid::new_v4(),
            submitted.workflow.state_revision,
            HumanReviewDecision::new(false, issues).unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(rejected.workflow.execution, ExecutionStatus::Queued);
    assert_eq!(rejected.workflow.active_run_id, Some(body_run_id));
    let rejected_body = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(rejected_body.pending_feedback.len(), 1);
    assert_eq!(rejected_body.pending_feedback[0].text, issues);
    assert_eq!(
        rejected_body.pending_feedback[0].status,
        nekowriter_core::FeedbackStatus::Queued
    );
    assert_eq!(rejected_body.pending_feedback[0].bound_tool_call_id, None);
    // 锚点取最后一条已有结果的工具调用：只有序号更大的下一条结果才会带上它。
    let anchor = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(body_run_id)))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(anchor.sequence, 1);
    assert_eq!(anchor.after_tool_sequence, 1);
    assert_eq!(anchor.status, "queued");
    assert_eq!(
        store
            .list_materialized_chapter_body_feedback(body_run_id)
            .await
            .unwrap(),
        Vec::new(),
        "尚未产生新的工具结果时，意见还不能算已投递"
    );

    // 模型按退回意见改一次正文：意见随这条结果绑定。这一次请求也把上一轮那条
    // 尚未封存的写作结果带进快照（调度器就是这么传的）。
    assert_eq!(
        store
            .list_unsealed_agent_tool_result_provider_ids(body_run_id)
            .await
            .unwrap(),
        vec!["body-call".to_owned()]
    );
    let revision_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: body_run_id,
                settings_snapshot: json!({ "model": "writing-test" }),
                prompt_snapshot: json!({ "key": "chapter.write" }),
                context_snapshot: json!({
                    "messages": [],
                    "toolResultBinding": { "providerCallIds": ["body-call"] },
                }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    store
            .execute_chapter_body_agent_tool_call(
                revision_request.id,
                chapter_id,
                Some("writing-provider-revision".to_owned()),
                json!({}),
                ChapterBodyAgentToolCall {
                    provider_call_id: "revision-call".to_owned(),
                    name: "replace_content".to_owned(),
                    arguments: json!({ "old_str": "踏上刚刚进站的末班车", "new_str": "退掉车票，留下等寄信人" }),
                },
            )
            .await
            .unwrap();
    let bound = store
        .list_materialized_chapter_body_feedback(body_run_id)
        .await
        .unwrap();
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].text, issues);
    assert_eq!(bound[0].provider_tool_call_id, "revision-call");
    assert_eq!(bound[0].status, nekowriter_core::FeedbackStatus::Bound);
    let revised = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(
        revised.body.content.as_deref(),
        Some("沈宁攥紧旧信，退掉车票，留下等寄信人。")
    );
    assert_ne!(revised.body.revision_id, Some(rejected_revision_id));

    // 下一次请求封存时这条意见进入请求快照：退回后的修稿真的带上了意见。
    // 两个绑定就是调度器实际写进上下文快照的形状（已绑定意见 + 它的工具结果）。
    let sealed = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: body_run_id,
                settings_snapshot: json!({ "model": "writing-test" }),
                prompt_snapshot: json!({ "key": "chapter.write" }),
                context_snapshot: json!({
                    "messages": [],
                    "feedbackBinding": { "ids": [bound[0].id.to_string()] },
                    "toolResultBinding": { "providerCallIds": ["revision-call"] },
                }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let included = store
        .list_materialized_chapter_body_feedback(body_run_id)
        .await
        .unwrap();
    assert_eq!(included.len(), 1);
    assert_eq!(
        included[0].status,
        nekowriter_core::FeedbackStatus::Included
    );
    let included_row = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(body_run_id)))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(included_row.status, "included");
    assert_eq!(
        included_row.included_request_id,
        Some(sealed.id.to_string())
    );
    let sealed_tool = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(body_run_id)))
        .filter(
            tool_call::Entity::COLUMN
                .provider_call_id
                .eq("revision-call"),
        )
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sealed_tool.included_request_id, Some(sealed.id.to_string()));
}

/// 正文编辑失败时模型看到的完整结果（码、消息、字段集），以及失败不推进正文。
///
/// 这些码此前只出现在 `agent_body_edit_error_result` 的定义处。这里经公开入口
/// `execute_chapter_body_agent_tool_call` 把四条分支各走一遍：`agent_body_edit_error_result`
/// 的形参由 `ChapterBodyEditError` 改成 `&ChapterBodyEditError` 不改变其中任何一项。
#[tokio::test]
async fn chapter_body_edit_failures_report_their_codes_shapes_and_keep_the_body() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (_version_id, chapter_id, run_id) = drafting_chapter_body_run(&store, "正文编辑错误").await;

    // 起点：追加一段含两处「钟」的正文，后面用其中一处构造“匹配多处”。
    let opened = "灯火在雨里摇晃。车站的钟响了三次。钟声散进雾里。";
    let appended = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        1,
        "append_content",
        json!({ "content": opened }),
    )
    .await;
    assert!(appended.newly_resolved);
    assert_eq!(appended.result["status"], "applied");
    let after_append = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(after_append.body.content.as_deref(), Some(opened));
    let revision_id = after_append.body.revision_id.unwrap();
    let revision_no = after_append.body.revision_no;
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 0);

    // 1) 没有匹配 → EDIT_NO_MATCH：模型看到的就是这六个字段，没有多余键。
    let missing = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        2,
        "replace_content",
        json!({ "old_str": "候车室的灯灭了", "new_str": "候车室的灯又亮了" }),
    )
    .await;
    assert!(missing.newly_resolved);
    assert!(missing.requeue);
    assert_eq!(missing.provider_call_id, "body-tool-call-2");
    assert_eq!(
        missing.result,
        json!({
            "status": "error",
            "code": "EDIT_NO_MATCH",
            "message": "待替换正文在当前章节中没有匹配",
            "tool_call_id": "body-tool-call-2",
            "base_revision_id": revision_id.to_string(),
            "feedback": [],
        })
    );
    let after_missing = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(after_missing.body.revision_id, Some(revision_id));
    assert_eq!(after_missing.body.revision_no, revision_no);
    assert_eq!(after_missing.body.content.as_deref(), Some(opened));
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 1);

    // 2) 匹配多处 → EDIT_AMBIGUOUS_MATCH（“钟”在正文里出现两次）。
    let ambiguous = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        3,
        "replace_content",
        json!({ "old_str": "钟", "new_str": "远处的钟" }),
    )
    .await;
    assert!(ambiguous.newly_resolved);
    assert!(ambiguous.requeue);
    assert_eq!(
        ambiguous.result,
        json!({
            "status": "error",
            "code": "EDIT_AMBIGUOUS_MATCH",
            "message": "待替换正文在当前章节中匹配多处",
            "tool_call_id": "body-tool-call-3",
            "base_revision_id": revision_id.to_string(),
            "feedback": [],
        })
    );
    let after_ambiguous = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(after_ambiguous.body.revision_id, Some(revision_id));
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 2);

    // 3) 一次成功替换：计数归零，正文确实推进一版——失败没有把 Run 写坏。
    let applied = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        4,
        "replace_content",
        json!({ "old_str": "灯火在雨里摇晃。", "new_str": "灯火在雨里摇晃，站台空得只剩回声。" }),
    )
    .await;
    assert_eq!(applied.result["status"], "applied");
    let after_replace = store.get_chapter_body(chapter_id).await.unwrap();
    let replaced = "灯火在雨里摇晃，站台空得只剩回声。车站的钟响了三次。钟声散进雾里。";
    assert_eq!(after_replace.body.content.as_deref(), Some(replaced));
    assert_eq!(after_replace.body.revision_no, revision_no.map(|no| no + 1));
    let revision_id = after_replace.body.revision_id.unwrap();
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 0);

    // 4) 新旧文本相同 → EDIT_NO_CHANGE（在匹配之前就判定，所以文本可以不在正文里）。
    let unchanged = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        5,
        "replace_content",
        json!({ "old_str": "站台空得只剩回声", "new_str": "站台空得只剩回声" }),
    )
    .await;
    assert!(unchanged.newly_resolved);
    assert_eq!(
        unchanged.result,
        json!({
            "status": "error",
            "code": "EDIT_NO_CHANGE",
            "message": "替换前后文本完全相同，不会创建无意义修订",
            "tool_call_id": "body-tool-call-5",
            "base_revision_id": revision_id.to_string(),
            "feedback": [],
        })
    );
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 1);

    // 5) 空 old_str 在参数解析阶段就被拒：码是参数层的 VALIDATION_ERROR，
    //    消息来自 `ReplaceChapterContent::new`，不经过 `agent_body_edit_error_result`。
    let empty = dispatched_body_tool_call(
        &store,
        chapter_id,
        run_id,
        6,
        "replace_content",
        json!({ "old_str": "", "new_str": "替换后的正文" }),
    )
    .await;
    assert!(empty.newly_resolved);
    assert_eq!(
        empty.result,
        json!({
            "status": "error",
            "code": "VALIDATION_ERROR",
            "message": "待替换正文不能为空",
            "tool_call_id": "body-tool-call-6",
            "base_revision_id": revision_id.to_string(),
            "feedback": [],
        })
    );

    // 收束：失败计数是 2（未到上限 3），Run 仍可写，正文停在成功替换后的那一版。
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 2);
    let final_body = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(final_body.workflow.execution, ExecutionStatus::Queued);
    assert_eq!(final_body.body.content.as_deref(), Some(replaced));
    assert_eq!(final_body.body.revision_no, revision_no.map(|no| no + 1));
}
