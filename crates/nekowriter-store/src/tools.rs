//! Tool calls the application carries out between rounds (`docs/product-spec.md` §8.3,
//! §12.1): knowledge searches, which need the network and so cannot run inside the
//! transaction that records the model's answer.
//!
//! The call waits as `executing` while its run is queued. Before the run's next request,
//! the dispatcher begins the call: unavailable search, an invalid query, missing spaces
//! and an empty budget resolve it at once without a charge; otherwise the call is charged
//! once to its budget scope and the dispatcher searches. A call interrupted after its
//! charge is carried out again without a second charge.

use nekowriter_core::{
    call::CallRequest,
    field::FieldError,
    ids::{KnowledgeSpaceId, RunId, ToolCallId},
    knowledge::{SearchQuery, TopK},
    run::{RunKind, RunStatus},
    run_message::ToolOutcome,
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};

use crate::{
    Store, StoreError,
    calls::{requeue, resolve},
    command::WriteTx,
    entity::{
        budget_charge, knowledge_space, project, run,
        tool_call::{self, ToolCallStatus},
    },
    search::search_unavailable,
};

/// A knowledge search to carry out for a call, charged to its scope.
#[derive(Clone, Debug)]
pub struct KnowledgeSearch {
    pub call_id: ToolCallId,
    pub query: SearchQuery,
    /// All spaces when empty.
    pub spaces: Vec<KnowledgeSpaceId>,
    pub top_k: TopK,
    /// What the scope has left after this call.
    pub remaining_budget: u32,
}

impl Store {
    /// Begins the run's executing call, if it has one: resolves it at once when it cannot
    /// run, or charges it and returns the search to carry out.
    pub async fn begin_tool(&self, run_id: RunId) -> Result<Option<KnowledgeSearch>, StoreError> {
        let mut tx = self.begin_internal().await?;
        let Some(call) = tool_call::Entity::find()
            .filter(tool_call::Column::RunId.eq(run_id))
            .filter(tool_call::Column::Status.eq(ToolCallStatus::Executing))
            .one(tx.db())
            .await?
        else {
            return Ok(None);
        };
        let run = run::Entity::find_by_id(run_id)
            .one(tx.db())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("call {} has no run", call.id)))?;
        let Some(CallRequest::QueryKnowledge { query, spaces }) = call.parsed.clone() else {
            return Err(StoreError::Corrupt(format!(
                "call {} executes, but is not a knowledge query",
                call.id
            )));
        };
        let now = tx.now();
        let outcome = match check(tx.db(), &run, &call, &query, &spaces, now).await? {
            Ok(search) => {
                tx.commit().await?;
                return Ok(Some(search));
            }
            Err(outcome) => outcome,
        };
        conclude(&mut tx, run, &call, outcome).await?;
        tx.commit().await?;
        Ok(None)
    }

    /// Resolves an executing call with the outcome of its work.
    pub async fn finish_tool(
        &self,
        call_id: ToolCallId,
        outcome: ToolOutcome,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_internal().await?;
        let call = tool_call::Entity::find_by_id(call_id)
            .one(tx.db())
            .await?
            .filter(|call| call.status == ToolCallStatus::Executing)
            .ok_or_else(|| StoreError::Corrupt(format!("call {call_id} is not executing")))?;
        let run = run::Entity::find_by_id(call.run_id)
            .one(tx.db())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("call {call_id} has no run")))?;
        conclude(&mut tx, run, &call, outcome).await?;
        tx.commit().await
    }
}

/// Resolves the call; a run that is still queued goes on, a stopped one keeps the result
/// for when it continues.
async fn conclude(
    tx: &mut WriteTx,
    run: run::Model,
    call: &tool_call::Model,
    outcome: ToolOutcome,
) -> Result<(), StoreError> {
    resolve(tx, call, outcome).await?;
    if run.status == RunStatus::Queued {
        requeue(tx, run, false).await?;
    }
    Ok(())
}

/// The search to carry out, charged; or the outcome that ends the call uncharged.
async fn check<C: ConnectionTrait>(
    db: &C,
    run: &run::Model,
    call: &tool_call::Model,
    query: &str,
    spaces: &[String],
    now: Timestamp,
) -> Result<Result<KnowledgeSearch, ToolOutcome>, StoreError> {
    if let Some(reason) = search_unavailable(db).await? {
        return Ok(Err(ToolOutcome::KnowledgeUnavailable { reason }));
    }
    let query = match SearchQuery::parse(query) {
        Ok(query) => query,
        Err(FieldError { problem, .. }) => {
            return Ok(Err(ToolOutcome::KnowledgeQueryInvalid { problem }));
        }
    };
    let live = knowledge_space::Entity::find()
        .filter(knowledge_space::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    let mut ids = Vec::with_capacity(spaces.len());
    let mut gone = Vec::new();
    for name in spaces {
        match live.iter().find(|space| space.name.as_str() == name) {
            Some(space) => ids.push(space.id),
            None => gone.push(name.clone()),
        }
    }
    if !gone.is_empty() {
        return Ok(Err(ToolOutcome::KnowledgeSpacesGone { names: gone }));
    }

    let budget = project::Entity::find_by_id(run.project_id)
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("run {} has no project", run.id)))?
        .auxiliary_budget
        .get();
    let scope = scope(run)?;
    let charged = budget_charge::Entity::find_by_id(call.id)
        .one(db)
        .await?
        .is_some();
    let used = u32::try_from(
        budget_charge::Entity::find()
            .filter(budget_charge::Column::Scope.eq(scope.clone()))
            .count(db)
            .await?,
    )
    .unwrap_or(u32::MAX);
    if !charged {
        if used >= budget {
            return Ok(Err(ToolOutcome::QuotaExhausted { budget }));
        }
        budget_charge::ActiveModel {
            tool_call_id: Set(call.id),
            scope: Set(scope),
            charged_at: Set(now),
        }
        .insert(db)
        .await?;
    }
    let used = if charged { used } else { used + 1 };
    Ok(Ok(KnowledgeSearch {
        call_id: call.id,
        query,
        spaces: ids,
        top_k: TopK::DEFAULT,
        remaining_budget: budget.saturating_sub(used),
    }))
}

/// The budget scope of a run: its version's preparation, or its chapter's outline. Runs
/// of other kinds have no auxiliary tools.
fn scope(run: &run::Model) -> Result<String, StoreError> {
    match (run.kind, run.chapter_id) {
        (RunKind::Preparation, _) => Ok(format!("preparation:{}", run.version_id)),
        (RunKind::ChapterOutline, Some(chapter)) => Ok(format!("outline:{chapter}")),
        _ => Err(StoreError::Corrupt(format!(
            "run {} has no budget scope for auxiliary tools",
            run.id
        ))),
    }
}
