//! Knowledge spaces and entries (`docs/product-spec.md` §12.2). They work without any
//! embedding configuration; indexing and semantic search build on them.

use std::collections::HashMap;

use nekowriter_core::{
    conflict::{Resource, StateConflict},
    event::Event,
    ids::{CommandId, KnowledgeEntryId, KnowledgeSpaceId},
    knowledge::{EntryContent, EntryTitle, Keywords, SpaceName},
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DerivePartialModel, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    command::{Begun, CommandTx},
    entity::{knowledge_entry, knowledge_space},
    projects::check_revision,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpaceView {
    pub id: KnowledgeSpaceId,
    pub name: String,
    pub entry_count: u32,
    pub revision: i64,
    pub updated_at: Timestamp,
}

/// An entry without its content, for lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntrySummary {
    pub id: KnowledgeEntryId,
    pub space_id: KnowledgeSpaceId,
    pub title: String,
    pub keywords: Vec<String>,
    pub revision: i64,
    pub updated_at: Timestamp,
}

/// The columns of an entry that lists need; the content is left in the database.
#[derive(Debug, DerivePartialModel)]
#[sea_orm(entity = "knowledge_entry::Entity")]
struct EntryRow {
    id: KnowledgeEntryId,
    space_id: KnowledgeSpaceId,
    title: EntryTitle,
    keywords: Keywords,
    revision: i64,
    updated_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntryView {
    pub id: KnowledgeEntryId,
    pub space_id: KnowledgeSpaceId,
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub revision: i64,
    pub updated_at: Timestamp,
}

impl From<knowledge_entry::Model> for EntryView {
    fn from(entry: knowledge_entry::Model) -> Self {
        Self {
            id: entry.id,
            space_id: entry.space_id,
            title: entry.title.as_str().to_owned(),
            keywords: entry.keywords.as_slice().to_vec(),
            content: entry.content,
            revision: entry.revision,
            updated_at: entry.updated_at,
        }
    }
}

/// An entry's content as the user saves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewEntry {
    pub space_id: KnowledgeSpaceId,
    pub title: EntryTitle,
    pub keywords: Keywords,
    pub content: EntryContent,
}

impl Store {
    /// The live spaces by name, with how many live entries each holds.
    pub async fn list_spaces(&self) -> Result<Vec<SpaceView>, StoreError> {
        let spaces = knowledge_space::Entity::find()
            .filter(knowledge_space::Column::DeletedAt.is_null())
            .order_by_asc(knowledge_space::Column::Name)
            .all(&self.db)
            .await?;
        let counts: HashMap<KnowledgeSpaceId, i64> = live_entries()
            .select_only()
            .column(knowledge_entry::Column::SpaceId)
            .column_as(knowledge_entry::Column::Id.count(), "count")
            .group_by(knowledge_entry::Column::SpaceId)
            .into_tuple()
            .all(&self.db)
            .await?
            .into_iter()
            .collect();
        Ok(spaces
            .into_iter()
            .map(|space| {
                let count = counts.get(&space.id).copied().unwrap_or_default();
                space_view(&space, u32::try_from(count).unwrap_or(u32::MAX))
            })
            .collect())
    }

    pub async fn create_space(
        &self,
        command: CommandId,
        name: SpaceName,
    ) -> Result<SpaceView, StoreError> {
        let mut tx = match self.begin_command(command, "space.create", &name).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        check_name_free(tx.db(), &name, None).await?;
        let now = tx.now();
        let space = knowledge_space::ActiveModel {
            id: Set(KnowledgeSpaceId::generate()),
            name: Set(name),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(tx.db())
        .await?;
        tx.emit(Event::KnowledgeChanged);
        tx.commit(space_view(&space, 0)).await
    }

    pub async fn rename_space(
        &self,
        command: CommandId,
        id: KnowledgeSpaceId,
        expected_revision: i64,
        name: SpaceName,
    ) -> Result<SpaceView, StoreError> {
        let input = (id, expected_revision, &name);
        let tx = match self.begin_command(command, "space.rename", &input).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let space = live_space(tx.db(), id).await?;
        check_revision(Resource::Space, expected_revision, space.revision)?;
        check_name_free(tx.db(), &name, Some(id)).await?;
        change_space(tx, space, |model, _| model.name = Set(name)).await
    }

    /// Deletes an empty space.
    pub async fn delete_space(
        &self,
        command: CommandId,
        id: KnowledgeSpaceId,
        expected_revision: i64,
    ) -> Result<SpaceView, StoreError> {
        let input = (id, expected_revision);
        let tx = match self.begin_command(command, "space.delete", &input).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let space = live_space(tx.db(), id).await?;
        check_revision(Resource::Space, expected_revision, space.revision)?;
        let entries = live_entries()
            .filter(knowledge_entry::Column::SpaceId.eq(id))
            .count(tx.db())
            .await?;
        if entries > 0 {
            return Err(StoreError::StateConflict(StateConflict::SpaceNotEmpty));
        }
        change_space(tx, space, |model, now| model.deleted_at = Set(Some(now))).await
    }

    /// Live entries by title: of one space or all, and whose title or a keyword contains
    /// `filter`, ignoring case.
    pub async fn list_entries(
        &self,
        space: Option<KnowledgeSpaceId>,
        filter: &str,
    ) -> Result<Vec<EntrySummary>, StoreError> {
        let mut query = live_entries().order_by_asc(knowledge_entry::Column::Title);
        if let Some(space) = space {
            query = query.filter(knowledge_entry::Column::SpaceId.eq(space));
        }
        let needle = filter.trim().to_lowercase();
        Ok(query
            .into_partial_model::<EntryRow>()
            .all(&self.db)
            .await?
            .into_iter()
            .filter(|entry| {
                needle.is_empty()
                    || entry.title.as_str().to_lowercase().contains(&needle)
                    || entry
                        .keywords
                        .as_slice()
                        .iter()
                        .any(|keyword| keyword.to_lowercase().contains(&needle))
            })
            .map(|entry| EntrySummary {
                id: entry.id,
                space_id: entry.space_id,
                title: entry.title.as_str().to_owned(),
                keywords: entry.keywords.as_slice().to_vec(),
                revision: entry.revision,
                updated_at: entry.updated_at,
            })
            .collect())
    }

    pub async fn entry(&self, id: KnowledgeEntryId) -> Result<EntryView, StoreError> {
        Ok(live_entry(&self.db, id).await?.into())
    }

    pub async fn create_entry(
        &self,
        command: CommandId,
        new: NewEntry,
    ) -> Result<EntryView, StoreError> {
        let mut tx = match self.begin_command(command, "entry.create", &new).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        live_space(tx.db(), new.space_id).await?;
        let now = tx.now();
        let entry = knowledge_entry::ActiveModel {
            id: Set(KnowledgeEntryId::generate()),
            space_id: Set(new.space_id),
            title: Set(new.title),
            keywords: Set(new.keywords),
            content: Set(new.content.as_str().to_owned()),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(tx.db())
        .await?;
        tx.emit(Event::KnowledgeChanged);
        tx.commit(entry.into()).await
    }

    /// Replaces an entry's space, title, keywords and content.
    pub async fn update_entry(
        &self,
        command: CommandId,
        id: KnowledgeEntryId,
        expected_revision: i64,
        new: NewEntry,
    ) -> Result<EntryView, StoreError> {
        let input = (id, expected_revision, &new);
        let tx = match self.begin_command(command, "entry.update", &input).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let entry = live_entry(tx.db(), id).await?;
        check_revision(Resource::Entry, expected_revision, entry.revision)?;
        live_space(tx.db(), new.space_id).await?;
        change_entry(tx, entry, |model, _| {
            model.space_id = Set(new.space_id);
            model.title = Set(new.title);
            model.keywords = Set(new.keywords);
            model.content = Set(new.content.as_str().to_owned());
        })
        .await
    }

    pub async fn delete_entry(
        &self,
        command: CommandId,
        id: KnowledgeEntryId,
        expected_revision: i64,
    ) -> Result<EntryView, StoreError> {
        let input = (id, expected_revision);
        let tx = match self.begin_command(command, "entry.delete", &input).await? {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let entry = live_entry(tx.db(), id).await?;
        check_revision(Resource::Entry, expected_revision, entry.revision)?;
        change_entry(tx, entry, |model, now| model.deleted_at = Set(Some(now))).await
    }
}

fn live_entries() -> sea_orm::Select<knowledge_entry::Entity> {
    knowledge_entry::Entity::find().filter(knowledge_entry::Column::DeletedAt.is_null())
}

async fn live_space<C: ConnectionTrait>(
    db: &C,
    id: KnowledgeSpaceId,
) -> Result<knowledge_space::Model, StoreError> {
    knowledge_space::Entity::find_by_id(id)
        .filter(knowledge_space::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Space))
}

async fn live_entry<C: ConnectionTrait>(
    db: &C,
    id: KnowledgeEntryId,
) -> Result<knowledge_entry::Model, StoreError> {
    live_entries()
        .filter(knowledge_entry::Column::Id.eq(id))
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Entry))
}

/// Refuses a name another live space has.
async fn check_name_free<C: ConnectionTrait>(
    db: &C,
    name: &SpaceName,
    renamed: Option<KnowledgeSpaceId>,
) -> Result<(), StoreError> {
    let taken = knowledge_space::Entity::find()
        .filter(knowledge_space::Column::DeletedAt.is_null())
        .filter(knowledge_space::Column::Name.eq(name.clone()))
        .all(db)
        .await?
        .into_iter()
        .any(|space| Some(space.id) != renamed);
    if taken {
        return Err(StoreError::StateConflict(StateConflict::SpaceNameTaken));
    }
    Ok(())
}

fn space_view(space: &knowledge_space::Model, entry_count: u32) -> SpaceView {
    SpaceView {
        id: space.id,
        name: space.name.as_str().to_owned(),
        entry_count,
        revision: space.revision,
        updated_at: space.updated_at,
    }
}

async fn change_space(
    mut tx: CommandTx,
    space: knowledge_space::Model,
    apply: impl FnOnce(&mut knowledge_space::ActiveModel, Timestamp),
) -> Result<SpaceView, StoreError> {
    let now = tx.now();
    let revision = space.revision + 1;
    let mut model = space.into_active_model();
    apply(&mut model, now);
    model.revision = Set(revision);
    model.updated_at = Set(now);
    let space = model.update(tx.db()).await?;
    let entries = live_entries()
        .filter(knowledge_entry::Column::SpaceId.eq(space.id))
        .count(tx.db())
        .await?;
    tx.emit(Event::KnowledgeChanged);
    tx.commit(space_view(
        &space,
        u32::try_from(entries).unwrap_or(u32::MAX),
    ))
    .await
}

async fn change_entry(
    mut tx: CommandTx,
    entry: knowledge_entry::Model,
    apply: impl FnOnce(&mut knowledge_entry::ActiveModel, Timestamp),
) -> Result<EntryView, StoreError> {
    let now = tx.now();
    let revision = entry.revision + 1;
    let mut model = entry.into_active_model();
    apply(&mut model, now);
    model.revision = Set(revision);
    model.updated_at = Set(now);
    let entry = model.update(tx.db()).await?;
    tx.emit(Event::KnowledgeChanged);
    tx.commit(entry.into()).await
}
