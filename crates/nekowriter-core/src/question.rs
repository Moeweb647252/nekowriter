//! Questions the agent asks the user with `ask_user`, and their answers
//! (`docs/product-spec.md` §8.6).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most questions one call may ask.
pub const MAX_QUESTIONS: usize = 10;
/// The most options a choice may offer.
pub const MAX_OPTIONS: usize = 20;
/// The longest custom answer, in characters.
pub const ANSWER_MAX_CHARS: u32 = 5_000;

/// One question, as the agent asks it. Choices always also accept custom text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Question {
    /// Stable within the call; answers refer to it.
    pub id: String,
    pub title: String,
    pub kind: QuestionKind,
    /// The choices of a single or multiple choice question.
    pub options: Vec<QuestionOption>,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum QuestionKind {
    Single,
    Multiple,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
}

/// Why the agent's questions cannot be shown. The agent is told and may ask again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum QuestionProblem {
    NoQuestions,
    TooManyQuestions {
        max: u32,
    },
    DuplicateQuestion {
        id: String,
    },
    EmptyTitle {
        id: String,
    },
    /// A choice question offers no options.
    NoOptions {
        id: String,
    },
    TooManyOptions {
        id: String,
        max: u32,
    },
    DuplicateOption {
        id: String,
        option: String,
    },
    EmptyOption {
        id: String,
        option: String,
    },
}

/// Checks the agent's questions before they are shown.
pub fn check_questions(questions: &[Question]) -> Result<(), QuestionProblem> {
    if questions.is_empty() {
        return Err(QuestionProblem::NoQuestions);
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(QuestionProblem::TooManyQuestions {
            max: u32::try_from(MAX_QUESTIONS).unwrap_or(u32::MAX),
        });
    }
    let mut ids = BTreeSet::new();
    for question in questions {
        let id = || question.id.clone();
        if !ids.insert(question.id.as_str()) {
            return Err(QuestionProblem::DuplicateQuestion { id: id() });
        }
        if question.title.trim().is_empty() {
            return Err(QuestionProblem::EmptyTitle { id: id() });
        }
        if question.kind == QuestionKind::Text {
            continue;
        }
        if question.options.is_empty() {
            return Err(QuestionProblem::NoOptions { id: id() });
        }
        if question.options.len() > MAX_OPTIONS {
            return Err(QuestionProblem::TooManyOptions {
                id: id(),
                max: u32::try_from(MAX_OPTIONS).unwrap_or(u32::MAX),
            });
        }
        let mut options = BTreeSet::new();
        for option in &question.options {
            if !options.insert(option.id.as_str()) {
                return Err(QuestionProblem::DuplicateOption {
                    id: id(),
                    option: option.id.clone(),
                });
            }
            if option.label.trim().is_empty() {
                return Err(QuestionProblem::EmptyOption {
                    id: id(),
                    option: option.id.clone(),
                });
            }
        }
    }
    Ok(())
}

/// The user's answer to one question: the selected options and any custom text, kept
/// apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Answer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub custom_text: Option<String>,
}

/// Why the user's answers do not fit the questions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum AnswerProblem {
    /// The answer refers to no question of the call, or answers one twice.
    UnknownQuestion {
        question_id: String,
    },
    /// A required question has neither a selection nor text.
    Missing {
        question_id: String,
    },
    /// A single choice question has more than one option selected.
    SeveralSelected {
        question_id: String,
    },
    UnknownOption {
        question_id: String,
        option_id: String,
    },
    TooLong {
        question_id: String,
        max_chars: u32,
    },
}

/// An answer as the agent reads it: the question and the chosen labels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnsweredQuestion {
    pub question: String,
    pub selected: Vec<String>,
    pub custom_text: Option<String>,
}

/// Checks the user's answers against the questions and returns them as the agent reads
/// them, in question order. Unanswered optional questions are answered empty.
pub fn check_answers(
    questions: &[Question],
    answers: &[Answer],
) -> Result<Vec<AnsweredQuestion>, AnswerProblem> {
    let mut seen = BTreeSet::new();
    for answer in answers {
        let known = questions
            .iter()
            .any(|question| question.id == answer.question_id);
        if !known || !seen.insert(answer.question_id.as_str()) {
            return Err(AnswerProblem::UnknownQuestion {
                question_id: answer.question_id.clone(),
            });
        }
    }
    questions
        .iter()
        .map(|question| {
            let answer = answers
                .iter()
                .find(|answer| answer.question_id == question.id);
            let question_id = || question.id.clone();
            let selected_ids = answer.map_or(&[][..], |answer| &answer.selected_option_ids[..]);
            let custom_text = answer
                .and_then(|answer| answer.custom_text.as_deref())
                .map(str::trim)
                .filter(|text| !text.is_empty());
            if custom_text.is_some_and(|text| text.chars().count() > ANSWER_MAX_CHARS as usize) {
                return Err(AnswerProblem::TooLong {
                    question_id: question_id(),
                    max_chars: ANSWER_MAX_CHARS,
                });
            }
            if question.kind == QuestionKind::Single && selected_ids.len() > 1 {
                return Err(AnswerProblem::SeveralSelected {
                    question_id: question_id(),
                });
            }
            let mut selected = Vec::with_capacity(selected_ids.len());
            for option_id in selected_ids {
                let option = question
                    .options
                    .iter()
                    .find(|option| &option.id == option_id)
                    .ok_or_else(|| AnswerProblem::UnknownOption {
                        question_id: question_id(),
                        option_id: option_id.clone(),
                    })?;
                selected.push(option.label.clone());
            }
            if question.required && selected.is_empty() && custom_text.is_none() {
                return Err(AnswerProblem::Missing {
                    question_id: question_id(),
                });
            }
            Ok(AnsweredQuestion {
                question: question.title.clone(),
                selected,
                custom_text: custom_text.map(str::to_owned),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str, kind: QuestionKind, required: bool) -> Question {
        Question {
            id: id.to_owned(),
            title: format!("Question {id}"),
            kind,
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
            required,
        }
    }

    fn answer(id: &str, selected: &[&str], text: Option<&str>) -> Answer {
        Answer {
            question_id: id.to_owned(),
            selected_option_ids: selected.iter().map(|&option| option.to_owned()).collect(),
            custom_text: text.map(str::to_owned),
        }
    }

    #[test]
    fn questions_need_unique_ids_and_options_for_choices() {
        assert_eq!(check_questions(&[]), Err(QuestionProblem::NoQuestions));
        let single = choice("q1", QuestionKind::Single, true);
        assert_eq!(
            check_questions(&[single.clone(), single.clone()]),
            Err(QuestionProblem::DuplicateQuestion {
                id: "q1".to_owned()
            })
        );
        let mut bare = single;
        bare.options.clear();
        assert_eq!(
            check_questions(&[bare]),
            Err(QuestionProblem::NoOptions {
                id: "q1".to_owned()
            })
        );
        let mut text = choice("q2", QuestionKind::Text, false);
        text.options.clear();
        assert_eq!(check_questions(&[text]), Ok(()));
    }

    #[test]
    fn answers_keep_selections_and_custom_text_apart() {
        let questions = [
            choice("q1", QuestionKind::Single, true),
            choice("q2", QuestionKind::Multiple, false),
        ];
        let answered =
            check_answers(&questions, &[answer("q1", &["b"], Some("  and fog "))]).unwrap();
        assert_eq!(answered[0].selected, ["Snow"]);
        assert_eq!(answered[0].custom_text.as_deref(), Some("and fog"));
        assert_eq!(answered[1].selected, Vec::<String>::new());
    }

    #[test]
    fn answers_respect_the_question_kinds() {
        let questions = [choice("q1", QuestionKind::Single, true)];
        assert!(matches!(
            check_answers(&questions, &[answer("q1", &["a", "b"], None)]),
            Err(AnswerProblem::SeveralSelected { .. })
        ));
        assert!(matches!(
            check_answers(&questions, &[answer("q1", &[], Some(" "))]),
            Err(AnswerProblem::Missing { .. })
        ));
        assert!(matches!(
            check_answers(&questions, &[answer("q1", &["z"], None)]),
            Err(AnswerProblem::UnknownOption { .. })
        ));
        assert!(matches!(
            check_answers(&questions, &[answer("q9", &[], None)]),
            Err(AnswerProblem::UnknownQuestion { .. })
        ));
    }
}
