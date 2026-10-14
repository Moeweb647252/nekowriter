//! Knowledge spaces and entries (`docs/product-spec.md` §12.2).

use axum::{extract::State, http::StatusCode};
use nekowriter_core::{
    ids::{CommandId, KnowledgeEntryId, KnowledgeSpaceId},
    knowledge::{EntryContent, EntryTitle, Keywords, SpaceName},
};
use nekowriter_store::{EntrySummary, EntryView, NewEntry, SpaceView};
use serde::Deserialize;
use ts_rs::TS;

use crate::http::{
    AppState,
    error::ApiError,
    extract::{Json, Path, Query},
    handlers::RevisionRequest,
};

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateSpaceRequest {
    pub command_id: CommandId,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameSpaceRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    pub name: String,
}

/// An entry as the user saves it.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntryFields {
    pub space_id: KnowledgeSpaceId,
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateEntryRequest {
    pub command_id: CommandId,
    #[serde(flatten)]
    #[ts(flatten)]
    pub entry: EntryFields,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateEntryRequest {
    pub command_id: CommandId,
    pub expected_revision: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub entry: EntryFields,
}

impl EntryFields {
    fn parse(&self) -> Result<NewEntry, ApiError> {
        Ok(NewEntry {
            space_id: self.space_id,
            title: EntryTitle::parse(&self.title)?,
            keywords: Keywords::parse(&self.keywords)?,
            content: EntryContent::parse(&self.content)?,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct EntriesParams {
    space: Option<KnowledgeSpaceId>,
    /// Text that the title or a keyword contains.
    #[serde(default)]
    filter: String,
}

pub(crate) async fn spaces(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<SpaceView>>, ApiError> {
    Ok(axum::Json(state.store.list_spaces().await?))
}

pub(crate) async fn create_space(
    State(state): State<AppState>,
    Json(request): Json<CreateSpaceRequest>,
) -> Result<(StatusCode, axum::Json<SpaceView>), ApiError> {
    let name = SpaceName::parse(&request.name)?;
    let space = state.store.create_space(request.command_id, name).await?;
    Ok((StatusCode::CREATED, axum::Json(space)))
}

pub(crate) async fn rename_space(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeSpaceId>,
    Json(request): Json<RenameSpaceRequest>,
) -> Result<axum::Json<SpaceView>, ApiError> {
    let name = SpaceName::parse(&request.name)?;
    Ok(axum::Json(
        state
            .store
            .rename_space(request.command_id, id, request.expected_revision, name)
            .await?,
    ))
}

pub(crate) async fn delete_space(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeSpaceId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<SpaceView>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .delete_space(request.command_id, id, request.expected_revision)
            .await?,
    ))
}

pub(crate) async fn entries(
    State(state): State<AppState>,
    Query(params): Query<EntriesParams>,
) -> Result<axum::Json<Vec<EntrySummary>>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .list_entries(params.space, &params.filter)
            .await?,
    ))
}

pub(crate) async fn entry(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeEntryId>,
) -> Result<axum::Json<EntryView>, ApiError> {
    Ok(axum::Json(state.store.entry(id).await?))
}

pub(crate) async fn create_entry(
    State(state): State<AppState>,
    Json(request): Json<CreateEntryRequest>,
) -> Result<(StatusCode, axum::Json<EntryView>), ApiError> {
    let entry = request.entry.parse()?;
    let entry = state.store.create_entry(request.command_id, entry).await?;
    Ok((StatusCode::CREATED, axum::Json(entry)))
}

pub(crate) async fn update_entry(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeEntryId>,
    Json(request): Json<UpdateEntryRequest>,
) -> Result<axum::Json<EntryView>, ApiError> {
    let entry = request.entry.parse()?;
    Ok(axum::Json(
        state
            .store
            .update_entry(request.command_id, id, request.expected_revision, entry)
            .await?,
    ))
}

pub(crate) async fn delete_entry(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeEntryId>,
    Json(request): Json<RevisionRequest>,
) -> Result<axum::Json<EntryView>, ApiError> {
    Ok(axum::Json(
        state
            .store
            .delete_entry(request.command_id, id, request.expected_revision)
            .await?,
    ))
}
