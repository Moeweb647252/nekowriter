//! Setting-proposal tool (spec/05 §6): `propose_setting_change`.
//!
//! The archivist validates the target and every `find` against the **current** text before it
//! creates a `pending` proposal; the author decides later (spec/06 §5, D-013). Isolated side
//! stories cannot propose changes to the shared setting materials (spec/05 §6).

use sea_orm::EntityTrait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::character_designs;
use crate::db::json::{
    FindReplace, NewCharacter, ProposalEdits, ProposalTargetType, SettingDocKind,
};
use crate::events::ServerEvent;
use crate::services::proposals;

use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposeInput {
    target: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    edits: Option<Vec<FindReplace>>,
    #[serde(default)]
    append: Option<String>,
    #[serde(default)]
    character: Option<CharacterInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterInput {
    name: String,
    #[serde(default)]
    role_label: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    profile: String,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

/// What the proposal targets.
enum Target {
    /// One of the three setting documents.
    Doc(SettingDocKind),
    /// A character design, by id and name.
    Design(i32, String),
    /// A new character design.
    New,
}

impl Target {
    /// The label shown in results and error messages, from `shared.label`.
    fn label(&self, ctx: &ToolCtx<'_>) -> String {
        match self {
            Target::Doc(kind) => ctx.prompts.label("setting_doc", kind.as_str()),
            Target::Design(_, name) => name.clone(),
            Target::New => ctx.prompts.label("setting_doc", "characters"),
        }
    }
}

/// `propose_setting_change` (spec/05 §6).
pub struct ProposeSettingChangeTool;

#[async_trait::async_trait]
impl ToolHandler for ProposeSettingChangeTool {
    fn name(&self) -> &'static str {
        "propose_setting_change"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Canon
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("append")
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: ProposeInput = decode(input)?;
        let book_id = ctx
            .book
            .map(|book| book.id)
            .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))?;
        // Setting materials are shared: an isolated side story cannot propose changes
        // (spec/05 §6).
        if !ctx.visibility.can_write_shared() {
            return Err(ToolError::new(ToolErrorKind::IsolatedReadonly)
                .with("target", input.target.clone()));
        }

        let target = parse_target(ctx, &input.target).await?;
        let edits = choose_edits(&input)?;

        // A new character needs the character payload; the other targets do not.
        if matches!(target, Target::New) && !matches!(edits, ProposalEdits::CreateCharacter { .. })
        {
            return Err(ToolError::new(ToolErrorKind::ExactlyOne).with("fields", vec!["character"]));
        }
        if matches!(target, Target::Design(..) | Target::Doc(_))
            && matches!(edits, ProposalEdits::CreateCharacter { .. })
        {
            return Err(
                ToolError::new(ToolErrorKind::ExactlyOne).with("fields", vec!["edits", "append"])
            );
        }

        let rationale = input.rationale.trim().to_owned();
        if rationale.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["rationale must not be empty".to_owned()]));
        }

        // Every `find` must currently match exactly once in the target text (spec/05 §6).
        let doc_label = target.label(ctx);
        match &target {
            Target::Doc(kind) => {
                let doc = crate::services::setting_docs::load(ctx.tx, book_id, *kind)
                    .await
                    .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
                check_edits(&doc.content, &edits, &doc_label)?;
            }
            Target::Design(design_id, _) => {
                let design = character_designs::Entity::find_by_id(*design_id)
                    .one(ctx.tx)
                    .await
                    .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
                    .ok_or_else(|| {
                        ToolError::new(ToolErrorKind::CharacterNotFound)
                            .with("name", doc_label.clone())
                    })?;
                check_edits(&design.profile, &edits, &doc_label)?;
            }
            Target::New => {
                if let ProposalEdits::CreateCharacter { character } = &edits {
                    let existing = character_designs::Entity::find_by_book_name((
                        book_id,
                        character.name.trim().to_owned(),
                    ))
                    .one(ctx.tx)
                    .await
                    .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
                    if existing.is_some() {
                        return Err(ToolError::new(ToolErrorKind::CharacterExists)
                            .with("name", character.name.trim().to_owned()));
                    }
                }
            }
        }

        let (target_type, target_kind, target_id) = match &target {
            Target::Doc(kind) => (ProposalTargetType::SettingDoc, Some(*kind), None),
            Target::Design(id, _) => (ProposalTargetType::CharacterDesign, None, Some(*id)),
            Target::New => (ProposalTargetType::NewCharacter, None, None),
        };
        let row = proposals::create_inner(
            ctx.tx,
            proposals::NewProposal {
                book_id,
                target_type,
                target_kind,
                target_id,
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
            status: crate::db::json::ProposalStatus::Pending,
        });

        let text = ctx
            .prompts
            .render_value(
                "result.canon_write",
                crate::prompts::template_value(&json!({
                    "action": "propose_setting_change",
                    "target": doc_label,
                    // The template also tests `moved` and the position fields for every action;
                    // strict-undefined rendering rejects a missing variable.
                    "count": Value::Null,
                    "prev_event": Value::Null,
                    "next_event": Value::Null,
                    "moved": false,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text).with_ui(json!({
            "proposal_id": row.id,
            "target": doc_label,
        })))
    }
}

/// Parses the `target` parameter (spec/05 §6).
async fn parse_target(ctx: &ToolCtx<'_>, target: &str) -> Result<Target, ToolError> {
    let target = target.trim();
    match target {
        "outline" => Ok(Target::Doc(SettingDocKind::Outline)),
        "world" => Ok(Target::Doc(SettingDocKind::World)),
        "protagonist" => Ok(Target::Doc(SettingDocKind::Protagonist)),
        "new_character" => Ok(Target::New),
        other => {
            let Some(name) = other.strip_prefix("character:") else {
                return Err(ToolError::new(ToolErrorKind::Validation).with(
                    "errors",
                    vec![format!(
                        "unknown target {other:?}; use outline, world, protagonist, character:<name> or new_character"
                    )],
                ));
            };
            let name = name.trim();
            let book_id = ctx
                .book
                .map(|book| book.id)
                .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))?;
            let row = character_designs::Entity::find_by_book_name((book_id, name.to_owned()))
                .one(ctx.tx)
                .await
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
            match row {
                Some(row) => Ok(Target::Design(row.id, row.name)),
                None => Err(ToolError::new(ToolErrorKind::CharacterNotFound).with("name", name)),
            }
        }
    }
}

/// Exactly one of `edits` / `append` / `character` (spec/05 §6).
fn choose_edits(input: &ProposeInput) -> Result<ProposalEdits, ToolError> {
    let edits = input
        .edits
        .as_ref()
        .filter(|edits| !edits.is_empty())
        .map(|edits| ProposalEdits::Edits {
            edits: edits.clone(),
        });
    let append = input
        .append
        .as_ref()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| ProposalEdits::Append {
            text: text.to_owned(),
        });
    let character = input
        .character
        .as_ref()
        .map(|character| ProposalEdits::CreateCharacter {
            character: NewCharacter {
                name: character.name.trim().to_owned(),
                role_label: character.role_label.trim().to_owned(),
                summary: character.summary.trim().to_owned(),
                profile: character.profile.clone(),
            },
        });
    let provided: Vec<&str> = [
        edits.is_some().then_some("edits"),
        append.is_some().then_some("append"),
        character.is_some().then_some("character"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if provided.len() != 1 {
        return Err(ToolError::new(ToolErrorKind::ExactlyOne)
            .with("fields", vec!["edits", "append", "character"]));
    }
    Ok(edits
        .or(append)
        .or(character)
        .unwrap_or(ProposalEdits::Append {
            text: String::new(),
        }))
}

/// Checks every `find` against the current text, exactly once each (spec/05 §6).
fn check_edits(text: &str, edits: &ProposalEdits, doc_label: &str) -> Result<(), ToolError> {
    if let ProposalEdits::Edits { edits } = edits {
        // The edits apply in order to the evolving text, so the check replays them.
        let mut current = text.to_owned();
        for edit in edits {
            let count = current.matches(&edit.find).count();
            if count != 1 || edit.find.is_empty() {
                return Err(ToolError::new(ToolErrorKind::FindFailed)
                    .with("doc_label", doc_label.to_owned())
                    .with("find", edit.find.clone())
                    .with("count", count));
            }
            current = current.replacen(&edit.find, &edit.replace, 1);
        }
    }
    Ok(())
}

/// The archivist role's proposal handler (spec/05 §2).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![std::sync::Arc::new(ProposeSettingChangeTool)]
}
