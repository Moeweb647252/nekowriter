//! The agent's questions and the user's answers (`docs/product-spec.md` §8.6).

mod common;

use common::{
    answer_with, call, dispatch_next, drafting, next_results, open_store, started, workspace,
};
use nekowriter_core::{
    call::CallRequest,
    conflict::StateConflict,
    feedback::FeedbackText,
    ids::{CommandId, VersionId},
    question::{
        Answer, AnswerProblem, AnsweredQuestion, Question, QuestionKind, QuestionOption,
        QuestionProblem,
    },
    run::RunStatus,
    run_message::ToolOutcome,
};
use nekowriter_store::{RoundRecord, Store, StoreError};

fn weather(id: &str) -> Question {
    Question {
        id: id.to_owned(),
        title: "What weather opens the chapter?".to_owned(),
        kind: QuestionKind::Single,
        options: vec![
            QuestionOption {
                id: "a".to_owned(),
                label: "Rain".to_owned(),
            },
            QuestionOption {
                id: "b".to_owned(),
                label: "Snow".to_owned(),
            },
        ],
        required: true,
    }
}

fn ask(questions: Vec<Question>) -> RoundRecord {
    RoundRecord::Call {
        call: call("ask_user"),
        request: CallRequest::Ask { questions },
    }
}

fn answer(selected: &[&str], text: Option<&str>) -> Answer {
    Answer {
        question_id: "q1".to_owned(),
        selected_option_ids: selected.iter().map(|&id| id.to_owned()).collect(),
        custom_text: text.map(str::to_owned),
    }
}

/// Answers the version's open question.
async fn reply(store: &Store, version: VersionId, answers: Vec<Answer>) -> Result<(), StoreError> {
    let question = workspace(store, version)
        .await
        .question
        .expect("a question waits");
    store
        .answer_questions(CommandId::generate(), question.call_id, answers)
        .await
        .map(drop)
}

#[tokio::test]
async fn a_question_waits_for_its_answer_and_carries_feedback_given_meanwhile() {
    let (_dir, store) = open_store().await;
    let (version, run, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;

    answer_with(&store, request, ask(vec![weather("q1")])).await;
    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::WaitingAnswer);
    assert_eq!(view.question.unwrap().questions, [weather("q1")]);

    store
        .submit_feedback(
            CommandId::generate(),
            run,
            FeedbackText::parse("Keep it short.").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::WaitingAnswer
    );
    reply(&store, version, vec![answer(&["b"], Some("with fog"))])
        .await
        .unwrap();

    let view = workspace(&store, version).await;
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert_eq!(view.question, None);
    assert_eq!(
        next_results(&store).await,
        [(
            ToolOutcome::Answered {
                answers: vec![AnsweredQuestion {
                    question: "What weather opens the chapter?".to_owned(),
                    selected: vec!["Snow".to_owned()],
                    custom_text: Some("with fog".to_owned()),
                }]
            },
            vec!["Keep it short.".to_owned()]
        )]
    );
}

#[tokio::test]
async fn answers_are_checked_and_given_once() {
    let (_dir, store) = open_store().await;
    let (version, _, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;
    answer_with(&store, request, ask(vec![weather("q1")])).await;
    let call = workspace(&store, version).await.question.unwrap().call_id;

    let missing = reply(&store, version, vec![answer(&[], Some(" "))]).await;
    assert!(matches!(
        missing,
        Err(StoreError::AnswerInvalid(AnswerProblem::Missing { .. }))
    ));
    reply(&store, version, vec![answer(&["a"], None)])
        .await
        .unwrap();

    let again = store
        .answer_questions(CommandId::generate(), call, vec![answer(&["b"], None)])
        .await
        .unwrap_err();
    assert!(matches!(
        again,
        StoreError::StateConflict(StateConflict::QuestionNotOpen)
    ));
}

#[tokio::test]
async fn questions_that_cannot_be_shown_go_back_to_the_agent() {
    let (_dir, store) = open_store().await;
    let (version, _, _) = drafting(&store).await;
    let (_, request) = dispatch_next(&store).await;

    answer_with(&store, request, ask(vec![weather("q1"), weather("q1")])).await;

    let view = workspace(&store, version).await;
    assert_eq!(view.question, None);
    assert_eq!(view.run.unwrap().status, RunStatus::Queued);
    assert_eq!(
        next_results(&store).await[0].0,
        ToolOutcome::QuestionRejected {
            problem: QuestionProblem::DuplicateQuestion {
                id: "q1".to_owned()
            }
        }
    );
}

#[tokio::test]
async fn preparation_can_ask_too() {
    let (_dir, store) = open_store().await;
    let version = started(&store).await;
    let (_, request) = dispatch_next(&store).await;

    answer_with(&store, request, ask(vec![weather("q1")])).await;

    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::WaitingAnswer
    );
    reply(&store, version, vec![answer(&["a"], None)])
        .await
        .unwrap();
    assert_eq!(
        workspace(&store, version).await.run.unwrap().status,
        RunStatus::Queued
    );
}
