//! 领域类型、输入校验和不含 I/O 的工作流基础。
//!
//! 这个 crate 不依赖数据库、HTTP 或模型协议。应用层负责把这些纯类型持久化并
//! 编排成完整工作流。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub mod budget;
pub mod dashboard;
pub mod export;
pub mod knowledge;
pub mod manuscript;
pub mod recycle;
pub mod template;
pub mod usage;
pub use budget::*;
pub use dashboard::*;
pub use export::*;
pub use knowledge::*;
pub use manuscript::*;
pub use template::*;
pub use usage::*;

/// 跨进程边界的 64 位整数统一使用十进制字符串，避免 JavaScript 的安全整数范围
/// 悄悄截断数据库中的修订号、序列号或时间戳。
pub mod decimal_i64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    /// # Errors
    ///
    /// 当调用方的序列化器拒绝字符串值时返回其错误。
    pub fn serialize<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    /// # Errors
    ///
    /// JSON 值不是十进制字符串，或字符串超出 `i64` 范围时返回反序列化错误。
    pub fn deserialize<'de, D>(deserializer: D) -> Result<i64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value
            .parse::<i64>()
            .map_err(|error| D::Error::custom(format!("64 位整数必须是十进制字符串：{error}")))
    }
}

/// [`decimal_i64`] 的可空字段版本。
pub(crate) mod option_decimal_i64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    /// # Errors
    ///
    /// 当调用方的序列化器拒绝可空字符串值时返回其错误。
    #[allow(clippy::ref_option)] // Serde `with` 模块固定要求 `&Option<T>` 签名。
    pub fn serialize<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    /// # Errors
    ///
    /// 非空 JSON 值不是十进制字符串，或数值超出 `i64` 范围时返回反序列化错误。
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                value.parse::<i64>().map_err(|error| {
                    D::Error::custom(format!("64 位整数必须是十进制字符串：{error}"))
                })
            })
            .transpose()
    }
}

pub const MAX_PROJECT_NAME_LEN: usize = 120;
pub const MAX_NOVEL_TITLE_LEN: usize = 200;
pub const MAX_VERSION_LABEL_LEN: usize = 80;
pub const MAX_NOVEL_BRIEF_LEN: usize = 20_000;
pub const MAX_ARTIFACT_CONTENT_LEN: usize = 2_000_000;
pub const MAX_REVIEW_ISSUES_LEN: usize = 20_000;
pub const MAX_CHAPTER_PLAN_ITEMS: usize = 500;
pub const MAX_CHAPTER_TITLE_LEN: usize = 200;
pub const MAX_CHAPTER_GOAL_LEN: usize = 20_000;
pub const MAX_CHAPTER_GUIDANCE_LEN: usize = 20_000;
pub const MAX_QUESTIONS_PER_TOOL_CALL: usize = 20;
pub const MAX_QUESTION_OPTIONS: usize = 50;
pub const MAX_QUESTION_ID_LEN: usize = 120;
pub const MAX_QUESTION_TITLE_LEN: usize = 1_000;
pub const MAX_QUESTION_OPTION_LABEL_LEN: usize = 1_000;
pub const MAX_QUESTION_CUSTOM_TEXT_LEN: usize = 20_000;
/// 同一个自动写作 Run 连续返回无效协议响应的上限。
///
/// 每次合规的阶段提交或工具调用都会清零；达到上限后必须由用户显式继续，不能无限
/// 消耗模型请求额度重试同一份错误输出。
pub const MAX_CONSECUTIVE_PROTOCOL_ERRORS: i64 = 3;

pub type ProjectId = Uuid;
pub type VersionId = Uuid;
pub type DocumentId = Uuid;
pub type RevisionId = Uuid;
pub type RunId = Uuid;
pub type ReviewId = Uuid;
pub type ChapterId = Uuid;
pub type FeedbackId = Uuid;
pub type QuestionId = Uuid;

/// `ask_user` 中一题可接受的回答交互形式。
///
/// 单选和多选都允许另附自定义文本；文本题只接受自定义文本。选项 ID 与问题 ID
/// 是模型在同一工具调用内声明的稳定键，回答时不会按显示位置猜测含义。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserQuestionKind {
    Single,
    Multiple,
    Text,
}

/// `ask_user` 问题的一个预设选项。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserQuestionOption {
    pub id: String,
    pub label: String,
}

/// 一个由 Agent 声明、待用户回答的问题。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserQuestion {
    pub id: String,
    pub title: String,
    pub kind: UserQuestionKind,
    pub options: Vec<UserQuestionOption>,
    pub required: bool,
}

/// 单次 `ask_user` 工具调用的完整问题模式。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserQuestionSchema {
    pub questions: Vec<UserQuestion>,
}

/// 一题的用户答案；预设选项和自由输入分开保存，避免把“其他”伪造成一个选项。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserQuestionAnswer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub custom_text: Option<String>,
}

/// 用户一次提交给整组问题的答案输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserQuestionAnswers {
    pub answers: Vec<UserQuestionAnswer>,
}

/// 工作台当前可见的一组待答问题。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUserQuestion {
    pub id: QuestionId,
    pub tool_call_id: Uuid,
    pub schema: UserQuestionSchema,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
}

/// `ask_user` 模式或答案不满足显式合同的原因。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum UserQuestionValidationError {
    #[error("问题列表不能为空")]
    EmptyQuestions,
    #[error("一次询问最多包含 {max} 个问题")]
    TooManyQuestions { max: usize },
    #[error("问题模式无效：{0}")]
    InvalidSchema(String),
    #[error("问题答案无效：{0}")]
    InvalidAnswer(String),
}

impl UserQuestionSchema {
    /// 校验并保留 Agent 原样声明的问题模式。
    ///
    /// # Errors
    ///
    /// 问题 ID/标题/选项缺失或重复，题型与选项不匹配，或列表超过固定安全上限时返回
    /// [`UserQuestionValidationError`]。
    pub fn validate(&self) -> Result<(), UserQuestionValidationError> {
        if self.questions.is_empty() {
            return Err(UserQuestionValidationError::EmptyQuestions);
        }
        if self.questions.len() > MAX_QUESTIONS_PER_TOOL_CALL {
            return Err(UserQuestionValidationError::TooManyQuestions {
                max: MAX_QUESTIONS_PER_TOOL_CALL,
            });
        }
        let mut question_ids = std::collections::HashSet::with_capacity(self.questions.len());
        for question in &self.questions {
            validate_user_question_token("问题 ID", &question.id, MAX_QUESTION_ID_LEN)?;
            if !question_ids.insert(question.id.as_str()) {
                return Err(UserQuestionValidationError::InvalidSchema(format!(
                    "问题 ID {} 重复",
                    question.id
                )));
            }
            validate_user_question_token("问题标题", &question.title, MAX_QUESTION_TITLE_LEN)?;
            match question.kind {
                UserQuestionKind::Text if !question.options.is_empty() => {
                    return Err(UserQuestionValidationError::InvalidSchema(format!(
                        "文本题 {} 不能声明预设选项",
                        question.id
                    )));
                }
                UserQuestionKind::Single | UserQuestionKind::Multiple
                    if question.options.is_empty() =>
                {
                    return Err(UserQuestionValidationError::InvalidSchema(format!(
                        "选择题 {} 至少需要一个选项",
                        question.id
                    )));
                }
                _ => {}
            }
            if question.options.len() > MAX_QUESTION_OPTIONS {
                return Err(UserQuestionValidationError::InvalidSchema(format!(
                    "问题 {} 的选项不能超过 {} 项",
                    question.id, MAX_QUESTION_OPTIONS
                )));
            }
            let mut option_ids = std::collections::HashSet::with_capacity(question.options.len());
            for option in &question.options {
                validate_user_question_token("选项 ID", &option.id, MAX_QUESTION_ID_LEN)?;
                if !option_ids.insert(option.id.as_str()) {
                    return Err(UserQuestionValidationError::InvalidSchema(format!(
                        "问题 {} 的选项 ID {} 重复",
                        question.id, option.id
                    )));
                }
                validate_user_question_token(
                    "选项标签",
                    &option.label,
                    MAX_QUESTION_OPTION_LABEL_LEN,
                )?;
            }
        }
        Ok(())
    }

    /// 将用户提交规范化为与模式同序、每题恰好一项的稳定工具结果。
    ///
    /// 可选题可以省略，返回值会以空选择和空自定义文本补齐；必答题必须至少给出一个
    /// 预设选择或非空自定义文本。此函数不 trim 或改写用户文本，仅以 Unicode 空白
    /// 判定“是否已作答”。
    ///
    /// # Errors
    ///
    /// 模式损坏、回答包含未知/重复问题或选项、单选选择多个选项、文本题附带选项，或
    /// 必答题为空时返回 [`UserQuestionValidationError`]。
    pub fn validate_answers(
        &self,
        answers: &[UserQuestionAnswer],
    ) -> Result<Vec<UserQuestionAnswer>, UserQuestionValidationError> {
        self.validate()?;
        if answers.len() > self.questions.len() {
            return Err(UserQuestionValidationError::InvalidAnswer(
                "回答数量超过问题数量".to_owned(),
            ));
        }
        let mut supplied = std::collections::HashMap::with_capacity(answers.len());
        for answer in answers {
            validate_user_question_token(
                "回答中的问题 ID",
                &answer.question_id,
                MAX_QUESTION_ID_LEN,
            )
            .map_err(|error| UserQuestionValidationError::InvalidAnswer(error.to_string()))?;
            if supplied
                .insert(answer.question_id.as_str(), answer)
                .is_some()
            {
                return Err(UserQuestionValidationError::InvalidAnswer(format!(
                    "问题 {} 被重复回答",
                    answer.question_id
                )));
            }
        }
        if let Some(answer) = answers.iter().find(|answer| {
            !self
                .questions
                .iter()
                .any(|question| question.id == answer.question_id)
        }) {
            return Err(UserQuestionValidationError::InvalidAnswer(format!(
                "不存在问题 {}",
                answer.question_id
            )));
        }
        let mut normalized = Vec::with_capacity(self.questions.len());
        for question in &self.questions {
            let answer = supplied
                .get(question.id.as_str())
                .copied()
                .cloned()
                .unwrap_or(UserQuestionAnswer {
                    question_id: question.id.clone(),
                    selected_option_ids: Vec::new(),
                    custom_text: None,
                });
            let custom_text_is_present = answer
                .custom_text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty());
            if answer
                .custom_text
                .as_deref()
                .is_some_and(|text| text.chars().count() > MAX_QUESTION_CUSTOM_TEXT_LEN)
            {
                return Err(UserQuestionValidationError::InvalidAnswer(format!(
                    "问题 {} 的自定义答案超过 {} 个字符",
                    question.id, MAX_QUESTION_CUSTOM_TEXT_LEN
                )));
            }
            let mut selected_ids =
                std::collections::HashSet::with_capacity(answer.selected_option_ids.len());
            for selected_option_id in &answer.selected_option_ids {
                validate_user_question_token(
                    "回答中的选项 ID",
                    selected_option_id,
                    MAX_QUESTION_ID_LEN,
                )
                .map_err(|error| UserQuestionValidationError::InvalidAnswer(error.to_string()))?;
                if !selected_ids.insert(selected_option_id.as_str()) {
                    return Err(UserQuestionValidationError::InvalidAnswer(format!(
                        "问题 {} 重复选择了选项 {}",
                        question.id, selected_option_id
                    )));
                }
                if !question
                    .options
                    .iter()
                    .any(|option| option.id == *selected_option_id)
                {
                    return Err(UserQuestionValidationError::InvalidAnswer(format!(
                        "问题 {} 不包含选项 {}",
                        question.id, selected_option_id
                    )));
                }
            }
            match question.kind {
                UserQuestionKind::Text if !answer.selected_option_ids.is_empty() => {
                    return Err(UserQuestionValidationError::InvalidAnswer(format!(
                        "文本题 {} 不能选择预设选项",
                        question.id
                    )));
                }
                UserQuestionKind::Single if answer.selected_option_ids.len() > 1 => {
                    return Err(UserQuestionValidationError::InvalidAnswer(format!(
                        "单选题 {} 最多选择一个预设选项",
                        question.id
                    )));
                }
                _ => {}
            }
            if question.required && answer.selected_option_ids.is_empty() && !custom_text_is_present
            {
                return Err(UserQuestionValidationError::InvalidAnswer(format!(
                    "必答问题 {} 尚未回答",
                    question.id
                )));
            }
            normalized.push(answer);
        }
        Ok(normalized)
    }
}

fn validate_user_question_token(
    field: &str,
    value: &str,
    max_len: usize,
) -> Result<(), UserQuestionValidationError> {
    if value.trim().is_empty() {
        return Err(UserQuestionValidationError::InvalidSchema(format!(
            "{field} 不能为空"
        )));
    }
    if value.chars().count() > max_len {
        return Err(UserQuestionValidationError::InvalidSchema(format!(
            "{field} 不能超过 {max_len} 个字符"
        )));
    }
    Ok(())
}

/// 当前章节的一次用户指导输入。
///
/// 空指导是明确允许的：用户可以只使用已通过的准备产物和章节计划开始细纲，
/// 但它仍会持久化为一次输入快照，不能在运行开始后悄悄替换。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChapterGuidance {
    pub content: String,
}

impl NewChapterGuidance {
    /// # Errors
    ///
    /// 指导超过长度限制时返回 [`ValidationError`]；空文本合法。
    pub fn new(content: impl AsRef<str>) -> Result<Self, ValidationError> {
        let content = content.as_ref();
        if content.chars().count() > MAX_CHAPTER_GUIDANCE_LEN {
            return Err(ValidationError::TooLong {
                field: "章节指导",
                max: MAX_CHAPTER_GUIDANCE_LEN,
            });
        }
        Ok(Self {
            content: content.to_owned(),
        })
    }
}

/// 章节指导文档的当前可见修订。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentChapterGuidance {
    pub document_id: DocumentId,
    pub revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub revision_no: Option<i64>,
    pub content: Option<String>,
}

/// 一次可审核的章节细纲输入。
///
/// 无论来自模型的结构化阶段提交还是用户手工录入，细纲都必须先通过同一份
/// 领域校验，再成为正式文档修订。这样审核、依赖和上下文折叠没有两条实现。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChapterOutline {
    pub content: String,
}

impl NewChapterOutline {
    /// # Errors
    ///
    /// 空细纲或超过单个产物长度上限时返回 [`ValidationError`]。
    pub fn new(content: impl AsRef<str>) -> Result<Self, ValidationError> {
        Ok(Self {
            content: validate_document_content(
                "章节细纲",
                content.as_ref(),
                MAX_ARTIFACT_CONTENT_LEN,
            )?,
        })
    }
}

/// 当前章节细纲对应的一次审核决定。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterOutlineReview {
    pub id: ReviewId,
    pub target_revision_id: RevisionId,
    pub status: ReviewStatus,
    pub passed: Option<bool>,
    pub confidence: Option<serde_json::Number>,
    pub issues: Option<String>,
}

/// 当前章节细纲文档的可见修订与其审核状态。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentChapterOutline {
    pub document_id: DocumentId,
    pub revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub revision_no: Option<i64>,
    pub content: Option<String>,
    pub review: Option<ChapterOutlineReview>,
}

/// 一次追加正文的工具输入。
///
/// 正文统一以 LF 保存；输入中的 CRLF 与单独 CR 会在创建输入时归一化，其他
/// 字符保持原样，不自动补换行、trim 或 Unicode 归一化。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendChapterContent {
    pub content: String,
}

impl AppendChapterContent {
    /// # Errors
    ///
    /// 追加内容为空或超过单次产物限制时返回 [`ValidationError`]。
    pub fn new(content: impl AsRef<str>) -> Result<Self, ValidationError> {
        let content = normalize_line_endings(content.as_ref());
        Ok(Self {
            content: validate_document_content("追加正文", &content, MAX_ARTIFACT_CONTENT_LEN)?,
        })
    }
}

/// 一次精确替换正文的工具输入。
///
/// `new_str` 可以为空，表示删除；`old_str` 必须非空，且稍后必须在当前正文中
/// （包括重叠位置）恰好出现一次。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceChapterContent {
    pub old_str: String,
    pub new_str: String,
}

impl ReplaceChapterContent {
    /// # Errors
    ///
    /// 待替换文本为空、或任一参数超过长度限制时返回 [`ValidationError`]。
    pub fn new(
        old_str: impl AsRef<str>,
        new_str: impl AsRef<str>,
    ) -> Result<Self, ValidationError> {
        let old_str = normalize_line_endings(old_str.as_ref());
        let old_str = validate_document_content("待替换正文", &old_str, MAX_ARTIFACT_CONTENT_LEN)?;
        let new_str = normalize_line_endings(new_str.as_ref());
        if new_str.chars().count() > MAX_ARTIFACT_CONTENT_LEN {
            return Err(ValidationError::TooLong {
                field: "替换后的正文",
                max: MAX_ARTIFACT_CONTENT_LEN,
            });
        }
        Ok(Self { old_str, new_str })
    }
}

/// 精确正文替换不能执行时的领域错误。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChapterBodyEditError {
    #[error("待替换正文不能为空")]
    EmptyOldText,
    #[error("待替换正文在当前章节中没有匹配")]
    NoMatch,
    #[error("待替换正文在当前章节中匹配多处")]
    MultipleMatches,
    #[error("替换前后文本完全相同，不会创建无意义修订")]
    NoChange,
}

/// 在正文中执行一次严格唯一的精确替换。
///
/// 搜索按 UTF-8 字符边界推进一个字符而不是整个匹配长度，因此 `aaa` 中的 `aa`
/// 会被正确识别为两个重叠匹配。该函数不处理阶段守卫或持久化，只决定候选文本。
///
/// # Errors
///
/// 没有匹配、存在多处（含重叠）匹配或新旧文本相同会返回
/// [`ChapterBodyEditError`]。
pub fn replace_unique_chapter_content(
    current: &str,
    old_str: &str,
    new_str: &str,
) -> Result<String, ChapterBodyEditError> {
    if old_str.is_empty() {
        return Err(ChapterBodyEditError::EmptyOldText);
    }
    if old_str == new_str {
        return Err(ChapterBodyEditError::NoChange);
    }
    let mut cursor = 0;
    let mut matching_start = None;
    while cursor < current.len() {
        let Some(relative_start) = current[cursor..].find(old_str) else {
            break;
        };
        let start = cursor + relative_start;
        if matching_start.replace(start).is_some() {
            return Err(ChapterBodyEditError::MultipleMatches);
        }
        let next_char_len = current[start..]
            .chars()
            .next()
            .expect("非空匹配必定位于有效 UTF-8 字符边界")
            .len_utf8();
        cursor = start + next_char_len;
    }
    let start = matching_start.ok_or(ChapterBodyEditError::NoMatch)?;
    let end = start + old_str.len();
    Ok(format!(
        "{}{}{}",
        &current[..start],
        new_str,
        &current[end..]
    ))
}

/// 将正文输入归一化为 LF，以让后续精确替换的匹配语义在不同操作系统一致。
#[must_use]
pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

/// 当前章节正文对应的一次整章审核决定。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterFinalReview {
    pub id: ReviewId,
    pub target_revision_id: RevisionId,
    pub status: ReviewStatus,
    pub passed: Option<bool>,
    pub confidence: Option<serde_json::Number>,
    pub issues: Option<String>,
}

/// 当前章节正文文档的可见修订与整章审核状态。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentChapterBody {
    pub document_id: DocumentId,
    pub revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub revision_no: Option<i64>,
    pub content: Option<String>,
    pub review: Option<ChapterFinalReview>,
}

/// 一条不可变文档修订的只读视图。
///
/// 引用永久锚定在具体修订上，因此该修订即使已经不是文档的当前修订，也必须仍能被
/// 读取。`is_current` 只用于界面提示“来自旧修订”，不改变引用本身的指向。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRevisionView {
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    #[serde(with = "decimal_i64")]
    pub revision_no: i64,
    pub role: String,
    pub chapter_id: Option<ChapterId>,
    #[serde(with = "decimal_i64")]
    pub word_count: i64,
    pub content: String,
    pub is_current: bool,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
}

/// 一段可由后端逐字复核的文本范围。
///
/// 位置以 Unicode scalar value（Rust `char`）计数，而非 UTF-8 字节或浏览器常见
/// 的 UTF-16 code unit。`quote` 是对范围的不可省略校验副本：存储层必须确认它和
/// 对应快照中的原文完全相同，才能接受引用。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextCitationRange {
    #[serde(with = "decimal_i64")]
    pub start_char: i64,
    #[serde(with = "decimal_i64")]
    pub end_char: i64,
    pub quote: String,
}

/// 用户意见所附的一条可验证文本引用。
///
/// 正式文档引用永久锚定在不可变修订上；待审正文候选还没有修订，因此必须锚定
/// 到产生候选的工具调用、候选全文哈希和候选范围。两种来源都由 Store 在写入时
/// 校验，不能由客户端凭 UUID 或偏移自行声明成立。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackCitation {
    #[serde(rename_all = "camelCase")]
    Document {
        document_id: DocumentId,
        revision_id: RevisionId,
        #[serde(flatten)]
        range: TextCitationRange,
        chapter_id: Option<ChapterId>,
    },
    #[serde(rename_all = "camelCase")]
    Candidate {
        tool_call_id: Uuid,
        candidate_hash: String,
        candidate_range: TextCitationRange,
    },
}

/// 一条尚未被用户接受或拒绝的正文编辑候选。
///
/// 候选内容不属于正式正文修订；它仅在 `ApproveRequired` 模式的当前正文 Run 中
/// 可见。接受时存储层会重新核对 `base_revision_id`，从而不会把旧候选套用到已被
/// 其他命令改写的正文上。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChapterBodyProposal {
    pub tool_call_id: Uuid,
    pub provider_call_id: String,
    pub tool_name: String,
    pub base_revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub base_revision_no: Option<i64>,
    pub candidate_content: String,
    /// SHA-256（小写十六进制）保护候选引用不被客户端重新指向另一段未持久化文本。
    pub candidate_hash: String,
}

/// 当前待审正文编辑对应的统一审核记录。
///
/// 编辑候选本身尚未形成文档修订，所以它通过 `target_tool_call_id` 绑定 Review；
/// 接受或拒绝后，候选和该 Review 会在同一事务内一并结束。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterEditReview {
    pub id: ReviewId,
    pub target_tool_call_id: Uuid,
    pub status: ReviewStatus,
    pub passed: Option<bool>,
    pub confidence: Option<serde_json::Number>,
    pub issues: Option<String>,
}

/// 一条已经完整到达、但因用户暂停而尚未执行的正文工具调用。
///
/// `candidate_content` 只在服务端已能安全计算候选正文时出现；它仍不是正式修订。
/// 参数校验失败的调用同样会停在工具边界，此时内容为空，恢复后才把稳定错误结果
/// 回送给 Agent。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldChapterBodyTool {
    pub tool_call_id: Uuid,
    pub provider_call_id: String,
    pub tool_name: String,
    pub base_revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub base_revision_no: Option<i64>,
    pub candidate_content: Option<String>,
    /// 仅当 `candidate_content` 存在时出现的 SHA-256 小写十六进制摘要。
    pub candidate_hash: Option<String>,
}

/// 模型直接提交整章时命中暂停边界的持久化摘要。
///
/// 结构化提交并不携带正文，因此暂停时只向界面暴露其稳定摘要；原始模型文本仍由
/// Store 保存在 Run 检查点，恢复后才按当时的完成守卫进入整章审核或协议纠正。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldChapterBodySubmission {
    pub request_id: Uuid,
    pub summary: String,
}

/// 一条自由意见的持久化投递状态。
///
/// 自由意见与正文编辑提案的正式审核互不混用：前者在同一 Run 的下一次合法工具
/// 结果中投递，后者直接决定一条待审候选是否成为正式正文修订。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Queued,
    Bound,
    Included,
}

/// 当前正文 Run 中尚未封存到模型请求的自由意见。
///
/// `bound_tool_call_id` 为空表示该批次仍在等待下一条完整工具结果；非空表示它将与
/// 对应工具结果一起物化进下一次模型请求。引用和意见一同持久化、物化，不能在
/// 已经发出的请求快照上原地追加。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChapterBodyFeedback {
    pub id: FeedbackId,
    #[serde(with = "decimal_i64")]
    pub sequence: i64,
    pub text: String,
    pub citations: Vec<FeedbackCitation>,
    pub status: FeedbackStatus,
    pub bound_tool_call_id: Option<Uuid>,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
}

/// 已经封存到模型请求、但该请求尚未给出成功响应的自由意见投递状态。
///
/// 这不是反馈本身的状态机：`included` 已经说明意见进入了不可变请求快照。
/// 此处只让工作台说明该快照目前正在发送、失败、未知或已取消，避免用户把队列
/// 项消失误读为意见没有送达。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackDeliveryStatus {
    Sent,
    Failed,
    Unknown,
    Cancelled,
}

/// 当前正文 Run 中最近一次未获成功响应的已封存自由意见。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveredChapterBodyFeedback {
    pub id: FeedbackId,
    #[serde(with = "decimal_i64")]
    pub sequence: i64,
    pub text: String,
    pub citations: Vec<FeedbackCitation>,
    pub request_id: Uuid,
    pub delivery_status: FeedbackDeliveryStatus,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
}

/// 准备阶段的固定产物键。顺序是用户审核与上下文折叠的稳定顺序，不能由
/// Agent 的实际生成顺序改变。
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationArtifactKind {
    Background,
    Research,
    Characters,
    Outline,
}

impl PreparationArtifactKind {
    pub const ALL: [Self; 4] = [
        Self::Background,
        Self::Research,
        Self::Characters,
        Self::Outline,
    ];

    #[must_use]
    pub const fn stable_key(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Research => "research",
            Self::Characters => "characters",
            Self::Outline => "outline",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Background => "背景设定",
            Self::Research => "资料收集",
            Self::Characters => "人物设定",
            Self::Outline => "小说粗纲",
        }
    }
}

/// 一个尚未分配稳定 ID 的章节计划输入。
///
/// 输入的数组顺序就是章节顺序。服务端在首次提交粗纲时生成稳定的
/// `plan_item_id`，使模型和人工输入都不能通过客户端排序悄悄改变计划。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPlanDraft {
    pub title: String,
    pub goal: String,
}

impl ChapterPlanDraft {
    /// # Errors
    ///
    /// 标题或目标为空，或超出各自长度限制时返回 [`ValidationError`]。
    pub fn new(title: impl AsRef<str>, goal: impl AsRef<str>) -> Result<Self, ValidationError> {
        Ok(Self {
            title: validate_required_text("章节标题", title.as_ref(), MAX_CHAPTER_TITLE_LEN)?,
            goal: validate_document_content("章节目标", goal.as_ref(), MAX_CHAPTER_GOAL_LEN)?,
        })
    }
}

/// 用户在正文完本后新增的一篇番外。
///
/// 番外不属于冻结的正文章节计划，因此只保存独立标题；后续仍使用同一套章节
/// 指导、细纲、正文和终稿审核流程。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewExtraChapter {
    pub title: String,
}

impl NewExtraChapter {
    /// 创建并校验一篇番外的标题。
    ///
    /// # Errors
    ///
    /// 标题为空或超过章节标题长度上限时返回 [`ValidationError`]。
    pub fn new(title: impl AsRef<str>) -> Result<Self, ValidationError> {
        Ok(Self {
            title: validate_required_text("番外标题", title.as_ref(), MAX_CHAPTER_TITLE_LEN)?,
        })
    }
}

/// 经过服务端校验和编号的章节计划项。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPlanItem {
    pub plan_item_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub order_index: i64,
    pub title: String,
    pub goal: String,
}

/// 粗纲附带的类型化章节计划。
///
/// 计划在准备阶段审核通过前只属于该粗纲修订；通过后才在同一事务中物化为
/// chapters 行，之后正文流程不得改写其顺序或内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPlan {
    pub items: Vec<ChapterPlanItem>,
}

impl ChapterPlan {
    /// 从一次提交的有序草案建立稳定章节计划。
    ///
    /// # Errors
    ///
    /// 空计划或计划项过多时返回 [`ValidationError`]。
    pub fn from_drafts(drafts: &[ChapterPlanDraft]) -> Result<Self, ValidationError> {
        if drafts.is_empty() {
            return Err(ValidationError::Required {
                field: "章节计划"
            });
        }
        if drafts.len() > MAX_CHAPTER_PLAN_ITEMS {
            return Err(ValidationError::TooLong {
                field: "章节计划项",
                max: MAX_CHAPTER_PLAN_ITEMS,
            });
        }
        Ok(Self {
            items: drafts
                .iter()
                .enumerate()
                .map(|(index, draft)| {
                    let normalized = ChapterPlanDraft::new(&draft.title, &draft.goal)?;
                    Ok(ChapterPlanItem {
                        plan_item_id: Uuid::new_v4(),
                        order_index: (index + 1) as i64,
                        title: normalized.title,
                        goal: normalized.goal,
                    })
                })
                .collect::<Result<Vec<_>, ValidationError>>()?,
        })
    }

    /// 验证从持久化 JSON 重新读取的计划仍符合领域约束。
    ///
    /// # Errors
    ///
    /// 当计划为空、顺序不连续、ID 重复或文本失效时返回 [`ValidationError`]。
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.items.is_empty() {
            return Err(ValidationError::Required {
                field: "章节计划"
            });
        }
        if self.items.len() > MAX_CHAPTER_PLAN_ITEMS {
            return Err(ValidationError::TooLong {
                field: "章节计划项",
                max: MAX_CHAPTER_PLAN_ITEMS,
            });
        }
        let mut seen_ids = std::collections::HashSet::with_capacity(self.items.len());
        for (index, item) in self.items.iter().enumerate() {
            if item.order_index != (index + 1) as i64 || !seen_ids.insert(item.plan_item_id) {
                return Err(ValidationError::Required {
                    field: "章节计划顺序",
                });
            }
            let _ = ChapterPlanDraft::new(&item.title, &item.goal)?;
        }
        Ok(())
    }
}

/// 将结构化计划渲染成审核与上下文折叠共用的纯文本。
#[must_use]
pub fn render_chapter_plan_plaintext(plan: &ChapterPlan) -> String {
    plan.items
        .iter()
        .map(|item| format!("第{}章：{}\n{}", item.order_index, item.title, item.goal))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 章节所属的作品部分。M1 只会创建 main，extra 为后续番外流程预留。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterKind {
    Main,
    Extra,
}

impl ChapterKind {
    #[must_use]
    pub const fn stable_key(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Extra => "extra",
        }
    }
}

/// 章节自身的不可变进度标记；更细的运行状态仍以 [`WorkflowSnapshot`] 为准。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterStatus {
    Planned,
    InProgress,
    Approved,
}

impl ChapterStatus {
    #[must_use]
    pub const fn stable_key(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::InProgress => "in_progress",
            Self::Approved => "approved",
        }
    }
}

/// 章节目录需要的持久化视图，不包含正文内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterSummary {
    pub id: ChapterId,
    pub version_id: VersionId,
    pub kind: ChapterKind,
    pub plan_item_id: Option<Uuid>,
    #[serde(with = "decimal_i64")]
    pub order_index: i64,
    pub title: String,
    /// 目录中的显示名，与阅读区和导出使用同一套命名。
    pub label: String,
    /// 当前已提交正文的字数；本章尚未写过正文时为 0。阅读区用它估计每章高度，
    /// 因此跳到远端的章节不需要先把中间所有章节渲染一遍。
    #[serde(with = "decimal_i64")]
    pub body_word_count: i64,
    pub status: ChapterStatus,
    pub approved_revision_id: Option<RevisionId>,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
}

/// 已审核通过、可作为阶段折叠来源的一份产物修订。
///
/// `content` 保持修订原文；折叠函数不会补标题、ID、角色或摘要。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedPreparationArtifact {
    pub kind: PreparationArtifactKind,
    pub revision_id: RevisionId,
    pub content: String,
    /// 只有粗纲携带结构化章节计划。它会被渲染成与审核一致的纯文本，而不会
    /// 作为 JSON 信封混入后续 Agent 上下文。
    pub chapter_plan: Option<ChapterPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewNovelBrief {
    pub content: String,
}

impl NewNovelBrief {
    /// 创建一份保留原文格式的小说目标。
    ///
    /// # Errors
    ///
    /// 当内容仅含空白或超过目标长度上限时返回 [`ValidationError`]。
    pub fn new(content: impl AsRef<str>) -> Result<Self, ValidationError> {
        Ok(Self {
            content: validate_document_content("小说目标", content.as_ref(), MAX_NOVEL_BRIEF_LEN)?,
        })
    }
}

/// 一次准备提交中的一个产物。正文保留原样，不在输入层偷偷截断、摘要或去除
/// 首尾换行；是否为空只以 Unicode 空白检查。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparationArtifactInput {
    pub kind: PreparationArtifactKind,
    pub content: String,
}

impl PreparationArtifactInput {
    /// 创建一份保留原文格式的准备产物输入。
    ///
    /// # Errors
    ///
    /// 当内容仅含空白或超过该产物的长度上限时返回 [`ValidationError`]。
    pub fn new(
        kind: PreparationArtifactKind,
        content: impl AsRef<str>,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            kind,
            content: validate_document_content(
                kind.label(),
                content.as_ref(),
                MAX_ARTIFACT_CONTENT_LEN,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HumanReviewDecision {
    pub passed: bool,
    pub issues: String,
}

impl HumanReviewDecision {
    /// 创建一项人工审核决定。
    ///
    /// # Errors
    ///
    /// 当拒绝决定没有非空理由，或理由超过长度上限时返回 [`ValidationError`]。
    pub fn new(passed: bool, issues: impl AsRef<str>) -> Result<Self, ValidationError> {
        let issues = issues.as_ref();
        if !passed && issues.trim().is_empty() {
            return Err(ValidationError::Required {
                field: "拒绝原因"
            });
        }
        if issues.chars().count() > MAX_REVIEW_ISSUES_LEN {
            return Err(ValidationError::TooLong {
                field: "审核意见",
                max: MAX_REVIEW_ISSUES_LEN,
            });
        }
        Ok(Self {
            passed,
            issues: issues.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Open,
    /// 已创建审核 Agent 子 Run，等待该次委托的确定结果。
    Delegated,
    Approved,
    Rejected,
    Stale,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparationReview {
    pub id: ReviewId,
    pub kind: PreparationArtifactKind,
    pub target_revision_id: RevisionId,
    pub status: ReviewStatus,
    pub passed: Option<bool>,
    pub confidence: Option<serde_json::Number>,
    pub issues: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentPreparationArtifact {
    pub document_id: DocumentId,
    pub kind: PreparationArtifactKind,
    pub revision_id: Option<RevisionId>,
    #[serde(with = "option_decimal_i64")]
    pub revision_no: Option<i64>,
    pub content: Option<String>,
    pub chapter_plan: Option<ChapterPlan>,
    pub review: Option<PreparationReview>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentNovelBrief {
    pub document_id: DocumentId,
    pub revision_id: Option<RevisionId>,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFold {
    pub artifact_revision_ids: Vec<RevisionId>,
    #[serde(with = "decimal_i64")]
    pub through_message_sequence: i64,
    pub plain_text: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparationSnapshot {
    pub version_id: VersionId,
    pub workflow: WorkflowSnapshot,
    pub novel_brief: CurrentNovelBrief,
    pub artifacts: Vec<CurrentPreparationArtifact>,
    pub pending_question: Option<PendingUserQuestion>,
    pub context_fold: Option<ContextFold>,
}

/// 章节细纲工作区的读取模型。
///
/// `context_fold` 只会在当前细纲审核通过后出现；其内容是细纲原文，而不含审核
/// 意见、运行状态或任何内部 JSON。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterOutlineSnapshot {
    pub chapter_id: ChapterId,
    pub workflow: WorkflowSnapshot,
    pub guidance: CurrentChapterGuidance,
    pub outline: CurrentChapterOutline,
    pub pending_question: Option<PendingUserQuestion>,
    pub context_fold: Option<ContextFold>,
}

/// 章节正文工作区的读取模型。
///
/// `outline_context_fold` 是已经通过的细纲纯文本；`context_fold` 只会在整章终稿
/// 审核通过后出现，内容为通过的正文全文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterBodySnapshot {
    pub chapter_id: ChapterId,
    pub workflow: WorkflowSnapshot,
    pub body: CurrentChapterBody,
    pub pending_proposal: Option<PendingChapterBodyProposal>,
    pub pending_proposal_review: Option<ChapterEditReview>,
    /// 当前正文 Run 最近一次已经决定的逐次编辑审核，用于展示审核意见和置信度。
    /// 有新的待审提案时由 `pending_proposal_review` 取代，避免混淆两个候选。
    pub last_proposal_review: Option<ChapterEditReview>,
    pub held_tool: Option<HeldChapterBodyTool>,
    pub held_submission: Option<HeldChapterBodySubmission>,
    pub pending_question: Option<PendingUserQuestion>,
    pub pending_feedback: Vec<PendingChapterBodyFeedback>,
    pub delivered_feedback: Vec<DeliveredChapterBodyFeedback>,
    pub outline_context_fold: Option<ContextFold>,
    pub context_fold: Option<ContextFold>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ArtifactFoldError {
    #[error("准备产物 {kind} 缺失，不能折叠上下文")]
    MissingArtifact { kind: &'static str },
    #[error("准备产物 {kind} 出现多个当前修订，不能折叠上下文")]
    DuplicateArtifact { kind: &'static str },
    #[error("准备产物 {kind} 为空，不能折叠上下文")]
    EmptyArtifact { kind: &'static str },
    #[error("小说粗纲缺少有效的结构化章节计划，不能折叠上下文")]
    MissingChapterPlan,
}

/// 将四项已经审核通过的准备产物渲染为模型可见的纯文本折叠块。
///
/// 这刻意不接受消息、审核意见或运行元数据：阶段通过后，后续 Agent 只能看见
/// 各产物的完整原文。输入顺序不影响输出；四项之间仅由本函数追加两个 LF。
///
/// # Errors
///
/// 当任一必需产物缺失、重复或为空时返回 [`ArtifactFoldError`]。
pub fn render_preparation_artifact_plaintext(
    artifacts: &[ApprovedPreparationArtifact],
) -> Result<String, ArtifactFoldError> {
    let mut ordered = Vec::with_capacity(PreparationArtifactKind::ALL.len());
    for kind in PreparationArtifactKind::ALL {
        let matching = artifacts
            .iter()
            .filter(|artifact| artifact.kind == kind)
            .collect::<Vec<_>>();
        let [artifact] = matching.as_slice() else {
            return if matching.is_empty() {
                Err(ArtifactFoldError::MissingArtifact {
                    kind: kind.stable_key(),
                })
            } else {
                Err(ArtifactFoldError::DuplicateArtifact {
                    kind: kind.stable_key(),
                })
            };
        };
        if artifact.content.trim().is_empty() {
            return Err(ArtifactFoldError::EmptyArtifact {
                kind: kind.stable_key(),
            });
        }
        ordered.push(match kind {
            PreparationArtifactKind::Outline => {
                let plan = artifact
                    .chapter_plan
                    .as_ref()
                    .ok_or(ArtifactFoldError::MissingChapterPlan)?;
                plan.validate()
                    .map_err(|_| ArtifactFoldError::MissingChapterPlan)?;
                format!(
                    "{}\n\n{}",
                    artifact.content,
                    render_chapter_plan_plaintext(plan)
                )
            }
            _ => artifact.content.clone(),
        });
    }
    Ok(ordered.join("\n\n"))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub selected_version_id: VersionId,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 模型思考等级。`Off` 表示不向模型发送思考参数。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    pub const ALL: [Self; 4] = [Self::Off, Self::Low, Self::Medium, Self::High];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// 项目级辅助工具预算的默认值（细则 §7.1 的初始建议值）。
pub const DEFAULT_AUXILIARY_TOOL_BUDGET: i64 = 10;
/// 单个项目辅助工具预算的上限。
pub const MAX_AUXILIARY_TOOL_BUDGET: i64 = 10_000;
/// 项目覆盖模型 ID 的长度上限。
pub const MAX_PROJECT_MODEL_ID_LEN: usize = 256;

const fn default_auxiliary_tool_budget() -> i64 {
    DEFAULT_AUXILIARY_TOOL_BUDGET
}

/// 模型能力资料：描述一个具体模型能做什么。
///
/// 这份资料属于「模型」而不是「某一级设置」：系统写作模型与项目模型引用都携带
/// 它，因此调度器、Provider 与命令前置检查必须从**有效模型**取用，不能把系统模型
/// 的资料套用到项目引用的模型上。
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilityProfile {
    pub streaming: bool,
    pub tools: bool,
    pub structured_output: bool,
    pub stream_usage: bool,
    /// 是否对该模型验证过显式思考参数（`reasoning_effort`）。
    ///
    /// 缺省为 `false`：较早保存的能力资料没有该字段时按“未经验证”处理，而不是把
    /// 未知能力当作可用。
    ///
    /// 服务端只保存这个声明，不重验历史：验证由连接测试完成（`POST
    /// /api/settings/test-model` 携带 `thinking: true` 时会真的发出 `reasoning_effort`，
    /// 被拒时返回明确错误），界面也只在验证通过后才允许打开该开关。
    #[serde(default)]
    pub thinking: bool,
}

/// 项目对写作模型的引用：既指定模型 ID，也声明该模型的能力资料。
///
/// 能力资料必须与模型 ID 一起声明：只有模型 ID 而没有资料，会让校验与实际请求
/// 发往两个不同的模型。项目覆盖只改变「在已配置服务上使用哪个模型」，不改变服务
/// 地址与凭据。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectModelReference {
    pub model_id: String,
    pub capabilities: ModelCapabilityProfile,
}

/// 项目级创作设置。
///
/// 字段级缺省是真实语义：数据库中较早写入的 `{}` 或缺失字段表示“未显式设置”，
/// 应解析为同一默认值，而不是损坏记录。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    /// 覆盖系统默认模型；`None` 表示沿用系统设置。
    #[serde(default)]
    pub model: Option<ProjectModelReference>,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default = "default_auxiliary_tool_budget", with = "decimal_i64")]
    pub auxiliary_tool_budget: i64,
    /// 与项目行的 `settings_revision` 一致，仅用于让快照自描述。
    #[serde(default, with = "decimal_i64")]
    pub revision: i64,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            model: None,
            thinking_level: ThinkingLevel::Off,
            auxiliary_tool_budget: DEFAULT_AUXILIARY_TOOL_BUDGET,
            revision: 0,
        }
    }
}

/// 待保存的项目设置；并发修订由调用方另行传入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProjectSettings {
    pub model: Option<ProjectModelReference>,
    pub thinking_level: ThinkingLevel,
    #[serde(with = "decimal_i64")]
    pub auxiliary_tool_budget: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelVersion {
    pub id: VersionId,
    pub project_id: ProjectId,
    pub label: String,
    pub novel_title: String,
    pub source_version_id: Option<VersionId>,
    #[serde(with = "option_decimal_i64")]
    pub body_completed_at_ms: Option<i64>,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStage {
    Initializing,
    PreparingGenerating,
    PreparingReviewing,
    PreparingRevising,
    WritingChapterReady,
    WritingOutlineGenerating,
    WritingOutlineReviewing,
    WritingDrafting,
    WritingChapterReviewing,
    WritingChapterDone,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Idle,
    Queued,
    RequestInFlight,
    ToolBoundary,
    WaitingAnswer,
    WaitingReview,
    Paused,
    Interrupted,
    Failed,
    Stopped,
}

/// 一个自动写作 Run 停止后必须向用户说明的明确原因。
///
/// 这不是错误文本的替代品：详细诊断仍保存在对应 model request / Run 事件中；该枚举
/// 只让快照和界面能稳定地区分需要用户继续的工程边界。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunWaitReason {
    RequestLimit,
    ProtocolErrorLimit,
    UserStopped,
}

/// 复制稿本时重建的一份已通过阶段上下文来源。
///
/// 它没有对应的 Run：复制不会把源稿本的运行、消息或用量带入新稿本。`id` 只是
/// 该折叠来源在新稿本中的稳定标识，供模型请求快照追溯。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopiedContextFoldStage {
    Preparation,
    ChapterOutline,
    ChapterBody,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopiedContextFold {
    pub id: Uuid,
    pub stage: CopiedContextFoldStage,
    #[serde(with = "option_decimal_i64")]
    pub chapter_order: Option<i64>,
    pub fold: ContextFold,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSnapshot {
    pub version_id: VersionId,
    pub stage: WorkflowStage,
    pub execution: ExecutionStatus,
    #[serde(with = "decimal_i64")]
    pub state_revision: i64,
    pub active_run_id: Option<RunId>,
    pub active_chapter_id: Option<ChapterId>,
    pub approval_mode: ApprovalMode,
    pub pause_requested: bool,
    pub wait_reason: Option<RunWaitReason>,
    /// 用户显式停止时保留的稳定交互边界。再次继续不能把待答问题、待审提案或
    /// 已暂停的工具调用误变成新的模型请求。
    pub resume_execution: Option<ExecutionStatus>,
    /// 复制已通过内容时重建的上下文来源。普通写作 Run 的折叠仍保存在各自的
    /// 检查点中，不能重复写入这里。
    pub copied_context_folds: Vec<CopiedContextFold>,
}

impl WorkflowSnapshot {
    #[must_use]
    pub fn initial(version_id: VersionId) -> Self {
        Self {
            version_id,
            stage: WorkflowStage::Initializing,
            execution: ExecutionStatus::Idle,
            state_revision: 0,
            active_run_id: None,
            active_chapter_id: None,
            approval_mode: ApprovalMode::AutoApprove,
            pause_requested: false,
            wait_reason: None,
            resume_execution: None,
            copied_context_folds: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparationReviewCompletion {
    AllApproved,
    RevisionsRequired,
}

/// 当前章节细纲审核的终态。
///
/// 细纲只有一个当前修订和一条开放审核，因此不像准备阶段那样需要等待批次内
/// 的其余审核决定；但通过前仍必须由存储层校验目标修订和输入依赖没有变化。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChapterOutlineReviewCompletion {
    Approved,
    RevisionsRequired,
}

/// 当前章节整章终稿审核的终态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChapterFinalReviewCompletion {
    Approved,
    RevisionsRequired,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum WorkflowTransitionError {
    #[error("当前工作流状态 {stage:?} 不允许执行 {action}")]
    InvalidTransition {
        stage: WorkflowStage,
        action: &'static str,
    },
}

/// 模型请求从调度队列真正取得许可后的执行转换。
///
/// 只有当前工作流拥有的顶层写作 Run 可以发起请求；`queued` 是唯一允许的
/// 起点，防止同一 Run 在网络调用尚未结束时重复扣减请求额度。
///
/// # Errors
///
/// Run 不属于当前工作流、执行轴不在 `queued`，或工作流并非可生成阶段时返回
/// [`WorkflowTransitionError`]。
pub fn dispatch_model_request(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id) || workflow.execution != ExecutionStatus::Queued {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "dispatch_model_request",
        });
    }
    transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "dispatch_model_request",
        workflow.stage.clone(),
        ExecutionStatus::RequestInFlight,
    )
}

/// 在网络请求尚未封存前终止一个已经排队的模型 Run。
///
/// 这覆盖“已取得调度许可，但最新设置、凭据或上下文无法组成请求”的情况。此时
/// 不存在 `model_requests` 记录，因此不能伪造一次已发送请求；Run 与工作流仍必须
/// 一起离开 `queued`，避免界面永久显示执行中。
///
/// # Errors
///
/// Run 不属于当前工作流、执行轴不在 `queued`，或工作流并非可生成阶段时返回
/// [`WorkflowTransitionError`]。
pub fn fail_queued_model_run(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id) || workflow.execution != ExecutionStatus::Queued {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "fail_queued_model_run",
        });
    }
    transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "fail_queued_model_run",
        workflow.stage.clone(),
        ExecutionStatus::Failed,
    )
}

/// 因请求额度耗尽等明确工程边界停止一个尚未发送的排队 Run。
///
/// 与失败不同，`stopped` 表示用户可以在修正或确认后显式继续并领取下一轮额度；
/// 它不会删除已有上下文、工具结果或意见。
///
/// # Errors
///
/// Run 不属于当前工作流、执行轴不在 `queued`，或当前阶段不需要模型写作时返回
/// [`WorkflowTransitionError`]。
pub fn stop_queued_model_run(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id) || workflow.execution != ExecutionStatus::Queued {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "stop_queued_model_run",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "stop_queued_model_run",
        workflow.stage.clone(),
        ExecutionStatus::Stopped,
    )?;
    next.wait_reason = Some(RunWaitReason::RequestLimit);
    next.resume_execution = Some(ExecutionStatus::Queued);
    Ok(next)
}

/// 用户显式停止当前自动写作 Run。
///
/// 停止不是暂停：它阻止后续模型请求，并由存储层作废当前 request。稳定的人机边界
/// （待答问题、待审正文提案、已暂停工具调用）会记录在 `resume_execution`，显式继续
/// 后仍回到原边界，绝不自动提交或应用它们。
///
/// # Errors
///
/// Run 不是当前活跃写作 Run，或执行轴已经是终态/空闲时返回
/// [`WorkflowTransitionError`]。
pub fn stop_writing_run(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || !matches!(
            workflow.execution,
            ExecutionStatus::Queued
                | ExecutionStatus::RequestInFlight
                | ExecutionStatus::ToolBoundary
                | ExecutionStatus::WaitingAnswer
                | ExecutionStatus::WaitingReview
                | ExecutionStatus::Paused
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "stop_writing_run",
        });
    }
    let stage = workflow.stage.clone();
    let mut next = transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "stop_writing_run",
        stage,
        ExecutionStatus::Stopped,
    )?;
    next.wait_reason = Some(RunWaitReason::UserStopped);
    next.resume_execution = Some(workflow.execution.clone());
    Ok(next)
}

/// 在进程退出或调度器丢失执行上下文后，把尚未形成稳定交互边界的写作 Run 标记为
/// 已中断。
///
/// 已等待问题、待审提案和已暂停的 Run 都有可直接恢复的持久化交互边界，不经过本
/// 转换。这里仅覆盖尚未再次请求模型的 `queued`、模型在途的
/// `request_in_flight` 和极短暂的 `tool_boundary` 状态。
///
/// # Errors
///
/// Run 不是当前活跃 Run、执行轴不是自动运行状态，或阶段不属于可生成写作阶段时
/// 返回 [`WorkflowTransitionError`]。
pub fn interrupt_writing_run(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || !matches!(
            workflow.execution,
            ExecutionStatus::Queued
                | ExecutionStatus::RequestInFlight
                | ExecutionStatus::ToolBoundary
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "interrupt_writing_run",
        });
    }
    transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "interrupt_writing_run",
        workflow.stage.clone(),
        ExecutionStatus::Interrupted,
    )
}

/// 用户显式继续一个失败、停止或中断的自动写作 Run。
///
/// 这只恢复能请求模型的阶段，保留 Run、消息账本、工具结果和暂停意图。额度与
/// `resume_count` 的更新属于 Store 事务；Core 仅确保工作流不会把普通暂停或审核
/// 等待误当成一次续授。
///
/// # Errors
///
/// Run 不是当前活跃 Run、执行轴不是可显式继续的终态，或阶段不属于可生成写作阶段
/// 时返回 [`WorkflowTransitionError`]。
pub fn continue_writing_run(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || !matches!(
            workflow.execution,
            ExecutionStatus::Interrupted | ExecutionStatus::Failed | ExecutionStatus::Stopped
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_writing_run",
        });
    }
    let execution = if workflow.execution == ExecutionStatus::Stopped {
        match workflow.resume_execution.as_ref() {
            Some(
                ExecutionStatus::WaitingAnswer
                | ExecutionStatus::WaitingReview
                | ExecutionStatus::Paused,
            ) => workflow.resume_execution.clone().unwrap(),
            _ => ExecutionStatus::Queued,
        }
    } else {
        ExecutionStatus::Queued
    };
    transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "continue_writing_run",
        workflow.stage.clone(),
        execution,
    )
}

/// 模型网络调用结束后离开 `request_in_flight`。
///
/// 成功响应先落在 `tool_boundary`，由应用服务继续处理工具调用或阶段提交；网络或
/// 协议不确定性则显式落在 `failed`，绝不把失败包装为可继续生成的成功状态。
///
/// # Errors
///
/// Run 不属于当前工作流、执行轴不在 `request_in_flight`，或工作流不是可生成
/// 阶段时返回 [`WorkflowTransitionError`]。
pub fn complete_model_request(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
    completion: ModelRequestCompletion,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "complete_model_request",
        });
    }
    let execution = match completion {
        ModelRequestCompletion::Succeeded => ExecutionStatus::ToolBoundary,
        ModelRequestCompletion::Failed | ModelRequestCompletion::Unknown => ExecutionStatus::Failed,
    };
    transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "complete_model_request",
        workflow.stage.clone(),
        execution,
    )
}

/// 已 dispatch 模型请求的终态类别。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRequestCompletion {
    Succeeded,
    Failed,
    Unknown,
}

/// 从初始化状态启动准备 Agent。目标输入与文档槽是否齐全由应用服务在同一事务中
/// 校验，Core 只负责纯状态转换。
///
/// # Errors
///
/// 当工作流不处于初始化阶段时返回 [`WorkflowTransitionError`]。
pub fn begin_preparation(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::Initializing],
        "begin_preparation",
        WorkflowStage::PreparingGenerating,
        ExecutionStatus::Queued,
    )?;
    next.active_run_id = Some(run_id);
    Ok(next)
}

/// 一次准备产物提交完成后进入集中审核。无论产物是初次生成还是修订轮生成，
/// 审核阶段都使用相同状态，并由持久化的 Review 行决定具体待审项。
///
/// # Errors
///
/// 当工作流不处于准备生成或准备修订阶段时返回 [`WorkflowTransitionError`]。
pub fn submit_preparation_for_review(
    workflow: &WorkflowSnapshot,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    transition_workflow(
        workflow,
        &[
            WorkflowStage::PreparingGenerating,
            WorkflowStage::PreparingRevising,
        ],
        "submit_preparation_for_review",
        WorkflowStage::PreparingReviewing,
        ExecutionStatus::WaitingReview,
    )
}

/// 记录一项人工或受托审核的终态，但尚未替整个审核批次推进阶段。
/// 每次决定都会递增工作流修订，使并发页面必须基于最新快照继续操作。
///
/// # Errors
///
/// 当工作流不处于准备审核阶段时返回 [`WorkflowTransitionError`]。
pub fn record_preparation_review_decision(
    workflow: &WorkflowSnapshot,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    transition_workflow(
        workflow,
        &[WorkflowStage::PreparingReviewing],
        "record_preparation_review_decision",
        WorkflowStage::PreparingReviewing,
        ExecutionStatus::WaitingReview,
    )
}

/// 全部本轮准备审核已经有终态后推进工作流。`AllApproved` 的调用方必须已在同一
/// 事务验证当前修订、依赖和折叠检查点；函数本身绝不把“部分通过”解释为阶段通过。
///
/// # Errors
///
/// 当工作流不处于准备审核阶段时返回 [`WorkflowTransitionError`]。
pub fn finish_preparation_review(
    workflow: &WorkflowSnapshot,
    completion: PreparationReviewCompletion,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    let (next_stage, execution) = match completion {
        PreparationReviewCompletion::AllApproved => {
            (WorkflowStage::WritingChapterReady, ExecutionStatus::Idle)
        }
        PreparationReviewCompletion::RevisionsRequired => {
            (WorkflowStage::PreparingRevising, ExecutionStatus::Queued)
        }
    };
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::PreparingReviewing],
        "finish_preparation_review",
        next_stage,
        execution,
    )?;
    if completion == PreparationReviewCompletion::AllApproved {
        next.active_run_id = None;
    }
    Ok(next)
}

/// 保存当前准备就绪章节的指导输入，并产生新的工作流修订。
///
/// # Errors
///
/// 工作流不在章节指导界面，或输入的章节不是当前章节时返回
/// [`WorkflowTransitionError`]。
pub fn record_chapter_guidance(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "record_chapter_guidance",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterReady],
        "record_chapter_guidance",
        WorkflowStage::WritingChapterReady,
        ExecutionStatus::Idle,
    )
}

/// 为当前章节建立细纲 Run，并进入等待模型调度的细纲阶段。
///
/// # Errors
///
/// 工作流不在章节指导界面，或章节不是当前章节时返回
/// [`WorkflowTransitionError`]。
pub fn begin_chapter_outline(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "begin_chapter_outline",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterReady],
        "begin_chapter_outline",
        WorkflowStage::WritingOutlineGenerating,
        ExecutionStatus::Queued,
    )?;
    next.active_run_id = Some(run_id);
    Ok(next)
}

/// 将当前章的一份完整细纲提交到审核队列。
///
/// # Errors
///
/// 当前章节不匹配，或工作流不在细纲生成状态时返回
/// [`WorkflowTransitionError`]。
pub fn submit_chapter_outline_for_review(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "submit_chapter_outline_for_review",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingOutlineGenerating],
        "submit_chapter_outline_for_review",
        WorkflowStage::WritingOutlineReviewing,
        ExecutionStatus::WaitingReview,
    )
}

/// 处理当前章节细纲审核的终态。
///
/// 通过时调用方须在同一事务里写入细纲纯文本折叠和 Run 成功状态；拒绝时复用
/// 原细纲 Run 与辅助工具预算，并重新进入可继续生成的排队状态。
///
/// # Errors
///
/// 当前章节不匹配，或工作流不在细纲审核状态时返回
/// [`WorkflowTransitionError`]。
pub fn finish_chapter_outline_review(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    completion: ChapterOutlineReviewCompletion,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "finish_chapter_outline_review",
        });
    }
    let (stage, execution) = match completion {
        ChapterOutlineReviewCompletion::Approved => {
            (WorkflowStage::WritingDrafting, ExecutionStatus::Idle)
        }
        ChapterOutlineReviewCompletion::RevisionsRequired => (
            WorkflowStage::WritingOutlineGenerating,
            ExecutionStatus::Queued,
        ),
    };
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingOutlineReviewing],
        "finish_chapter_outline_review",
        stage,
        execution,
    )?;
    if completion == ChapterOutlineReviewCompletion::Approved {
        next.active_run_id = None;
    }
    Ok(next)
}

/// 为已经通过细纲的当前章节安排正文 Run。
///
/// 细纲审核通过和正文 Run 创建可位于同一存储事务，但它们是两个清晰的领域转换：
/// 前者封存细纲，后者才取得正文阶段的执行所有权。
///
/// # Errors
///
/// 当前章节不匹配、工作流不在正文阶段，或已有活动 Run 时返回
/// [`WorkflowTransitionError`]。
pub fn schedule_chapter_body_run(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) || workflow.active_run_id.is_some() {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "schedule_chapter_body_run",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "schedule_chapter_body_run",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Queued,
    )?;
    next.active_run_id = Some(run_id);
    Ok(next)
}

/// 记录一次已自动接受的正文修订。
///
/// M1 的正文编辑直接形成修订；M2 的待审提案门控会在调用此转换前决定是否允许
/// 提交，但不会改变正文修订与工作流版本递增的语义。
///
/// 用户在模型回合进行中（`Queued` 或 `RequestInFlight`）编辑正文时，执行轴保持
/// 不变：那个回合仍要沿原有回调路径收束（模型编辑按最新正文复验，业务失败则
/// 排下一轮）。否则工作流会停在既没有在途请求、也没有任何继续入口的空闲态。
///
/// # Errors
///
/// 当前章节不匹配或不在正文阶段时返回 [`WorkflowTransitionError`]。
pub fn record_chapter_body_edit(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "record_chapter_body_edit",
        });
    }
    let execution = match workflow.execution {
        ExecutionStatus::Queued | ExecutionStatus::RequestInFlight => workflow.execution.clone(),
        _ => ExecutionStatus::Idle,
    };
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "record_chapter_body_edit",
        WorkflowStage::WritingDrafting,
        execution,
    )
}

/// 在正文工具调用已经被完整接收并原子处理后，为同一 Run 排队下一轮模型请求。
///
/// 工具调用的正文修订（或可回送模型的业务错误）、请求终态和这次转换由存储层放在
/// 同一个事务中提交。这里不把中间的 `tool_boundary` 暴露给其他读取者：它只定义
/// `request_in_flight → queued` 的最终可恢复状态，下一轮仍须重新取得调度许可。
///
/// # Errors
///
/// 当前 Run、章节或执行轴不匹配时返回 [`WorkflowTransitionError`]，从而拒绝迟到
/// 的模型回调重复应用已经处理过的工具调用。
pub fn continue_chapter_body_after_tool(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_chapter_body_after_tool",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "continue_chapter_body_after_tool",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Queued,
    )
}

/// 让一个已经形成稳定结果的写作工具调用继续到下一轮请求。
///
/// 正文编辑之外的 `ask_user` 工具同样需要这一转换：工具结果必须先作为配对消息
/// 持久化，随后才可重新排队。阶段保持不变，正文的暂停意图也不会被此函数清除。
///
/// # Errors
///
/// 当前 Run 不活跃、执行轴不是 `request_in_flight`，或阶段不属于可调用写作工具的
/// 集合时返回 [`WorkflowTransitionError`]。
pub fn continue_after_agent_tool(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_after_agent_tool",
        });
    }
    let stage = workflow.stage.clone();
    transition_workflow(
        workflow,
        &[
            WorkflowStage::PreparingGenerating,
            WorkflowStage::PreparingRevising,
            WorkflowStage::WritingOutlineGenerating,
            WorkflowStage::WritingDrafting,
        ],
        "continue_after_agent_tool",
        stage,
        ExecutionStatus::Queued,
    )
}

/// 因连续协议错误达到工程上限，停止当前已完成网络响应的写作 Run。
///
/// 这是模型响应线性化后的终态：该请求及其原始 assistant 消息、配对工具错误（如
/// 有）和协议纠正消息都已由存储层在同一事务中保存。用户显式继续同一 Run 时才会
/// 重新授予请求额度并回到 `queued`。
///
/// # Errors
///
/// 当前 Run 不活跃、执行轴不是 `request_in_flight`，或阶段不属于自动写作阶段时
/// 返回 [`WorkflowTransitionError`]。
pub fn stop_writing_after_protocol_error(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "stop_writing_after_protocol_error",
        });
    }
    let stage = workflow.stage.clone();
    let mut next = transition_workflow(
        workflow,
        &model_request_workflow_stages(),
        "stop_writing_after_protocol_error",
        stage,
        ExecutionStatus::Stopped,
    )?;
    next.wait_reason = Some(RunWaitReason::ProtocolErrorLimit);
    next.resume_execution = Some(ExecutionStatus::RequestInFlight);
    Ok(next)
}

/// 将当前模型请求停在一组必须由用户回答的问题上。
///
/// 询问不会改变阶段，只把执行轴从 `request_in_flight` 切到 `waiting_answer`。正文
/// 的暂停意图会保留，因而答案随后产生的工具结果仍会遵循原本的下一工具边界规则。
///
/// # Errors
///
/// 当前 Run 不活跃、不是可请求用户输入的写作阶段，或模型请求已经结束时返回
/// [`WorkflowTransitionError`]。
pub fn wait_for_user_answer(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "wait_for_user_answer",
        });
    }
    let stage = workflow.stage.clone();
    transition_workflow(
        workflow,
        &[
            WorkflowStage::PreparingGenerating,
            WorkflowStage::PreparingRevising,
            WorkflowStage::WritingOutlineGenerating,
            WorkflowStage::WritingDrafting,
        ],
        "wait_for_user_answer",
        stage,
        ExecutionStatus::WaitingAnswer,
    )
}

/// 将已验证的用户答案作为工具结果回送，并让同一 Run 再次进入可请求状态。
///
/// # Errors
///
/// 当前 Run 不活跃、执行轴不在 `waiting_answer`，或阶段已经离开可询问阶段时返回
/// [`WorkflowTransitionError`]。
pub fn continue_after_user_answer(
    workflow: &WorkflowSnapshot,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::WaitingAnswer
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_after_user_answer",
        });
    }
    let stage = workflow.stage.clone();
    transition_workflow(
        workflow,
        &[
            WorkflowStage::PreparingGenerating,
            WorkflowStage::PreparingRevising,
            WorkflowStage::WritingOutlineGenerating,
            WorkflowStage::WritingDrafting,
        ],
        "continue_after_user_answer",
        stage,
        ExecutionStatus::Queued,
    )
}

/// 因阶段提交没有满足完成守卫而要求 Agent 继续正文工具循环。
///
/// 例如仍有未投递自由意见时，结构化“提交本章”不能直接绕过意见批次；Store 会保留
/// 原始助手文本并追加协议纠正消息，再使用本转换恢复同一 Run 的排队状态。
///
/// # Errors
///
/// 当前 Run、章节或执行轴不匹配时返回 [`WorkflowTransitionError`]。
pub fn continue_chapter_body_after_protocol_correction(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_chapter_body_after_protocol_correction",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "continue_chapter_body_after_protocol_correction",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Queued,
    )
}

/// 将一条完整的正文编辑调用保留为待审提案。
///
/// 该转换只用于 `ApproveRequired`：模型响应已经完整接收，但候选正文还没有成为
/// 正式修订，因而同一 Run 必须在用户决定前停在 `waiting_review`。
///
/// # Errors
///
/// 当前 Run、章节、阶段或执行轴不匹配时返回
/// [`WorkflowTransitionError`]，从而拒绝迟到的模型回调创建新的待审提案。
pub fn hold_chapter_body_proposal(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "hold_chapter_body_proposal",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "hold_chapter_body_proposal",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::WaitingReview,
    )
}

/// 用户对正文编辑提案作出接受或拒绝决定后，重新为同一 Run 排队下一轮请求。
///
/// 接受或拒绝的正文/工具结果写入由存储层负责；本函数只表达经过该门控后的工作流
/// 状态转换。它故意不依赖当前审核模式，因此切换到自动接受后，既有提案仍须显式
/// 决定。
///
/// # Errors
///
/// 当前 Run、章节、阶段或执行轴不匹配时返回
/// [`WorkflowTransitionError`]。
pub fn continue_chapter_body_after_proposal(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::WaitingReview
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "continue_chapter_body_after_proposal",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "continue_chapter_body_after_proposal",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Queued,
    )
}

/// 将当前正文提交至整章终稿审核。
///
/// # Errors
///
/// 当前章节不匹配或不在正文阶段时返回 [`WorkflowTransitionError`]。
pub fn submit_chapter_for_review(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "submit_chapter_for_review",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "submit_chapter_for_review",
        WorkflowStage::WritingChapterReviewing,
        ExecutionStatus::WaitingReview,
    )
}

/// 处理当前章节整章终稿审核的终态。
///
/// 通过时调用方需在同一事务里封存正文纯文本、锁定章节通过修订并完成正文 Run；
/// 拒绝时保留同一个 Run 与正文历史，并回到可继续写作的排队状态。
///
/// # Errors
///
/// 当前章节不匹配或不在整章审核状态时返回 [`WorkflowTransitionError`]。
pub fn finish_chapter_final_review(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    completion: ChapterFinalReviewCompletion,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "finish_chapter_final_review",
        });
    }
    let (stage, execution) = match completion {
        ChapterFinalReviewCompletion::Approved => {
            (WorkflowStage::WritingChapterDone, ExecutionStatus::Idle)
        }
        ChapterFinalReviewCompletion::RevisionsRequired => {
            (WorkflowStage::WritingDrafting, ExecutionStatus::Queued)
        }
    };
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterReviewing],
        "finish_chapter_final_review",
        stage,
        execution,
    )?;
    if completion == ChapterFinalReviewCompletion::Approved {
        next.active_run_id = None;
    }
    Ok(next)
}

/// 处理一篇番外的整章终稿审核通过。
///
/// 正文已经明确完本时，番外通过后直接回到 `completed`，这样正文完本标记不会被
/// 改写，用户也能继续新增下一篇番外。退回修订仍由
/// [`finish_chapter_final_review`] 处理，以复用原有的正文阶段转换。
///
/// # Errors
///
/// 当前章节不匹配或不在整章审核状态时返回 [`WorkflowTransitionError`]。
pub fn finish_extra_chapter_final_review(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "finish_extra_chapter_final_review",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterReviewing],
        "finish_extra_chapter_final_review",
        WorkflowStage::Completed,
        ExecutionStatus::Idle,
    )?;
    next.active_run_id = None;
    Ok(next)
}

/// 番外章被软删除后，把工作流从那一章上收回来。
///
/// 番外只能加在已完本的稿本上，因此移除一章之后既不该继续停留在「正在写这一章」，
/// 也不该让活跃章节指向一条已经不可见的记录：统一退回已完本，把活跃章节交给调用方
/// 给出的仍然可见的那一章（通常是删除前排在最后的一章）。正文计划章不走这条转换。
///
/// # Errors
///
/// 当前阶段不在番外可处的阶段内时返回 [`WorkflowTransitionError`]。
pub fn withdraw_extra_chapter(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    remaining_active: Option<ChapterId>,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    let mut next = transition_workflow(
        workflow,
        &[
            WorkflowStage::Completed,
            WorkflowStage::WritingChapterReady,
            WorkflowStage::WritingOutlineGenerating,
            WorkflowStage::WritingOutlineReviewing,
            WorkflowStage::WritingDrafting,
            WorkflowStage::WritingChapterReviewing,
        ],
        "withdraw_extra_chapter",
        WorkflowStage::Completed,
        ExecutionStatus::Idle,
    )?;
    if workflow.active_chapter_id == Some(chapter_id) {
        next.active_run_id = None;
        next.active_chapter_id = remaining_active;
    }
    Ok(next)
}

/// 在本章终稿通过后，显式将工作流切换到下一份计划章节的指导界面。
///
/// 章节目录的顺序和“下一章是否存在”由存储层在同一事务中判定；领域层只保证
/// 不会跳过 `writing_chapter_done`，也不会把已完成章节再次设为活跃章节。该转换
/// 不创建模型 Run，符合“进入下一章后先由用户给出指导”的约束。
///
/// # Errors
///
/// 当前工作流不在本章完成状态，或下一章节 ID 与已完成章节相同时返回
/// [`WorkflowTransitionError`]。
pub fn enter_next_chapter(
    workflow: &WorkflowSnapshot,
    next_chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id == Some(next_chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "enter_next_chapter",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterDone],
        "enter_next_chapter",
        WorkflowStage::WritingChapterReady,
        ExecutionStatus::Idle,
    )?;
    next.active_run_id = None;
    next.active_chapter_id = Some(next_chapter_id);
    Ok(next)
}

/// 在正文已经确认完本后，开始一篇新的番外。
///
/// 番外和正文计划章使用同一套章节流程，但只能从稳定的完本状态进入；这确保正文
/// 的完成时间和已通过的正文章节不会被新内容覆盖。
///
/// # Errors
///
/// 稿本不在完本状态，或番外 ID 与当前活动章节相同时返回
/// [`WorkflowTransitionError`]。
pub fn enter_extra_chapter(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id == Some(chapter_id) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "enter_extra_chapter",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::Completed],
        "enter_extra_chapter",
        WorkflowStage::WritingChapterReady,
        ExecutionStatus::Idle,
    )?;
    next.active_run_id = None;
    next.active_chapter_id = Some(chapter_id);
    Ok(next)
}

/// 在最后一份计划章节已定稿后，将稿本显式标记为完本。
///
/// 存储层负责在同一事务中确认所有计划章节均已通过，且没有开放审核或活跃
/// Run；领域层保证确认操作不能从任何写作中间态越级到 `completed`。
///
/// # Errors
///
/// 工作流不处于 `writing_chapter_done` 时返回 [`WorkflowTransitionError`]。
pub fn confirm_novel_completion(
    workflow: &WorkflowSnapshot,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingChapterDone],
        "confirm_novel_completion",
        WorkflowStage::Completed,
        ExecutionStatus::Idle,
    )?;
    next.active_run_id = None;
    Ok(next)
}

fn transition_workflow(
    workflow: &WorkflowSnapshot,
    allowed_stages: &[WorkflowStage],
    action: &'static str,
    stage: WorkflowStage,
    execution: ExecutionStatus,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if !allowed_stages.contains(&workflow.stage) {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action,
        });
    }
    let (wait_reason, resume_execution) = if execution == ExecutionStatus::Stopped {
        (
            workflow.wait_reason.clone(),
            workflow.resume_execution.clone(),
        )
    } else {
        (None, None)
    };
    Ok(WorkflowSnapshot {
        version_id: workflow.version_id,
        stage,
        execution,
        state_revision: workflow.state_revision + 1,
        active_run_id: workflow.active_run_id,
        active_chapter_id: workflow.active_chapter_id,
        approval_mode: workflow.approval_mode,
        pause_requested: workflow.pause_requested,
        wait_reason,
        resume_execution,
        copied_context_folds: workflow.copied_context_folds.clone(),
    })
}

fn model_request_workflow_stages() -> [WorkflowStage; 4] {
    [
        WorkflowStage::PreparingGenerating,
        WorkflowStage::PreparingRevising,
        WorkflowStage::WritingOutlineGenerating,
        WorkflowStage::WritingDrafting,
    ]
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    AutoApprove,
    ApproveRequired,
}

/// 立即更新正文审核模式。
///
/// 模式是工作流控制数据而非阶段转换；相同模式不会制造虚假的状态修订。已有待审
/// 提案由其自身的持久记录约束，切换模式不会隐式接受或撤销它。
#[must_use]
pub fn set_approval_mode(
    workflow: &WorkflowSnapshot,
    approval_mode: ApprovalMode,
) -> WorkflowSnapshot {
    if workflow.approval_mode == approval_mode {
        return workflow.clone();
    }
    WorkflowSnapshot {
        version_id: workflow.version_id,
        stage: workflow.stage.clone(),
        execution: workflow.execution.clone(),
        state_revision: workflow.state_revision + 1,
        active_run_id: workflow.active_run_id,
        active_chapter_id: workflow.active_chapter_id,
        approval_mode,
        pause_requested: workflow.pause_requested,
        wait_reason: workflow.wait_reason.clone(),
        resume_execution: workflow.resume_execution.clone(),
        copied_context_folds: workflow.copied_context_folds.clone(),
    }
}

/// 持久化一条当前正文 Run 的自由意见，并把它线性化到工作流修订。
///
/// 自由意见不会代替待审编辑的正式决定，因此 `waiting_review` 不在允许集合中；
/// 存储层还会校验 Run、批次锚点和引用归属。本函数仅保证并发标签页不能在同一
/// `state_revision` 上无序追加意见。
///
/// # Errors
///
/// 当前 Run/章节不匹配、阶段不是正文写作，或执行轴处于正式审核等待时返回
/// [`WorkflowTransitionError`]。
pub fn record_chapter_body_feedback(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || !matches!(
            workflow.execution,
            ExecutionStatus::Queued
                | ExecutionStatus::RequestInFlight
                | ExecutionStatus::ToolBoundary
                | ExecutionStatus::WaitingAnswer
                | ExecutionStatus::Paused
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "record_chapter_body_feedback",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "record_chapter_body_feedback",
        WorkflowStage::WritingDrafting,
        workflow.execution.clone(),
    )
}

/// 请求在正文 Agent 的下一个完整工具调用边界暂停。
///
/// 请求本身不取消已经发往模型的网络调用；它只是持久化控制意图。存储层在收到下一
/// 个完整工具调用时调用 [`hold_chapter_body_tool_for_pause`]，从而不会执行半截 JSON
/// 或在工具写入之后才声称已经暂停。
///
/// # Errors
///
/// 当前 Run/章节不匹配、阶段不是正文写作，或正处于正式编辑提案审核时返回
/// [`WorkflowTransitionError`]。
pub fn request_chapter_body_pause(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || !matches!(
            workflow.execution,
            ExecutionStatus::Queued
                | ExecutionStatus::RequestInFlight
                | ExecutionStatus::ToolBoundary
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "request_chapter_body_pause",
        });
    }
    if workflow.pause_requested {
        return Ok(workflow.clone());
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "request_chapter_body_pause",
        WorkflowStage::WritingDrafting,
        workflow.execution.clone(),
    )?;
    next.pause_requested = true;
    Ok(next)
}

/// 处理自由意见时撤销尚未命中工具边界的暂停请求。
///
/// 这是暂停/意见时序中的特例：用户给出新意见意味着希望当前轮继续推进，因而不应
/// 让随后到达的工具调用仍被无意义地挂起。已进入 [`ExecutionStatus::Paused`] 的调用
/// 必须由存储层走恢复门控，不能使用本函数跳过候选正文或逐次审核。
///
/// # Errors
///
/// 当前 Run/章节不匹配、没有暂停请求，或暂停已经命中完整工具边界时返回
/// [`WorkflowTransitionError`]。
pub fn cancel_chapter_body_pause_for_feedback(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || !workflow.pause_requested
        || !matches!(
            workflow.execution,
            ExecutionStatus::Queued
                | ExecutionStatus::RequestInFlight
                | ExecutionStatus::ToolBoundary
        )
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "cancel_chapter_body_pause_for_feedback",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "cancel_chapter_body_pause_for_feedback",
        WorkflowStage::WritingDrafting,
        workflow.execution.clone(),
    )?;
    next.pause_requested = false;
    Ok(next)
}

/// 在下一条完整正文工具调用到达时进入已暂停状态。
///
/// 这条转换不应用工具编辑。工具审计、候选内容和模型请求终态由存储层和该转换一起
/// 落库，恢复时才重新按当时有效审核模式处理该调用。
///
/// # Errors
///
/// 没有先请求暂停、当前 Run/章节不匹配或不在正文请求中时返回
/// [`WorkflowTransitionError`]。
pub fn hold_chapter_body_tool_for_pause(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
        || !workflow.pause_requested
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "hold_chapter_body_tool_for_pause",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "hold_chapter_body_tool_for_pause",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Paused,
    )
}

/// 模型没有调用正文工具、而是直接提交本章时命中的暂停边界。
///
/// 它与工具边界使用相同的线性化条件，但 Store 会把结构化提交候选保存在 Run
/// 检查点中。恢复时必须重新检查正文、未投递意见与依赖，不能因为模型已说“完成”
/// 就提前打开整章审核。
///
/// # Errors
///
/// 没有先请求暂停、当前 Run/章节不匹配或不在正文请求中时返回
/// [`WorkflowTransitionError`]。
pub fn hold_chapter_body_submission_for_pause(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
        || !workflow.pause_requested
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "hold_chapter_body_submission_for_pause",
        });
    }
    transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "hold_chapter_body_submission_for_pause",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Paused,
    )
}

/// 清除已完成的暂停请求并恢复到下一次正文请求前的排队状态。
///
/// 该转换只表达暂停控制本身；存储层必须同时处理已挂起工具调用，才能使恢复后的
/// 工作流真正离开 `paused`。
///
/// # Errors
///
/// 当前 Run/章节不匹配、没有处于已暂停状态时返回
/// [`WorkflowTransitionError`]。
pub fn resume_chapter_body_after_pause(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::Paused
        || !workflow.pause_requested
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "resume_chapter_body_after_pause",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "resume_chapter_body_after_pause",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::Queued,
    )?;
    next.pause_requested = false;
    Ok(next)
}

/// 从暂停工具边界直接转入逐次确认的待审编辑状态。
///
/// 恢复时读取的是当下有效的审核模式；因此用户可以在暂停期间切换模式，但旧的挂起
/// 调用仍只会在这一条明确转换中被处理，绝不因模式切换而自动应用。
///
/// # Errors
///
/// 当前 Run/章节不匹配、没有处于已暂停状态时返回
/// [`WorkflowTransitionError`]。
pub fn resume_chapter_body_to_proposal(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::Paused
        || !workflow.pause_requested
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "resume_chapter_body_to_proposal",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "resume_chapter_body_to_proposal",
        WorkflowStage::WritingDrafting,
        ExecutionStatus::WaitingReview,
    )?;
    next.pause_requested = false;
    Ok(next)
}

/// 恢复一个已挂起的结构化章节提交并进入整章审核。
///
/// Store 在调用前重新检查正文修订、依赖与意见批次；本函数只表达通过这些守卫后的
/// 最终工作流转换。
///
/// # Errors
///
/// 当前 Run/章节不匹配、没有处于已暂停状态时返回
/// [`WorkflowTransitionError`]。
pub fn resume_chapter_body_submission_after_pause(
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
    run_id: RunId,
) -> Result<WorkflowSnapshot, WorkflowTransitionError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::Paused
        || !workflow.pause_requested
    {
        return Err(WorkflowTransitionError::InvalidTransition {
            stage: workflow.stage.clone(),
            action: "resume_chapter_body_submission_after_pause",
        });
    }
    let mut next = transition_workflow(
        workflow,
        &[WorkflowStage::WritingDrafting],
        "resume_chapter_body_submission_after_pause",
        WorkflowStage::WritingChapterReviewing,
        ExecutionStatus::WaitingReview,
    )?;
    next.pause_requested = false;
    Ok(next)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBundle {
    pub project: Project,
    pub version: NovelVersion,
    pub workflow: WorkflowSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListItem {
    pub project: Project,
    pub selected_version: NovelVersion,
    pub workflow: WorkflowSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSnapshot {
    pub project: Project,
    pub version: NovelVersion,
    pub workflow: WorkflowSnapshot,
    pub event_watermark: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProject {
    pub name: String,
    pub novel_title: String,
    pub version_label: String,
}

impl NewProject {
    /// 创建并校验项目及其首个稿本的输入。
    ///
    /// # Errors
    ///
    /// 当任一必填文本为空或超过长度上限时返回 [`ValidationError`]。
    pub fn new(
        name: impl AsRef<str>,
        novel_title: impl AsRef<str>,
        version_label: Option<impl AsRef<str>>,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            name: validate_required_text("项目名称", name.as_ref(), MAX_PROJECT_NAME_LEN)?,
            novel_title: validate_required_text(
                "小说标题",
                novel_title.as_ref(),
                MAX_NOVEL_TITLE_LEN,
            )?,
            version_label: match version_label {
                Some(value) => {
                    validate_required_text("稿本标签", value.as_ref(), MAX_VERSION_LABEL_LEN)?
                }
                None => "初稿".to_owned(),
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewNovelVersion {
    pub label: String,
    pub novel_title: String,
}

impl NewNovelVersion {
    /// 创建并校验新稿本输入。
    ///
    /// # Errors
    ///
    /// 当标签或小说标题为空，或超过长度上限时返回 [`ValidationError`]。
    pub fn new(
        label: impl AsRef<str>,
        novel_title: impl AsRef<str>,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            label: validate_required_text("稿本标签", label.as_ref(), MAX_VERSION_LABEL_LEN)?,
            novel_title: validate_required_text(
                "小说标题",
                novel_title.as_ref(),
                MAX_NOVEL_TITLE_LEN,
            )?,
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ValidationError {
    #[error("{field}不能为空")]
    Required { field: &'static str },
    #[error("{field}不能超过 {max} 个字符")]
    TooLong { field: &'static str, max: usize },
}

/// 校验必填短文本并去除首尾空白。
///
/// # Errors
///
/// 当文本为空或超过 `max_len` 个 Unicode 字符时返回 [`ValidationError`]。
pub fn validate_required_text(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<String, ValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::Required { field });
    }
    if trimmed.chars().count() > max_len {
        return Err(ValidationError::TooLong {
            field,
            max: max_len,
        });
    }
    Ok(trimmed.to_owned())
}

/// 校验可审核文档，但不改变其原始内容。文档中的首尾换行、段落与 Markdown
/// 格式均是产物的一部分，后续阶段折叠必须能逐字使用它们。
///
/// # Errors
///
/// 当内容仅含空白或超过 `max_len` 个 Unicode 字符时返回 [`ValidationError`]。
pub fn validate_document_content(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<String, ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::Required { field });
    }
    if value.chars().count() > max_len {
        return Err(ValidationError::TooLong {
            field,
            max: max_len,
        });
    }
    Ok(value.to_owned())
}

#[must_use]
pub fn non_whitespace_char_count(text: &str) -> u64 {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .count() as u64
}

#[must_use]
pub fn utc_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_count_excludes_all_unicode_whitespace() {
        assert_eq!(non_whitespace_char_count("春 风\n\t雨　夜"), 4);
    }

    #[test]
    fn project_input_is_trimmed_and_validated() {
        let project = NewProject::new("  雨夜  ", "  来客  ", None::<&str>).unwrap();
        assert_eq!(project.name, "雨夜");
        assert_eq!(project.novel_title, "来客");
        assert_eq!(project.version_label, "初稿");
        assert_eq!(
            NewProject::new("", "标题", None::<&str>).unwrap_err(),
            ValidationError::Required {
                field: "项目名称"
            }
        );
    }

    #[test]
    fn preparation_fold_keeps_only_complete_artifact_text_in_stable_order() {
        let ids = [
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        ];
        let chapter_plan = ChapterPlan::from_drafts(&[ChapterPlanDraft::new(
            "雨夜",
            "林舟在旧码头收到匿名线索。",
        )
        .unwrap()])
        .unwrap();
        let artifacts = vec![
            ApprovedPreparationArtifact {
                kind: PreparationArtifactKind::Outline,
                revision_id: ids[3],
                content: "# 粗纲\n\n第一章：雨夜".to_owned(),
                chapter_plan: Some(chapter_plan),
            },
            ApprovedPreparationArtifact {
                kind: PreparationArtifactKind::Characters,
                revision_id: ids[2],
                content: "林舟：沉默的记者".to_owned(),
                chapter_plan: None,
            },
            ApprovedPreparationArtifact {
                kind: PreparationArtifactKind::Background,
                revision_id: ids[0],
                content: "海港城连续落雨七日。".to_owned(),
                chapter_plan: None,
            },
            ApprovedPreparationArtifact {
                kind: PreparationArtifactKind::Research,
                revision_id: ids[1],
                content: "资料来源：用户提供的旧报纸。".to_owned(),
                chapter_plan: None,
            },
        ];

        assert_eq!(
            render_preparation_artifact_plaintext(&artifacts).unwrap(),
            "海港城连续落雨七日。\n\n资料来源：用户提供的旧报纸。\n\n林舟：沉默的记者\n\n# 粗纲\n\n第一章：雨夜\n\n第1章：雨夜\n林舟在旧码头收到匿名线索。"
        );
    }

    #[test]
    fn preparation_fold_rejects_partial_or_duplicate_artifacts() {
        let background = ApprovedPreparationArtifact {
            kind: PreparationArtifactKind::Background,
            revision_id: Uuid::new_v4(),
            content: "背景".to_owned(),
            chapter_plan: None,
        };
        assert_eq!(
            render_preparation_artifact_plaintext(std::slice::from_ref(&background)).unwrap_err(),
            ArtifactFoldError::MissingArtifact { kind: "research" }
        );
        let duplicate = vec![
            background.clone(),
            ApprovedPreparationArtifact {
                revision_id: Uuid::new_v4(),
                ..background
            },
        ];
        assert_eq!(
            render_preparation_artifact_plaintext(&duplicate).unwrap_err(),
            ArtifactFoldError::DuplicateArtifact { kind: "background" }
        );
    }

    #[test]
    fn preparation_review_only_advances_after_a_terminal_batch_result() {
        let version_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let started = begin_preparation(&WorkflowSnapshot::initial(version_id), run_id).unwrap();
        assert_eq!(started.stage, WorkflowStage::PreparingGenerating);
        assert_eq!(started.execution, ExecutionStatus::Queued);
        assert_eq!(started.active_run_id, Some(run_id));
        let reviewing = submit_preparation_for_review(&started).unwrap();
        assert_eq!(reviewing.execution, ExecutionStatus::WaitingReview);
        let revising =
            finish_preparation_review(&reviewing, PreparationReviewCompletion::RevisionsRequired)
                .unwrap();
        assert_eq!(revising.stage, WorkflowStage::PreparingRevising);
        assert_eq!(revising.execution, ExecutionStatus::Queued);
        assert!(matches!(
            finish_preparation_review(&revising, PreparationReviewCompletion::AllApproved),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn model_request_execution_is_linearized_with_the_active_run() {
        let version_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let queued = begin_preparation(&WorkflowSnapshot::initial(version_id), run_id).unwrap();

        let in_flight = dispatch_model_request(&queued, run_id).unwrap();
        assert_eq!(in_flight.execution, ExecutionStatus::RequestInFlight);
        assert_eq!(in_flight.state_revision, queued.state_revision + 1);

        assert!(dispatch_model_request(&in_flight, run_id).is_err());
        assert!(dispatch_model_request(&queued, Uuid::new_v4()).is_err());

        let completed =
            complete_model_request(&in_flight, run_id, ModelRequestCompletion::Succeeded).unwrap();
        assert_eq!(completed.execution, ExecutionStatus::ToolBoundary);
        assert_eq!(completed.state_revision, in_flight.state_revision + 1);

        let failed =
            complete_model_request(&in_flight, run_id, ModelRequestCompletion::Unknown).unwrap();
        assert_eq!(failed.execution, ExecutionStatus::Failed);

        let pre_dispatch_failed = fail_queued_model_run(&queued, run_id).unwrap();
        assert_eq!(pre_dispatch_failed.execution, ExecutionStatus::Failed);
        assert!(fail_queued_model_run(&in_flight, run_id).is_err());

        let interrupted = interrupt_writing_run(&in_flight, run_id).unwrap();
        assert_eq!(interrupted.execution, ExecutionStatus::Interrupted);
        let resumed = continue_writing_run(&interrupted, run_id).unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        assert!(continue_writing_run(&resumed, run_id).is_err());
    }

    #[test]
    fn explicit_stop_preserves_a_stable_answer_boundary_but_requeues_in_flight_work() {
        let version_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let queued = begin_preparation(&WorkflowSnapshot::initial(version_id), run_id).unwrap();
        let in_flight = dispatch_model_request(&queued, run_id).unwrap();

        let stopped_in_flight = stop_writing_run(&in_flight, run_id).unwrap();
        assert_eq!(stopped_in_flight.execution, ExecutionStatus::Stopped);
        assert_eq!(
            stopped_in_flight.wait_reason,
            Some(RunWaitReason::UserStopped)
        );
        assert_eq!(
            stopped_in_flight.resume_execution,
            Some(ExecutionStatus::RequestInFlight)
        );
        assert_eq!(
            continue_writing_run(&stopped_in_flight, run_id)
                .unwrap()
                .execution,
            ExecutionStatus::Queued
        );

        let waiting_answer = wait_for_user_answer(&in_flight, run_id).unwrap();
        let stopped_waiting = stop_writing_run(&waiting_answer, run_id).unwrap();
        assert_eq!(
            stopped_waiting.resume_execution,
            Some(ExecutionStatus::WaitingAnswer)
        );
        let resumed_waiting = continue_writing_run(&stopped_waiting, run_id).unwrap();
        assert_eq!(resumed_waiting.execution, ExecutionStatus::WaitingAnswer);
        assert_eq!(resumed_waiting.wait_reason, None);
        assert_eq!(resumed_waiting.resume_execution, None);
    }

    #[test]
    fn chapter_guidance_and_outline_start_require_the_active_chapter() {
        let version_id = Uuid::new_v4();
        let chapter_id = Uuid::new_v4();
        let other_chapter_id = Uuid::new_v4();
        let mut ready = WorkflowSnapshot::initial(version_id);
        ready.stage = WorkflowStage::WritingChapterReady;
        ready.active_chapter_id = Some(chapter_id);

        assert!(matches!(
            record_chapter_guidance(&ready, other_chapter_id),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
        let guided = record_chapter_guidance(&ready, chapter_id).unwrap();
        assert_eq!(guided.state_revision, 1);
        assert_eq!(guided.stage, WorkflowStage::WritingChapterReady);

        let run_id = Uuid::new_v4();
        let started = begin_chapter_outline(&guided, chapter_id, run_id).unwrap();
        assert_eq!(started.stage, WorkflowStage::WritingOutlineGenerating);
        assert_eq!(started.execution, ExecutionStatus::Queued);
        assert_eq!(started.active_run_id, Some(run_id));

        let reviewing = submit_chapter_outline_for_review(&started, chapter_id).unwrap();
        assert_eq!(reviewing.stage, WorkflowStage::WritingOutlineReviewing);
        assert_eq!(reviewing.execution, ExecutionStatus::WaitingReview);
        let revising = finish_chapter_outline_review(
            &reviewing,
            chapter_id,
            ChapterOutlineReviewCompletion::RevisionsRequired,
        )
        .unwrap();
        assert_eq!(revising.stage, WorkflowStage::WritingOutlineGenerating);
        assert_eq!(revising.execution, ExecutionStatus::Queued);
        assert_eq!(revising.active_run_id, Some(run_id));

        let reviewing = submit_chapter_outline_for_review(&revising, chapter_id).unwrap();
        let drafting = finish_chapter_outline_review(
            &reviewing,
            chapter_id,
            ChapterOutlineReviewCompletion::Approved,
        )
        .unwrap();
        assert_eq!(drafting.stage, WorkflowStage::WritingDrafting);
        assert_eq!(drafting.execution, ExecutionStatus::Idle);
        assert_eq!(drafting.active_run_id, None);

        let body_run_id = Uuid::new_v4();
        let drafting = schedule_chapter_body_run(&drafting, chapter_id, body_run_id).unwrap();
        assert_eq!(drafting.execution, ExecutionStatus::Queued);
        assert_eq!(drafting.active_run_id, Some(body_run_id));
        let body_request_in_flight = dispatch_model_request(&drafting, body_run_id).unwrap();
        let queued_after_tool =
            continue_chapter_body_after_tool(&body_request_in_flight, chapter_id, body_run_id)
                .unwrap();
        assert_eq!(queued_after_tool.execution, ExecutionStatus::Queued);
        assert_eq!(
            queued_after_tool.state_revision,
            body_request_in_flight.state_revision + 1
        );
        assert!(
            continue_chapter_body_after_tool(&queued_after_tool, chapter_id, body_run_id).is_err()
        );
        // 模型回合仍在进行（排队中或在途）时，用户编辑不把执行轴改成空闲，
        // 否则那个回合的回调路径会被废掉，工作流停在没有任何继续入口的状态。
        let edited_in_progress = record_chapter_body_edit(&drafting, chapter_id).unwrap();
        assert_eq!(edited_in_progress.execution, ExecutionStatus::Queued);
        assert_eq!(edited_in_progress.active_run_id, Some(body_run_id));
        let mut edited = edited_in_progress.clone();
        // 没有正在进行回合时，编辑照旧回到空闲。
        edited.execution = ExecutionStatus::Idle;
        edited.active_run_id = None;
        let edited = record_chapter_body_edit(&edited, chapter_id).unwrap();
        assert_eq!(edited.execution, ExecutionStatus::Idle);
        let reviewing = submit_chapter_for_review(&edited, chapter_id).unwrap();
        assert_eq!(reviewing.stage, WorkflowStage::WritingChapterReviewing);
        let revising = finish_chapter_final_review(
            &reviewing,
            chapter_id,
            ChapterFinalReviewCompletion::RevisionsRequired,
        )
        .unwrap();
        assert_eq!(revising.stage, WorkflowStage::WritingDrafting);
        let reviewing = submit_chapter_for_review(&revising, chapter_id).unwrap();
        let done = finish_chapter_final_review(
            &reviewing,
            chapter_id,
            ChapterFinalReviewCompletion::Approved,
        )
        .unwrap();
        assert_eq!(done.stage, WorkflowStage::WritingChapterDone);
        assert_eq!(done.active_run_id, None);
    }

    #[test]
    fn body_replacement_requires_one_exact_match_including_overlaps() {
        assert_eq!(
            replace_unique_chapter_content("港口的雨夜", "雨夜", "雾夜").unwrap(),
            "港口的雾夜"
        );
        assert_eq!(
            replace_unique_chapter_content("😀雨夜", "😀", "🌙").unwrap(),
            "🌙雨夜"
        );
        assert_eq!(
            replace_unique_chapter_content("雨夜", "", "雾夜").unwrap_err(),
            ChapterBodyEditError::EmptyOldText
        );
        assert_eq!(
            replace_unique_chapter_content("aaa", "aa", "b").unwrap_err(),
            ChapterBodyEditError::MultipleMatches
        );
        assert_eq!(
            replace_unique_chapter_content("雨夜", "晴天", "雾夜").unwrap_err(),
            ChapterBodyEditError::NoMatch
        );
        assert_eq!(
            replace_unique_chapter_content("雨夜", "雨夜", "雨夜").unwrap_err(),
            ChapterBodyEditError::NoChange
        );
    }

    #[test]
    fn chapter_body_proposal_gate_preserves_the_run_and_mode_switches_do_not_decide_it() {
        let version_id = Uuid::new_v4();
        let chapter_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let mut drafting = WorkflowSnapshot::initial(version_id);
        drafting.stage = WorkflowStage::WritingDrafting;
        drafting.execution = ExecutionStatus::Queued;
        drafting.active_chapter_id = Some(chapter_id);
        drafting.active_run_id = Some(run_id);

        let in_flight = dispatch_model_request(&drafting, run_id).unwrap();
        let waiting = hold_chapter_body_proposal(&in_flight, chapter_id, run_id).unwrap();
        assert_eq!(waiting.stage, WorkflowStage::WritingDrafting);
        assert_eq!(waiting.execution, ExecutionStatus::WaitingReview);
        assert_eq!(waiting.active_run_id, Some(run_id));

        let switched = set_approval_mode(&waiting, ApprovalMode::ApproveRequired);
        assert_eq!(switched.execution, ExecutionStatus::WaitingReview);
        assert_eq!(switched.active_run_id, Some(run_id));
        assert_eq!(switched.approval_mode, ApprovalMode::ApproveRequired);
        assert_eq!(
            set_approval_mode(&switched, ApprovalMode::ApproveRequired),
            switched
        );

        let queued = continue_chapter_body_after_proposal(&switched, chapter_id, run_id).unwrap();
        assert_eq!(queued.execution, ExecutionStatus::Queued);
        assert_eq!(queued.active_run_id, Some(run_id));
        assert!(continue_chapter_body_after_proposal(&queued, chapter_id, run_id).is_err());
    }

    #[test]
    fn chapter_body_pause_waits_for_a_tool_boundary_before_becoming_paused() {
        let version_id = Uuid::new_v4();
        let chapter_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let mut queued = WorkflowSnapshot::initial(version_id);
        queued.stage = WorkflowStage::WritingDrafting;
        queued.execution = ExecutionStatus::Queued;
        queued.active_chapter_id = Some(chapter_id);
        queued.active_run_id = Some(run_id);

        let requested = request_chapter_body_pause(&queued, chapter_id, run_id).unwrap();
        assert_eq!(requested.execution, ExecutionStatus::Queued);
        assert!(requested.pause_requested);
        assert_eq!(
            request_chapter_body_pause(&requested, chapter_id, run_id).unwrap(),
            requested
        );

        let in_flight = dispatch_model_request(&requested, run_id).unwrap();
        let paused = hold_chapter_body_tool_for_pause(&in_flight, chapter_id, run_id).unwrap();
        assert_eq!(paused.execution, ExecutionStatus::Paused);
        assert!(paused.pause_requested);

        let resumed = resume_chapter_body_after_pause(&paused, chapter_id, run_id).unwrap();
        assert_eq!(resumed.execution, ExecutionStatus::Queued);
        assert!(!resumed.pause_requested);
        assert!(hold_chapter_body_tool_for_pause(&requested, chapter_id, run_id).is_err());
    }

    #[test]
    fn body_inputs_normalize_line_endings_before_persistence_or_matching() {
        assert_eq!(normalize_line_endings("甲\r\n乙\r丙"), "甲\n乙\n丙");
        assert_eq!(
            AppendChapterContent::new("甲\r\n乙").unwrap().content,
            "甲\n乙"
        );
        let replacement = ReplaceChapterContent::new("甲\r\n乙", "甲\r丙").unwrap();
        assert_eq!(replacement.old_str, "甲\n乙");
        assert_eq!(replacement.new_str, "甲\n丙");
        assert_eq!(
            replace_unique_chapter_content(
                "前言\n甲\n乙",
                &replacement.old_str,
                &replacement.new_str
            )
            .unwrap(),
            "前言\n甲\n丙"
        );
    }

    #[test]
    fn wire_i64_fields_are_decimal_strings_and_reject_json_numbers() {
        let version_id = Uuid::new_v4();
        let mut workflow = WorkflowSnapshot::initial(version_id);
        workflow.state_revision = i64::MAX;
        let mut encoded = serde_json::to_value(&workflow).unwrap();
        assert_eq!(encoded["stateRevision"], i64::MAX.to_string());
        encoded["stateRevision"] = serde_json::json!(1);
        assert!(serde_json::from_value::<WorkflowSnapshot>(encoded).is_err());
    }

    #[test]
    fn feedback_citations_round_trip_the_camel_case_wire_contract() {
        let document: FeedbackCitation = serde_json::from_value(serde_json::json!({
            "kind": "document",
            "documentId": "00000000-0000-0000-0000-000000000001",
            "revisionId": "00000000-0000-0000-0000-000000000002",
            "chapterId": null,
            "startChar": "0",
            "endChar": "2",
            "quote": "雨夜"
        }))
        .unwrap();
        assert_eq!(
            document,
            FeedbackCitation::Document {
                document_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                revision_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                range: TextCitationRange {
                    start_char: 0,
                    end_char: 2,
                    quote: "雨夜".to_owned(),
                },
                chapter_id: None,
            }
        );

        let candidate: FeedbackCitation = serde_json::from_value(serde_json::json!({
            "kind": "candidate",
            "toolCallId": "00000000-0000-0000-0000-000000000003",
            "candidateHash": "a".repeat(64),
            "candidateRange": { "startChar": "4", "endChar": "6", "quote": "胶卷" }
        }))
        .unwrap();
        assert_eq!(
            candidate,
            FeedbackCitation::Candidate {
                tool_call_id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
                candidate_hash: "a".repeat(64),
                candidate_range: TextCitationRange {
                    start_char: 4,
                    end_char: 6,
                    quote: "胶卷".to_owned(),
                },
            }
        );

        assert_eq!(
            serde_json::to_value(&document).unwrap(),
            serde_json::json!({
                "kind": "document",
                "documentId": "00000000-0000-0000-0000-000000000001",
                "revisionId": "00000000-0000-0000-0000-000000000002",
                "chapterId": null,
                "startChar": "0",
                "endChar": "2",
                "quote": "雨夜"
            })
        );
    }

    #[test]
    fn next_chapter_can_only_start_after_the_current_chapter_is_done() {
        let version_id = Uuid::new_v4();
        let completed_chapter_id = Uuid::new_v4();
        let next_chapter_id = Uuid::new_v4();
        let mut done = WorkflowSnapshot::initial(version_id);
        done.stage = WorkflowStage::WritingChapterDone;
        done.active_chapter_id = Some(completed_chapter_id);

        let ready = enter_next_chapter(&done, next_chapter_id).unwrap();
        assert_eq!(ready.stage, WorkflowStage::WritingChapterReady);
        assert_eq!(ready.execution, ExecutionStatus::Idle);
        assert_eq!(ready.active_chapter_id, Some(next_chapter_id));
        assert_eq!(ready.active_run_id, None);

        assert!(matches!(
            enter_next_chapter(&done, completed_chapter_id),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
        assert!(matches!(
            enter_next_chapter(&ready, Uuid::new_v4()),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));

        let completed = confirm_novel_completion(&done).unwrap();
        assert_eq!(completed.stage, WorkflowStage::Completed);
        assert_eq!(completed.active_chapter_id, Some(completed_chapter_id));
        assert_eq!(completed.active_run_id, None);
        assert!(matches!(
            confirm_novel_completion(&ready),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn extra_chapter_returns_to_completed_without_reopening_main_completion() {
        let version_id = Uuid::new_v4();
        let main_chapter_id = Uuid::new_v4();
        let extra_chapter_id = Uuid::new_v4();
        let mut main_done = WorkflowSnapshot::initial(version_id);
        main_done.stage = WorkflowStage::WritingChapterDone;
        main_done.active_chapter_id = Some(main_chapter_id);
        let completed = confirm_novel_completion(&main_done).unwrap();

        let extra_ready = enter_extra_chapter(&completed, extra_chapter_id).unwrap();
        assert_eq!(extra_ready.stage, WorkflowStage::WritingChapterReady);
        assert_eq!(extra_ready.execution, ExecutionStatus::Idle);
        assert_eq!(extra_ready.active_chapter_id, Some(extra_chapter_id));
        assert_eq!(extra_ready.active_run_id, None);

        let mut extra_reviewing = extra_ready.clone();
        extra_reviewing.stage = WorkflowStage::WritingChapterReviewing;
        extra_reviewing.execution = ExecutionStatus::WaitingReview;
        extra_reviewing.active_run_id = Some(Uuid::new_v4());
        let completed_again =
            finish_extra_chapter_final_review(&extra_reviewing, extra_chapter_id).unwrap();
        assert_eq!(completed_again.stage, WorkflowStage::Completed);
        assert_eq!(completed_again.execution, ExecutionStatus::Idle);
        assert_eq!(completed_again.active_chapter_id, Some(extra_chapter_id));
        assert_eq!(completed_again.active_run_id, None);

        assert!(matches!(
            enter_extra_chapter(&main_done, extra_chapter_id),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
        assert!(matches!(
            finish_extra_chapter_final_review(&completed, extra_chapter_id),
            Err(WorkflowTransitionError::InvalidTransition { .. })
        ));
    }
}

/// 创建一份复制已通过内容的稿本时用户提供的唯一文本输入。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewCopiedNovelVersion {
    pub label: String,
}

impl NewCopiedNovelVersion {
    /// # Errors
    ///
    /// 当稿本标签为空或超过长度上限时返回 [`ValidationError`]。
    pub fn new(label: impl AsRef<str>) -> Result<Self, ValidationError> {
        Ok(Self {
            label: validate_required_text("稿本标签", label.as_ref(), MAX_VERSION_LABEL_LEN)?,
        })
    }
}
