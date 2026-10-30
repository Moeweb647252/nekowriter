//! `propose_goal_change` (spec/05 §6, spec/06 §5, plan §4.6).
//!
//! The archivist proposes; the author decides. A proposal is a row in `setting_proposals` with a
//! goal target, and acceptance is applied by `services::proposals` through the goals service.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::{
    entities, foreshadowing, goal_nodes, setting_proposals, timeline_events,
};
use crate::db::json::{
    GoalLevel, GoalStatus, NewGoalTask, ProposalEdits, ProposalStatus, ProposalTargetType,
};
use crate::events::ServerEvent;
use crate::services::canon::Visibility as CanonVisibility;
use crate::services::proposals;

use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposeGoalInput {
    action: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    node: Option<i32>,
    #[serde(default)]
    parent: Option<i32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    chapter_from: Option<i32>,
    #[serde(default)]
    chapter_to: Option<i32>,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    events: Vec<i32>,
    #[serde(default)]
    foreshadowing: Vec<i32>,
}

/// `propose_goal_change` (spec/05 §6).
pub struct ProposeGoalChangeTool;

#[async_trait::async_trait]
impl ToolHandler for ProposeGoalChangeTool {
    fn name(&self) -> &'static str {
        "propose_goal_change"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ProposeGoalInput = serde_json::from_value(input).map_err(|err| {
            ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
        })?;
        let book_id = ctx
            .book
            .map(|book| book.id)
            .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))?;
        let rationale = input.rationale.trim().to_owned();
        if rationale.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["rationale must not be empty".to_owned()]));
        }
        let visibility = CanonVisibility::of_session(book_id, ctx.visibility);

        match input.action.as_str() {
            "complete" | "drop" => {
                let node_id = input.node.ok_or_else(|| {
                    ToolError::new(ToolErrorKind::Validation).with(
                        "errors",
                        vec!["node is required for complete/drop".to_owned()],
                    )
                })?;
                let node = load_visible_node(ctx, visibility, book_id, node_id).await?;
                if !ctx.visibility.can_write_shared() && node.track_id.is_none() {
                    return Err(ToolError::new(ToolErrorKind::IsolatedReadonly)
                        .with("target", node.title.clone()));
                }
                if !matches!(node.status, GoalStatus::Active | GoalStatus::Waiting) {
                    return Err(ToolError::new(ToolErrorKind::Validation)
                        .with("errors", vec!["the node is not open".to_owned()]));
                }
                let status = if input.action == "complete" {
                    GoalStatus::Done
                } else {
                    GoalStatus::Dropped
                };
                let edits = ProposalEdits::SetGoalStatus { status };
                if let Some(existing) = pending_status_proposal(ctx, node.id, status).await? {
                    return Ok(result(
                        ctx,
                        existing.id,
                        &input.action,
                        Some(node.id),
                        &node.title,
                    ));
                }
                let row = proposals::create_inner(
                    ctx.tx,
                    proposals::NewProposal {
                        book_id,
                        target_type: ProposalTargetType::GoalNode,
                        target_kind: None,
                        target_id: Some(node.id),
                        edits: &edits,
                        rationale: &rationale,
                        chapter_id: ctx.chapter.map(|chapter| chapter.id),
                        session_id: Some(ctx.session.id),
                    },
                )
                .await
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
                ctx.emit(ServerEvent::ProposalChanged {
                    book_id,
                    proposal_id: row.id,
                    status: ProposalStatus::Pending,
                });
                Ok(result(
                    ctx,
                    row.id,
                    &input.action,
                    Some(node.id),
                    &node.title,
                ))
            }
            "create" => {
                let title = clean_text(input.title.as_deref().unwrap_or_default(), "title", 120)?;
                let purpose = clean_text(
                    input.purpose.as_deref().unwrap_or_default(),
                    "purpose",
                    1000,
                )?;
                if input.chapter_from.is_some_and(|chapter| chapter < 1)
                    || input.chapter_to.is_some_and(|chapter| chapter < 1)
                {
                    return Err(ToolError::new(ToolErrorKind::Validation)
                        .with("errors", vec!["chapter numbers start at 1".to_owned()]));
                }
                if let (Some(from), Some(to)) = (input.chapter_from, input.chapter_to)
                    && to < from
                {
                    return Err(ToolError::new(ToolErrorKind::Validation).with(
                        "errors",
                        vec!["chapter_to must not be before chapter_from".to_owned()],
                    ));
                }
                let track_id = visibility.new_row_track();
                if let Some(parent_id) = input.parent {
                    let parent = load_visible_node(ctx, visibility, book_id, parent_id).await?;
                    if parent.level != GoalLevel::Plan
                        || parent.status != GoalStatus::Active
                        || parent.track_id != track_id
                    {
                        return Err(ToolError::new(ToolErrorKind::Validation).with(
                            "errors",
                            vec!["parent must be an open plan in scope".to_owned()],
                        ));
                    }
                }
                let entity_ids =
                    resolve_entity_names(ctx, visibility, book_id, &input.entities).await?;
                validate_link_ids(ctx, visibility, book_id, &input.events, Link::Event).await?;
                validate_link_ids(
                    ctx,
                    visibility,
                    book_id,
                    &input.foreshadowing,
                    Link::Foreshadowing,
                )
                .await?;
                let task = NewGoalTask {
                    parent_id: input.parent,
                    title,
                    purpose,
                    chapter_from: input.chapter_from,
                    chapter_to: input.chapter_to,
                    entity_ids,
                    event_ids: input.events,
                    foreshadowing_ids: input.foreshadowing,
                    track_id,
                };
                let edits = ProposalEdits::CreateGoalTask { task };
                let row = proposals::create_inner(
                    ctx.tx,
                    proposals::NewProposal {
                        book_id,
                        target_type: ProposalTargetType::NewGoalTask,
                        target_kind: None,
                        target_id: input.parent,
                        edits: &edits,
                        rationale: &rationale,
                        chapter_id: ctx.chapter.map(|chapter| chapter.id),
                        session_id: Some(ctx.session.id),
                    },
                )
                .await
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
                ctx.emit(ServerEvent::ProposalChanged {
                    book_id,
                    proposal_id: row.id,
                    status: ProposalStatus::Pending,
                });
                Ok(result(ctx, row.id, "create", None, "new task"))
            }
            other => Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec![format!("unknown action {other:?}")])),
        }
    }
}

fn result(
    ctx: &ToolCtx<'_>,
    proposal_id: i32,
    action: &str,
    node_id: Option<i32>,
    target: &str,
) -> ToolOutcome {
    let text = ctx
        .prompts
        .render_value(
            "result.canon_write",
            crate::prompts::template_value(&json!({
                "action": "propose_goal_change",
                "target": target,
                "count": Value::Null,
                "prev_event": Value::Null,
                "next_event": Value::Null,
                "moved": false,
            })),
        )
        .map(|rendered| rendered.text)
        .unwrap_or_default();
    ToolOutcome::text(text).with_ui(json!({
        "proposal_id": proposal_id,
        "action": action,
        "node_id": node_id,
    }))
}

fn clean_text(value: &str, field: &str, max: usize) -> Result<String, ToolError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ToolError::new(ToolErrorKind::Validation)
            .with("errors", vec![format!("{field} must not be empty")]));
    }
    if value.chars().count() > max {
        return Err(ToolError::new(ToolErrorKind::Validation)
            .with("errors", vec![format!("{field} is too long")]));
    }
    Ok(value.to_owned())
}

async fn load_visible_node(
    ctx: &ToolCtx<'_>,
    visibility: CanonVisibility,
    book_id: i32,
    node_id: i32,
) -> Result<goal_nodes::Model, ToolError> {
    let node = goal_nodes::Entity::find_by_id(node_id)
        .one(ctx.tx)
        .await
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
        .ok_or_else(|| {
            ToolError::new(ToolErrorKind::NotFound).with("target", format!("goal #{node_id}"))
        })?;
    if node.book_id != book_id || !visibility.sees(node.track_id) {
        return Err(
            ToolError::new(ToolErrorKind::NotFound).with("target", format!("goal #{node_id}"))
        );
    }
    Ok(node)
}

async fn pending_status_proposal(
    ctx: &ToolCtx<'_>,
    node_id: i32,
    status: GoalStatus,
) -> Result<Option<setting_proposals::Model>, ToolError> {
    let rows = setting_proposals::Entity::find()
        .filter(
            setting_proposals::COLUMN
                .target_type
                .eq(ProposalTargetType::GoalNode),
        )
        .filter(setting_proposals::COLUMN.target_id.eq(node_id))
        .filter(setting_proposals::COLUMN.status.eq(ProposalStatus::Pending))
        .order_by_desc(setting_proposals::COLUMN.id)
        .all(ctx.tx)
        .await
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
    for row in rows {
        if let ProposalEdits::SetGoalStatus { status: existing } = row.edits.clone().into_inner()
            && existing == status
        {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

async fn resolve_entity_names(
    ctx: &ToolCtx<'_>,
    visibility: CanonVisibility,
    book_id: i32,
    names: &[String],
) -> Result<Vec<i32>, ToolError> {
    let rows = entities::Entity::find()
        .filter(entities::COLUMN.book_id.eq(book_id))
        .all(ctx.tx)
        .await
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
    let mut ids = Vec::new();
    for name in names {
        let name = name.trim();
        let matches: Vec<&entities::Model> = rows
            .iter()
            .filter(|row| visibility.sees(row.track_id))
            .filter(|row| row.name == name || row.aliases.0.iter().any(|alias| alias == name))
            .collect();
        match matches.as_slice() {
            [row] => ids.push(row.id),
            [] => {
                return Err(
                    ToolError::new(ToolErrorKind::EntityNotFound).with("name", name.to_owned())
                );
            }
            _ => {
                return Err(
                    ToolError::new(ToolErrorKind::EntityAmbiguous).with("name", name.to_owned())
                );
            }
        }
    }
    Ok(ids)
}

enum Link {
    Event,
    Foreshadowing,
}

async fn validate_link_ids(
    ctx: &ToolCtx<'_>,
    visibility: CanonVisibility,
    book_id: i32,
    ids: &[i32],
    kind: Link,
) -> Result<(), ToolError> {
    for id in ids {
        let track = match kind {
            Link::Event => timeline_events::Entity::find_by_id(*id)
                .one(ctx.tx)
                .await
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
                .filter(|row| row.book_id == book_id)
                .map(|row| row.track_id),
            Link::Foreshadowing => foreshadowing::Entity::find_by_id(*id)
                .one(ctx.tx)
                .await
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
                .filter(|row| row.book_id == book_id)
                .map(|row| row.track_id),
        };
        let Some(track) = track else {
            return Err(ToolError::new(ToolErrorKind::NotFound).with("target", format!("#{id}")));
        };
        if !visibility.sees(track) {
            return Err(ToolError::new(ToolErrorKind::NotFound).with("target", format!("#{id}")));
        }
    }
    Ok(())
}

/// The archivist role's goal-proposal handler (spec/05 §2).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![std::sync::Arc::new(ProposeGoalChangeTool)]
}
