//! Answering the agent's questions (`docs/product-spec.md` §8.6).

use nekowriter_core::{
    call::CallRequest,
    conflict::{Resource, StateConflict},
    ids::{CommandId, RunId, ToolCallId},
    question::{Answer, Question, check_answers},
    run::RunStatus,
    run_message::ToolOutcome,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    calls::{requeue, resolve},
    command::Begun,
    entity::{
        run,
        tool_call::{self, ToolCallStatus},
    },
    workflow::{WorkspaceAck, load_version, touch},
};

/// The questions a run waits for answers to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QuestionView {
    pub call_id: ToolCallId,
    pub questions: Vec<Question>,
}

impl Store {
    /// Answers the agent's questions; the run goes on with the answers as the call's
    /// result. A call is answered once.
    pub async fn answer_questions(
        &self,
        command: CommandId,
        call_id: ToolCallId,
        answers: Vec<Answer>,
    ) -> Result<WorkspaceAck, StoreError> {
        let input = (call_id, &answers);
        let mut tx = match self
            .begin_command(command, "question.answer", &input)
            .await?
        {
            Begun::Replayed(ack) => return Ok(ack),
            Begun::Fresh(tx) => tx,
        };
        let call = tool_call::Entity::find_by_id(call_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Question))?;
        let run = run::Entity::find_by_id(call.run_id)
            .one(tx.db())
            .await?
            .ok_or(StoreError::NotFound(Resource::Run))?;
        let context = load_version(tx.db(), run.version_id).await?;
        let Some(CallRequest::Ask { questions }) = call.parsed.clone() else {
            return Err(StoreError::NotFound(Resource::Question));
        };
        if call.status != ToolCallStatus::AwaitingAnswer || run.status != RunStatus::WaitingAnswer {
            return Err(StoreError::StateConflict(StateConflict::QuestionNotOpen));
        }
        let answered = check_answers(&questions, &answers).map_err(StoreError::AnswerInvalid)?;
        resolve(
            tx.write(),
            &call,
            ToolOutcome::Answered { answers: answered },
        )
        .await?;
        requeue(tx.write(), run, false).await?;
        let workflow = touch(tx.write(), context.version.id, context.project.id).await?;
        tx.commit(WorkspaceAck {
            revision: workflow.revision,
        })
        .await
    }
}

/// The questions the run waits for answers to, if any.
pub(crate) async fn question_view<C: ConnectionTrait>(
    db: &C,
    run_id: RunId,
) -> Result<Option<QuestionView>, StoreError> {
    let call = tool_call::Entity::find()
        .filter(tool_call::Column::RunId.eq(run_id))
        .filter(tool_call::Column::Status.eq(ToolCallStatus::AwaitingAnswer))
        .one(db)
        .await?;
    Ok(call.and_then(|call| match call.parsed {
        Some(CallRequest::Ask { questions }) => Some(QuestionView {
            call_id: call.id,
            questions,
        }),
        _ => None,
    }))
}
