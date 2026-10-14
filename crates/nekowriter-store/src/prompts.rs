//! Overrides of the built-in prompts (`docs/product-spec.md` §13.1). The effective prompt
//! is the project's override, then the system's, then the built-in text, which lives in
//! the agent crate.

use nekowriter_core::{
    conflict::Resource,
    event::Event,
    ids::{CommandId, ProjectId},
    prompt::{PromptContent, PromptKey, PromptRole},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    command::Begun,
    entity::{
        project_prompt, system_prompt,
        system_settings::{self, ROW_ID},
    },
    projects::{active_project, check_revision},
};

/// The overrides in force for one project, or for none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptOverrides {
    system: Vec<StoredPrompt>,
    project: Vec<StoredPrompt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredPrompt {
    key: PromptKey,
    role: PromptRole,
    content: String,
}

impl PromptOverrides {
    #[must_use]
    pub fn system(&self, key: PromptKey, role: PromptRole) -> Option<&str> {
        find(&self.system, key, role)
    }

    #[must_use]
    pub fn project(&self, key: PromptKey, role: PromptRole) -> Option<&str> {
        find(&self.project, key, role)
    }

    /// The override in effect: the project's, then the system's; `None` uses the built-in
    /// text.
    #[must_use]
    pub fn effective(&self, key: PromptKey, role: PromptRole) -> Option<&str> {
        self.project(key, role).or_else(|| self.system(key, role))
    }
}

fn find(prompts: &[StoredPrompt], key: PromptKey, role: PromptRole) -> Option<&str> {
    prompts
        .iter()
        .find(|prompt| prompt.key == key && prompt.role == role)
        .map(|prompt| prompt.content.as_str())
}

/// The overrides in force at a scope, and the revision a save at that scope quotes: the
/// system settings' for the system, the project's for a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptsRead {
    pub revision: i64,
    pub overrides: PromptOverrides,
}

/// The revision of the settings or project after saving an override.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptSaved {
    pub revision: i64,
}

impl Store {
    /// The system overrides and, with a project, that project's, with the revision a save
    /// at that scope quotes.
    pub async fn prompts(&self, project: Option<ProjectId>) -> Result<PromptsRead, StoreError> {
        let revision = match project {
            Some(id) => active_project(&self.db, id).await?.revision,
            None => system_settings::Entity::find_by_id(ROW_ID)
                .one(&self.db)
                .await?
                .map_or(0, |row| row.revision),
        };
        Ok(PromptsRead {
            revision,
            overrides: prompt_overrides(&self.db, project).await?,
        })
    }

    /// Sets a system override, or deletes it with `None`.
    pub async fn save_system_prompt(
        &self,
        command: CommandId,
        expected_revision: i64,
        key: PromptKey,
        role: PromptRole,
        content: Option<PromptContent>,
    ) -> Result<PromptSaved, StoreError> {
        let input = (expected_revision, key, role, &content);
        let mut tx = match self
            .begin_command(command, "prompt.save_system", &input)
            .await?
        {
            Begun::Replayed(saved) => return Ok(saved),
            Begun::Fresh(tx) => tx,
        };
        let existing = system_settings::Entity::find_by_id(ROW_ID)
            .one(tx.db())
            .await?;
        let revision = existing.as_ref().map_or(0, |row| row.revision);
        check_revision(Resource::Settings, expected_revision, revision)?;
        let now = tx.now();
        // The settings revision covers the prompts too, so concurrent edits are detected.
        match existing {
            Some(row) => {
                let mut row = row.into_active_model();
                row.revision = Set(revision + 1);
                row.updated_at = Set(now);
                row.update(tx.db()).await?;
            }
            None => {
                system_settings::ActiveModel {
                    id: Set(ROW_ID),
                    api_base_url: Set(None),
                    api_key: Set(None),
                    default_model: Set(None),
                    revision: Set(1),
                    updated_at: Set(now),
                }
                .insert(tx.db())
                .await?;
            }
        }
        system_prompt::Entity::delete_by_id((key, role))
            .exec(tx.db())
            .await?;
        if let Some(content) = content {
            system_prompt::ActiveModel {
                key: Set(key),
                role: Set(role),
                content: Set(content.as_str().to_owned()),
                updated_at: Set(now),
            }
            .insert(tx.db())
            .await?;
        }
        tx.emit(Event::SettingsChanged);
        tx.commit(PromptSaved {
            revision: revision + 1,
        })
        .await
    }

    /// Sets a project's override, or deletes it with `None`.
    pub async fn save_project_prompt(
        &self,
        command: CommandId,
        project_id: ProjectId,
        expected_revision: i64,
        key: PromptKey,
        role: PromptRole,
        content: Option<PromptContent>,
    ) -> Result<PromptSaved, StoreError> {
        let input = (project_id, expected_revision, key, role, &content);
        let mut tx = match self
            .begin_command(command, "prompt.save_project", &input)
            .await?
        {
            Begun::Replayed(saved) => return Ok(saved),
            Begun::Fresh(tx) => tx,
        };
        let project = active_project(tx.db(), project_id).await?;
        check_revision(Resource::Project, expected_revision, project.revision)?;
        let now = tx.now();
        let revision = project.revision + 1;
        let mut project = project.into_active_model();
        project.revision = Set(revision);
        project.updated_at = Set(now);
        project.update(tx.db()).await?;
        project_prompt::Entity::delete_by_id((project_id, key, role))
            .exec(tx.db())
            .await?;
        if let Some(content) = content {
            project_prompt::ActiveModel {
                project_id: Set(project_id),
                key: Set(key),
                role: Set(role),
                content: Set(content.as_str().to_owned()),
                updated_at: Set(now),
            }
            .insert(tx.db())
            .await?;
        }
        tx.emit(Event::ProjectChanged { project_id });
        tx.commit(PromptSaved { revision }).await
    }
}

/// The overrides in force for `project`, or the system's alone.
pub(crate) async fn prompt_overrides<C: ConnectionTrait>(
    db: &C,
    project: Option<ProjectId>,
) -> Result<PromptOverrides, StoreError> {
    let system = system_prompt::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|row| StoredPrompt {
            key: row.key,
            role: row.role,
            content: row.content,
        })
        .collect();
    let project = match project {
        Some(id) => project_prompt::Entity::find()
            .filter(project_prompt::Column::ProjectId.eq(id))
            .all(db)
            .await?
            .into_iter()
            .map(|row| StoredPrompt {
                key: row.key,
                role: row.role,
                content: row.content,
            })
            .collect(),
        None => Vec::new(),
    };
    Ok(PromptOverrides { system, project })
}
