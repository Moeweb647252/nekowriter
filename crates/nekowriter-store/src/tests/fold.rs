//! 端到端：从准备到提纲、正文、终审跑完，检查折叠出的完整正文。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn approved_preparation_artifacts_fold_to_complete_plaintext_after_the_last_review() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("折叠测试", "港城雨夜", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部关于海港城与旧报纸的悬疑小说。").unwrap(),
        )
        .await
        .unwrap();
    let mut snapshot = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();
    let preparation_run_id = snapshot.workflow.active_run_id.unwrap();
    store
        .append_agent_messages(
            preparation_run_id,
            vec![NewAgentMessage {
                role: AgentMessageRole::Assistant,
                content: json!({
                    "kind": "text",
                    "role": "assistant",
                    "content": "我会先形成四项准备产物。"
                }),
                request_id: None,
                tool_call_id: None,
            }],
        )
        .await
        .unwrap();
    snapshot = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Outline,
                    "# 粗纲\n\n第一章：雨夜的来客",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Characters,
                    "林舟：调查旧报纸的记者。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "资料：用户提供的 1998 年港城晚报。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Background,
                    "海港城连续七日暴雨，旧码头即将拆除。",
                )
                .unwrap(),
            ],
            Some(vec![
                nekowriter_core::ChapterPlanDraft::new(
                    "雨夜的来客",
                    "林舟在暴雨中收到一张来自旧码头的匿名报纸，决定追查线索。",
                )
                .unwrap(),
                nekowriter_core::ChapterPlanDraft::new(
                    "仓库里的旧名",
                    "林舟循着报纸背面的名字回到仓库，必须决定是否公开父亲旧案。",
                )
                .unwrap(),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);
    let submitted_plan_item_id = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
        .and_then(|artifact| artifact.chapter_plan.as_ref())
        .map(|plan| {
            assert_eq!(plan.items.len(), 2);
            assert_eq!(plan.items[0].title, "雨夜的来客");
            assert_eq!(plan.items[1].title, "仓库里的旧名");
            plan.items[0].plan_item_id
        })
        .unwrap();

    for kind in [
        PreparationArtifactKind::Outline,
        PreparationArtifactKind::Background,
        PreparationArtifactKind::Research,
        PreparationArtifactKind::Characters,
    ] {
        let review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == kind)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        snapshot = store
            .decide_preparation_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
    }

    assert_eq!(snapshot.workflow.stage, WorkflowStage::WritingChapterReady);
    assert_eq!(snapshot.workflow.active_run_id, None);
    let fold = snapshot.context_fold.as_ref().unwrap();
    assert_eq!(
        fold.plain_text,
        "海港城连续七日暴雨，旧码头即将拆除。\n\n资料：用户提供的 1998 年港城晚报。\n\n林舟：调查旧报纸的记者。\n\n# 粗纲\n\n第一章：雨夜的来客\n\n第1章：雨夜的来客\n林舟在暴雨中收到一张来自旧码头的匿名报纸，决定追查线索。\n\n第2章：仓库里的旧名\n林舟循着报纸背面的名字回到仓库，必须决定是否公开父亲旧案。"
    );
    assert!(!fold.plain_text.contains("背景设定"));
    assert_eq!(fold.through_message_sequence, 1);
    let chapters = store.list_chapters(version_id).await.unwrap();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].title, "雨夜的来客");
    assert_eq!(chapters[0].kind, nekowriter_core::ChapterKind::Main);
    assert_eq!(chapters[0].status, nekowriter_core::ChapterStatus::Planned);
    assert_eq!(chapters[0].plan_item_id, Some(submitted_plan_item_id));
    assert_eq!(chapters[1].title, "仓库里的旧名");
    assert_eq!(chapters[1].kind, nekowriter_core::ChapterKind::Main);
    assert_eq!(chapters[1].status, nekowriter_core::ChapterStatus::Planned);
    assert_eq!(snapshot.workflow.active_chapter_id, Some(chapters[0].id));
    let initial_guidance = store.get_chapter_guidance(chapters[0].id).await.unwrap();
    assert_eq!(initial_guidance.content, None);

    let saved_guidance = store
        .save_chapter_guidance(
            chapters[0].id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            NewChapterGuidance::new("让林舟先观察码头的异常灯火。 ").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        saved_guidance.content.as_deref(),
        Some("让林舟先观察码头的异常灯火。 ")
    );
    let after_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
    let outline_started = store
        .start_chapter_outline(
            chapters[0].id,
            Uuid::new_v4(),
            after_guidance.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(
        outline_started.stage,
        WorkflowStage::WritingOutlineGenerating
    );
    assert_eq!(outline_started.active_chapter_id, Some(chapters[0].id));
    assert!(outline_started.active_run_id.is_some());
    assert_eq!(
        store.list_chapters(version_id).await.unwrap()[0].status,
        nekowriter_core::ChapterStatus::InProgress
    );

    let first_outline = store
            .submit_chapter_outline(
                chapters[0].id,
                Uuid::new_v4(),
                outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 林舟观察码头的异常灯火。\n2. 他在仓库外发现被撕去名字的报纸。\n3. 结尾让陌生来电打断追查。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
    assert_eq!(
        first_outline.workflow.stage,
        WorkflowStage::WritingOutlineReviewing
    );
    assert_eq!(first_outline.context_fold, None);
    let first_review_id = first_outline.outline.review.as_ref().unwrap().id;
    let rejected = store
        .decide_review(
            first_review_id,
            Uuid::new_v4(),
            first_outline.workflow.state_revision,
            HumanReviewDecision::new(false, "请让报纸线索与林舟的个人风险更直接相关。").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        rejected.workflow.stage,
        WorkflowStage::WritingOutlineGenerating
    );
    assert_eq!(
        rejected.workflow.active_run_id,
        outline_started.active_run_id
    );
    assert_eq!(
        store
            .get_chapter_outline(chapters[0].id)
            .await
            .unwrap()
            .outline
            .review
            .unwrap()
            .status,
        nekowriter_core::ReviewStatus::Rejected
    );

    let revised_outline_content = "1. 林舟观察码头的异常灯火。\n2. 他发现父亲旧案中的报纸被人藏进仓库。\n3. 陌生来电警告他立刻离开。";
    let revised_outline = store
        .submit_chapter_outline(
            chapters[0].id,
            Uuid::new_v4(),
            rejected.workflow.state_revision,
            NewChapterOutline::new(revised_outline_content).unwrap(),
        )
        .await
        .unwrap();
    let revised_review_id = revised_outline.outline.review.as_ref().unwrap().id;
    let approval_command_id = Uuid::new_v4();
    let drafting = store
        .decide_review(
            revised_review_id,
            approval_command_id,
            revised_outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(drafting.workflow.stage, WorkflowStage::WritingDrafting);
    let body_run_id = drafting.workflow.active_run_id.unwrap();
    let replayed = store
        .decide_review(
            revised_review_id,
            approval_command_id,
            revised_outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(replayed, drafting);

    let approved_outline = store.get_chapter_outline(chapters[0].id).await.unwrap();
    let outline_fold = approved_outline.context_fold.as_ref().unwrap();
    assert_eq!(outline_fold.plain_text, revised_outline_content);
    assert_eq!(outline_fold.artifact_revision_ids.len(), 1);
    assert_eq!(outline_fold.through_message_sequence, 0);
    assert!(!outline_fold.plain_text.contains("审核"));

    let first_body = store
        .append_chapter_body(
            chapters[0].id,
            Uuid::new_v4(),
            drafting.workflow.state_revision,
            AppendChapterContent::new("林舟踏进码头仓库，雨水沿着衣角滴下。").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_body.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(first_body.workflow.active_run_id, Some(body_run_id));
    let appended_body = store
        .append_chapter_body(
            chapters[0].id,
            Uuid::new_v4(),
            first_body.workflow.state_revision,
            AppendChapterContent::new("\n他把父亲的名字写在报纸背面。").unwrap(),
        )
        .await
        .unwrap();
    let replaced_body = store
        .replace_chapter_body(
            chapters[0].id,
            Uuid::new_v4(),
            appended_body.workflow.state_revision,
            ReplaceChapterContent::new("码头仓库", "旧仓库").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        replaced_body.body.content.as_deref(),
        Some("林舟踏进旧仓库，雨水沿着衣角滴下。\n他把父亲的名字写在报纸背面。")
    );
    let unchanged_body = store
        .replace_chapter_body(
            chapters[0].id,
            Uuid::new_v4(),
            replaced_body.workflow.state_revision,
            ReplaceChapterContent::new("旧仓库", "旧仓库").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unchanged_body, replaced_body);

    let first_final_review = store
        .submit_chapter_final_review(
            chapters[0].id,
            Uuid::new_v4(),
            replaced_body.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(
        first_final_review.workflow.stage,
        WorkflowStage::WritingChapterReviewing
    );
    let first_final_review_id = first_final_review.body.review.as_ref().unwrap().id;
    let body_rejected = store
        .decide_review(
            first_final_review_id,
            Uuid::new_v4(),
            first_final_review.workflow.state_revision,
            HumanReviewDecision::new(false, "请补足林舟带走报纸的选择与后果。").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(body_rejected.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(body_rejected.workflow.active_run_id, Some(body_run_id));
    let revised_body = store
        .append_chapter_body(
            chapters[0].id,
            Uuid::new_v4(),
            body_rejected.workflow.state_revision,
            AppendChapterContent::new("\n他决定先带走报纸，再追查来电来源。").unwrap(),
        )
        .await
        .unwrap();
    let final_review = store
        .submit_chapter_final_review(
            chapters[0].id,
            Uuid::new_v4(),
            revised_body.workflow.state_revision,
        )
        .await
        .unwrap();
    let final_review_id = final_review.body.review.as_ref().unwrap().id;
    let final_approval_command_id = Uuid::new_v4();
    let chapter_done = store
        .decide_review(
            final_review_id,
            final_approval_command_id,
            final_review.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        chapter_done.workflow.stage,
        WorkflowStage::WritingChapterDone
    );
    assert_eq!(chapter_done.workflow.active_run_id, None);
    assert_eq!(
        store.list_chapters(version_id).await.unwrap()[0].status,
        nekowriter_core::ChapterStatus::Approved
    );
    let advance_command_id = Uuid::new_v4();
    let next_chapter_ready = store
        .advance_to_next_chapter(
            version_id,
            advance_command_id,
            chapter_done.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(
        next_chapter_ready.workflow.stage,
        WorkflowStage::WritingChapterReady
    );
    assert_eq!(next_chapter_ready.workflow.active_run_id, None);
    assert_eq!(
        next_chapter_ready.workflow.active_chapter_id,
        Some(chapters[1].id)
    );
    assert_eq!(
        store.list_chapters(version_id).await.unwrap()[1].status,
        nekowriter_core::ChapterStatus::Planned
    );
    let replayed_next_chapter = store
        .advance_to_next_chapter(
            version_id,
            advance_command_id,
            chapter_done.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(replayed_next_chapter, next_chapter_ready);
    assert_eq!(
        store
            .get_chapter_guidance(chapters[1].id)
            .await
            .unwrap()
            .content,
        None
    );
    let approved_body = store.get_chapter_body(chapters[0].id).await.unwrap();
    let body_fold = approved_body.context_fold.as_ref().unwrap();
    assert_eq!(body_fold.artifact_revision_ids.len(), 1);
    assert_eq!(
        body_fold.plain_text,
        "林舟踏进旧仓库，雨水沿着衣角滴下。\n他把父亲的名字写在报纸背面。\n他决定先带走报纸，再追查来电来源。"
    );
    assert!(approved_body.outline_context_fold.is_some());

    let copy_command_id = Uuid::new_v4();
    let copied = store
        .copy_version(
            version_id,
            copy_command_id,
            NewCopiedNovelVersion::new("第二稿").unwrap(),
        )
        .await
        .unwrap();
    let replayed_copy = store
        .copy_version(
            version_id,
            copy_command_id,
            NewCopiedNovelVersion::new("第二稿").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replayed_copy, copied);
    assert_eq!(copied.version.source_version_id, Some(version_id));
    assert_eq!(copied.workflow.stage, WorkflowStage::WritingChapterReady);
    assert_eq!(copied.workflow.execution, ExecutionStatus::Idle);
    assert_eq!(copied.workflow.active_run_id, None);
    let copied_chapters = store.list_chapters(copied.version.id).await.unwrap();
    assert_eq!(copied_chapters.len(), 2);
    assert_eq!(
        copied_chapters[0].status,
        nekowriter_core::ChapterStatus::Approved
    );
    assert_eq!(
        copied_chapters[1].status,
        nekowriter_core::ChapterStatus::Planned
    );
    assert_eq!(
        copied.workflow.active_chapter_id,
        Some(copied_chapters[1].id)
    );
    let copied_preparation = store
        .get_preparation_snapshot(copied.version.id)
        .await
        .unwrap();
    assert_eq!(
        copied_preparation
            .context_fold
            .as_ref()
            .map(|fold| fold.plain_text.as_str()),
        Some(fold.plain_text.as_str())
    );
    let copied_contexts = store
        .list_completed_run_context_folds(copied.version.id)
        .await
        .unwrap();
    assert_eq!(copied_contexts.len(), 3);
    assert!(copied_contexts.iter().any(|context| {
        context.stage == CompletedRunContextStage::Preparation && context.chapter_order.is_none()
    }));
    assert!(copied_contexts.iter().any(|context| {
        context.stage == CompletedRunContextStage::ChapterOutline
            && context.chapter_order == Some(1)
    }));
    assert!(copied_contexts.iter().any(|context| {
        context.stage == CompletedRunContextStage::ChapterBody && context.chapter_order == Some(1)
    }));
    let copied_first_body = store.get_chapter_body(copied_chapters[0].id).await.unwrap();
    assert_eq!(
        copied_first_body
            .context_fold
            .as_ref()
            .map(|fold| fold.plain_text.as_str()),
        Some(body_fold.plain_text.as_str())
    );
    let second_guidance = store
        .save_chapter_guidance(
            chapters[1].id,
            Uuid::new_v4(),
            next_chapter_ready.workflow.state_revision,
            NewChapterGuidance::new("让林舟决定是否公开父亲旧案。 ").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        second_guidance.content.as_deref(),
        Some("让林舟决定是否公开父亲旧案。 ")
    );
    let after_second_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
    let second_outline_started = store
        .start_chapter_outline(
            chapters[1].id,
            Uuid::new_v4(),
            after_second_guidance.workflow.state_revision,
        )
        .await
        .unwrap();
    let second_outline = store
            .submit_chapter_outline(
                chapters[1].id,
                Uuid::new_v4(),
                second_outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 林舟回到仓库确认报纸背面的旧名。\n2. 他发现父亲旧案被人刻意遮掩。\n3. 结尾让他决定公开线索。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
    let second_drafting = store
        .decide_review(
            second_outline.outline.review.as_ref().unwrap().id,
            Uuid::new_v4(),
            second_outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    let second_body = store
        .append_chapter_body(
            chapters[1].id,
            Uuid::new_v4(),
            second_drafting.workflow.state_revision,
            AppendChapterContent::new("林舟把旧名刊登在清晨的报纸上。 ").unwrap(),
        )
        .await
        .unwrap();
    let second_final_review = store
        .submit_chapter_final_review(
            chapters[1].id,
            Uuid::new_v4(),
            second_body.workflow.state_revision,
        )
        .await
        .unwrap();
    let second_chapter_done = store
        .decide_review(
            second_final_review.body.review.as_ref().unwrap().id,
            Uuid::new_v4(),
            second_final_review.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        second_chapter_done.workflow.stage,
        WorkflowStage::WritingChapterDone
    );
    assert_eq!(
        second_chapter_done.workflow.active_chapter_id,
        Some(chapters[1].id)
    );
    let completion_command_id = Uuid::new_v4();
    let completed = store
        .confirm_novel_completion(
            version_id,
            completion_command_id,
            second_chapter_done.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(completed.workflow.stage, WorkflowStage::Completed);
    assert_eq!(completed.workflow.active_run_id, None);
    assert_eq!(completed.workflow.active_chapter_id, Some(chapters[1].id));
    assert_eq!(
        store.list_chapters(version_id).await.unwrap()[1].status,
        nekowriter_core::ChapterStatus::Approved
    );
    let replayed_completion = store
        .confirm_novel_completion(
            version_id,
            completion_command_id,
            second_chapter_done.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(replayed_completion, completed);
    assert!(
        store
            .get_version_snapshot(version_id)
            .await
            .unwrap()
            .version
            .body_completed_at_ms
            .is_some()
    );

    let body_completed_at = store
        .get_version_snapshot(version_id)
        .await
        .unwrap()
        .version
        .body_completed_at_ms;
    let extra_command_id = Uuid::new_v4();
    let extra_ready = store
        .add_extra_chapter(
            version_id,
            extra_command_id,
            completed.workflow.state_revision,
            NewExtraChapter::new("雨停之后").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        extra_ready.workflow.stage,
        WorkflowStage::WritingChapterReady
    );
    let replayed_extra = store
        .add_extra_chapter(
            version_id,
            extra_command_id,
            completed.workflow.state_revision,
            NewExtraChapter::new("雨停之后").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replayed_extra, extra_ready);
    let extra = store
        .list_chapters(version_id)
        .await
        .unwrap()
        .into_iter()
        .find(|chapter| chapter.kind == nekowriter_core::ChapterKind::Extra)
        .unwrap();
    assert_eq!(extra.order_index, 3);
    assert_eq!(extra.plan_item_id, None);
    assert_eq!(
        store.get_chapter_guidance(extra.id).await.unwrap().content,
        None
    );

    store
        .save_chapter_guidance(
            extra.id,
            Uuid::new_v4(),
            extra_ready.workflow.state_revision,
            NewChapterGuidance::new("让林舟在雨停后回到旧码头，确认来信人的身份。").unwrap(),
        )
        .await
        .unwrap();
    let extra_after_guidance = store.get_preparation_snapshot(version_id).await.unwrap();
    let extra_outline_started = store
        .start_chapter_outline(
            extra.id,
            Uuid::new_v4(),
            extra_after_guidance.workflow.state_revision,
        )
        .await
        .unwrap();
    let extra_outline = store
            .submit_chapter_outline(
                extra.id,
                Uuid::new_v4(),
                extra_outline_started.state_revision,
                NewChapterOutline::new(
                    "1. 雨停后林舟重返码头。\n2. 他从来信人手中得知旧案的最后真相。\n3. 以清晨的报纸收束这一段后记。",
                )
                .unwrap(),
            )
            .await
            .unwrap();
    let extra_drafting = store
        .decide_review(
            extra_outline.outline.review.as_ref().unwrap().id,
            Uuid::new_v4(),
            extra_outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    let extra_body = store
        .append_chapter_body(
            extra.id,
            Uuid::new_v4(),
            extra_drafting.workflow.state_revision,
            AppendChapterContent::new("雨停时，林舟在旧码头读完了最后一封信。").unwrap(),
        )
        .await
        .unwrap();
    let extra_final_review = store
        .submit_chapter_final_review(extra.id, Uuid::new_v4(), extra_body.workflow.state_revision)
        .await
        .unwrap();
    let extra_completed = store
        .decide_review(
            extra_final_review.body.review.as_ref().unwrap().id,
            Uuid::new_v4(),
            extra_final_review.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(extra_completed.workflow.stage, WorkflowStage::Completed);
    assert_eq!(extra_completed.workflow.active_chapter_id, Some(extra.id));
    assert_eq!(extra_completed.workflow.active_run_id, None);
    assert_eq!(
        store
            .get_version_snapshot(version_id)
            .await
            .unwrap()
            .version
            .body_completed_at_ms,
        body_completed_at
    );
    assert_eq!(
        store
            .get_chapter_body(extra.id)
            .await
            .unwrap()
            .context_fold
            .as_ref()
            .map(|fold| fold.plain_text.as_str()),
        Some("雨停时，林舟在旧码头读完了最后一封信。")
    );

    // 回收站：番外软删除后正文、修订 id 与位置都不动，恢复时原样回来。
    let body_before_delete = store.get_chapter_body(extra.id).await.unwrap().body;
    let revision_before_delete = body_before_delete.revision_id.unwrap();
    let deleted_snapshot = store
        .delete_extra_chapter(
            extra.id,
            Uuid::new_v4(),
            extra_completed.workflow.state_revision,
        )
        .await
        .unwrap();
    assert_eq!(deleted_snapshot.workflow.stage, WorkflowStage::Completed);
    assert_eq!(
        deleted_snapshot.workflow.active_chapter_id,
        Some(chapters[1].id)
    );
    assert!(
        store
            .list_chapters(version_id)
            .await
            .unwrap()
            .iter()
            .all(|chapter| chapter.id != extra.id)
    );
    let bin = store.get_recycle_bin().await.unwrap();
    let bin_chapter = bin
        .items
        .iter()
        .find(|item| item.id == extra.id)
        .expect("番外应该出现在回收站里");
    assert_eq!(bin_chapter.kind, RecycleItemKind::Chapter);
    assert!(bin_chapter.restorable);
    assert!(!bin_chapter.purgeable);
    assert!(
        bin_chapter
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("字正文"))
    );

    // 正文不提供物理删除：还在回收站里时明确拒绝。
    let purge_error = store
        .purge_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(purge_error, StoreError::RecycleNotPurgeable(_)));
    assert_eq!(
        store.get_recycle_bin().await.unwrap().items.len(),
        bin.items.len()
    );

    let restored_bin = store
        .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
        .await
        .unwrap();
    assert!(restored_bin.items.iter().all(|item| item.id != extra.id));
    let body_after_restore = store.get_chapter_body(extra.id).await.unwrap().body;
    assert_eq!(body_after_restore.revision_id, Some(revision_before_delete));
    assert_eq!(body_after_restore.content, body_before_delete.content);
    let restored_chapter = store
        .list_chapters(version_id)
        .await
        .unwrap()
        .into_iter()
        .find(|chapter| chapter.id == extra.id)
        .expect("恢复后番外应该回到目录");
    assert_eq!(restored_chapter.order_index, extra.order_index);
    assert_eq!(
        restored_chapter.status,
        nekowriter_core::ChapterStatus::Approved
    );

    // 重复恢复同一条记录：明确报错，而不是假装成功。
    let restore_error = store
        .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(restore_error, StoreError::RecycleItemNotFound));

    // 祖先仍在回收站里时，章节不能被单独恢复。
    let before_ancestor = store.get_preparation_snapshot(version_id).await.unwrap();
    store
        .delete_extra_chapter(
            extra.id,
            Uuid::new_v4(),
            before_ancestor.workflow.state_revision,
        )
        .await
        .unwrap();
    let bundle = store.get_version_snapshot(version_id).await.unwrap();
    assert_ne!(bundle.project.selected_version_id, version_id);
    let deleted_version_bin = store
        .delete_version(version_id, Uuid::new_v4(), bundle.version.revision)
        .await
        .unwrap();
    let bin_version = deleted_version_bin
        .items
        .iter()
        .find(|item| item.id == version_id)
        .expect("稿本应该出现在回收站里");
    assert_eq!(bin_version.kind, RecycleItemKind::Version);
    assert!(!bin_version.purgeable);
    assert!(bin_version.restorable);
    assert!(
        bin_version
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("章已定稿")),
        "回收站应该说明稿本里还剩多少章：{:?}",
        bin_version.detail
    );
    let ancestor_error = store
        .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(
        ancestor_error,
        StoreError::RecycleAncestorDeleted(_)
    ));

    // 先恢复稿本，再恢复番外，内容与修订仍然是删除前的那一份。
    store
        .restore_recycle_item(RecycleItemKind::Version, version_id, Uuid::new_v4())
        .await
        .unwrap();
    store
        .restore_recycle_item(RecycleItemKind::Chapter, extra.id, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_chapter_body(extra.id)
            .await
            .unwrap()
            .body
            .revision_id,
        Some(revision_before_delete)
    );

    let copied_before_guidance = store
        .get_preparation_snapshot(copied.version.id)
        .await
        .unwrap();
    store
        .save_chapter_guidance(
            copied_chapters[1].id,
            Uuid::new_v4(),
            copied_before_guidance.workflow.state_revision,
            NewChapterGuidance::new("从复制稿本继续写第二章。").unwrap(),
        )
        .await
        .unwrap();
    let copied_outline_started = store
        .start_chapter_outline(
            copied_chapters[1].id,
            Uuid::new_v4(),
            store
                .get_preparation_snapshot(copied.version.id)
                .await
                .unwrap()
                .workflow
                .state_revision,
        )
        .await
        .unwrap();
    assert_eq!(
        copied_outline_started.stage,
        WorkflowStage::WritingOutlineGenerating
    );

    drop(store);
    let reopened = Store::open(data_dir.path()).await.unwrap();
    let persisted = reopened.get_preparation_snapshot(version_id).await.unwrap();
    assert_eq!(persisted.context_fold, snapshot.context_fold);
    assert_eq!(persisted.workflow.stage, WorkflowStage::Completed);
    // 恢复只把数据与在作品里的位置放回去，不把写作光标搬回来：删除时工作流已经
    // 从那一章收回到仍然可见的最后一章，之后用户可能已经在别处继续写，所以
    // 光标保持删除后的位置。
    assert_eq!(persisted.workflow.active_chapter_id, Some(chapters[1].id));
    assert_eq!(
        reopened
            .get_chapter_outline(chapters[0].id)
            .await
            .unwrap()
            .context_fold
            .as_ref()
            .unwrap()
            .plain_text,
        revised_outline_content
    );
    assert!(
        reopened
            .get_chapter_body(chapters[0].id)
            .await
            .unwrap()
            .context_fold
            .is_some()
    );
}
