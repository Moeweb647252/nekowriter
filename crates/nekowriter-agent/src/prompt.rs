//! 内建提示词变量目录与受限模板渲染器。
//!
//! 模板语言刻意只支持一次性的 `{{variable_name}}` 文本替换。它不执行表达式、
//! 循环、包含或递归替换，因此用户内容即使包含 `{{...}}` 也不会被再次解释。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{BuiltinPrompt, PromptKey, builtin_prompt};

/// 一项内建提示词变量的公开说明。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptVariableDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub example: &'static str,
}

/// 一个已经渲染并可封存到模型请求快照的内建提示词对。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedBuiltinPrompt {
    pub key: PromptKey,
    pub system: String,
    pub user: String,
    pub system_content_hash: String,
    pub user_content_hash: String,
}

/// 受限提示词模板的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PromptTemplateError {
    #[error("模板中存在不完整的占位符")]
    UnclosedPlaceholder,
    #[error("模板中存在没有起始标记的 }}")]
    UnexpectedClosingDelimiter,
    #[error("模板变量名 {name:?} 无效")]
    InvalidVariableName { name: String },
    #[error("模板变量 {name:?} 不属于该提示词")]
    UnknownVariable { name: String },
    #[error("必需模板变量 {name:?} 没有出现在提示词对中")]
    MissingRequiredPlaceholder { name: String },
    #[error("渲染时缺少必需变量 {name:?}")]
    MissingRuntimeVariable { name: String },
}

const NOVEL_BRIEF: PromptVariableDefinition = PromptVariableDefinition {
    name: "novel_brief",
    description: "当前稿本的小说目标。",
    required: true,
    example: "一部关于海港城旧报纸的悬疑小说",
};
const CHAPTER_PLAN: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_plan",
    description: "当前章节的已锁定计划项（标题和目标）。",
    required: true,
    example: "第 1 章：雨夜的来客\n主角在码头收到匿名报纸。",
};
const CHAPTER_GUIDANCE: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_guidance",
    description: "用户为当前章节提交的指导或本轮意见。空字符串表示用户未额外填写。",
    required: true,
    example: "突出雨夜和主角的犹豫。",
};
const CHAPTER_OUTLINE: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_outline",
    description: "当前章节已经审核通过的细纲全文。",
    required: true,
    example: "林舟抵达码头，发现旧报纸上的名字。",
};
const CHAPTER_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_content",
    description: "当前章节已接受的正文全文。空字符串表示尚无正文。",
    required: true,
    example: "雨落在旧码头的铁皮屋顶上。",
};
const TARGET_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "target_content",
    description: "本次待审核的目标产物或正文。",
    required: true,
    example: "待审核的产物全文。",
};
const APPROVED_CONTEXT: PromptVariableDefinition = PromptVariableDefinition {
    name: "approved_context",
    description: "与当前审核目标相关、已经审核通过的上游产物全文。",
    required: true,
    example: "已通过的背景与资料。",
};
const CHAPTER_CONTEXT: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_context",
    description: "当前章节的锁定计划项和用户指导。",
    required: true,
    example: "第 1 章目标：在码头收到匿名报纸。",
};
const BASE_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "base_content",
    description: "正文编辑提案所基于的正式正文。",
    required: true,
    example: "雨落在旧码头的铁皮屋顶上。",
};

const PREPARATION_VARIABLES: &[PromptVariableDefinition] = &[NOVEL_BRIEF];
const CHAPTER_OUTLINE_VARIABLES: &[PromptVariableDefinition] = &[CHAPTER_PLAN, CHAPTER_GUIDANCE];
const CHAPTER_WRITE_VARIABLES: &[PromptVariableDefinition] = &[CHAPTER_CONTENT, CHAPTER_GUIDANCE];
const PREPARATION_REVIEW_BASIC_VARIABLES: &[PromptVariableDefinition] =
    &[NOVEL_BRIEF, TARGET_CONTENT];
const PREPARATION_REVIEW_CONTEXT_VARIABLES: &[PromptVariableDefinition] =
    &[APPROVED_CONTEXT, TARGET_CONTENT];
const CHAPTER_OUTLINE_REVIEW_VARIABLES: &[PromptVariableDefinition] =
    &[CHAPTER_CONTEXT, TARGET_CONTENT];
const CHAPTER_EDIT_REVIEW_VARIABLES: &[PromptVariableDefinition] = &[BASE_CONTENT, TARGET_CONTENT];
const CHAPTER_FINAL_REVIEW_VARIABLES: &[PromptVariableDefinition] =
    &[CHAPTER_OUTLINE, TARGET_CONTENT];

/// 返回一个内建提示词键允许的变量目录。
#[must_use]
pub const fn builtin_prompt_variables(key: PromptKey) -> &'static [PromptVariableDefinition] {
    match key {
        PromptKey::PreparationGenerate => PREPARATION_VARIABLES,
        PromptKey::ChapterOutline => CHAPTER_OUTLINE_VARIABLES,
        PromptKey::ChapterWrite => CHAPTER_WRITE_VARIABLES,
        PromptKey::ReviewPreparationBackground | PromptKey::ReviewPreparationResearch => {
            PREPARATION_REVIEW_BASIC_VARIABLES
        }
        PromptKey::ReviewPreparationCharacters | PromptKey::ReviewPreparationOutline => {
            PREPARATION_REVIEW_CONTEXT_VARIABLES
        }
        PromptKey::ReviewChapterOutline => CHAPTER_OUTLINE_REVIEW_VARIABLES,
        PromptKey::ReviewChapterEdit => CHAPTER_EDIT_REVIEW_VARIABLES,
        PromptKey::ReviewChapterFinal => CHAPTER_FINAL_REVIEW_VARIABLES,
    }
}

/// 校验一对内建提示词覆盖内容。
///
/// # Errors
///
/// 未闭合/未知变量、非法变量名，或少了该提示词要求的占位符时返回错误。
pub fn validate_builtin_prompt_pair(
    key: PromptKey,
    system: &str,
    user: &str,
) -> Result<(), PromptTemplateError> {
    let definitions = builtin_prompt_variables(key);
    let mut placeholders = scan_placeholders(system, definitions)?;
    placeholders.extend(scan_placeholders(user, definitions)?);
    for definition in definitions {
        if definition.required && !placeholders.contains(definition.name) {
            return Err(PromptTemplateError::MissingRequiredPlaceholder {
                name: definition.name.to_owned(),
            });
        }
    }
    Ok(())
}

/// 使用提供的运行时变量渲染一对内建提示词，并生成稳定内容 hash。
///
/// # Errors
///
/// 覆盖内容不符合受限模板语言、缺少必需占位符或运行时变量时返回错误；变量值
/// 永不递归展开。
pub fn render_builtin_prompt(
    key: PromptKey,
    variables: &BTreeMap<String, String>,
) -> Result<RenderedBuiltinPrompt, PromptTemplateError> {
    let prompt = builtin_prompt(key);
    render_builtin_prompt_content(prompt, variables)
}

/// 渲染指定的内建提示词内容。该入口用于先按覆盖优先级解析出实际内容后再封存。
///
/// # Errors
///
/// 同 [`render_builtin_prompt`]。
pub fn render_builtin_prompt_content(
    prompt: BuiltinPrompt,
    variables: &BTreeMap<String, String>,
) -> Result<RenderedBuiltinPrompt, PromptTemplateError> {
    render_prompt_content(prompt.key, prompt.system, prompt.user, variables)
}

/// 渲染一个已经按覆盖优先级解析的提示词内容对。
///
/// # Errors
///
/// 同 [`render_builtin_prompt`]。
pub fn render_prompt_content(
    key: PromptKey,
    system: &str,
    user: &str,
    variables: &BTreeMap<String, String>,
) -> Result<RenderedBuiltinPrompt, PromptTemplateError> {
    validate_builtin_prompt_pair(key, system, user)?;
    for definition in builtin_prompt_variables(key) {
        if definition.required && !variables.contains_key(definition.name) {
            return Err(PromptTemplateError::MissingRuntimeVariable {
                name: definition.name.to_owned(),
            });
        }
    }
    let system = render_template(system, variables, builtin_prompt_variables(key))?;
    let user = render_template(user, variables, builtin_prompt_variables(key))?;
    Ok(RenderedBuiltinPrompt {
        key,
        system_content_hash: content_hash(&system),
        user_content_hash: content_hash(&user),
        system,
        user,
    })
}

/// 扫描并渲染一个受限模板。
///
/// `\\{{name}}` 产生字面量 `{{name}}`，不会被作为占位符解析。变量值只会被附加
/// 一次，不会再扫描其中的花括号。
fn render_template(
    source: &str,
    variables: &BTreeMap<String, String>,
    definitions: &[PromptVariableDefinition],
) -> Result<String, PromptTemplateError> {
    let mut rendered = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if remaining.starts_with("\\{{") {
            let name_start = cursor + 3;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(PromptTemplateError::UnclosedPlaceholder);
            };
            let end = name_start + relative_end + 2;
            let name = &source[name_start..name_start + relative_end];
            validate_variable_name(name)?;
            rendered.push_str("{{");
            rendered.push_str(name);
            rendered.push_str("}}");
            cursor = end;
            continue;
        }
        if remaining.starts_with("{{") {
            let name_start = cursor + 2;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(PromptTemplateError::UnclosedPlaceholder);
            };
            let name = &source[name_start..name_start + relative_end];
            validate_variable_name(name)?;
            if !definitions.iter().any(|definition| definition.name == name) {
                return Err(PromptTemplateError::UnknownVariable {
                    name: name.to_owned(),
                });
            }
            let value =
                variables
                    .get(name)
                    .ok_or_else(|| PromptTemplateError::MissingRuntimeVariable {
                        name: name.to_owned(),
                    })?;
            rendered.push_str(value);
            cursor = name_start + relative_end + 2;
            continue;
        }
        if remaining.starts_with("}}") {
            return Err(PromptTemplateError::UnexpectedClosingDelimiter);
        }
        let character = remaining
            .chars()
            .next()
            .expect("cursor is always a UTF-8 boundary before source end");
        rendered.push(character);
        cursor += character.len_utf8();
    }
    Ok(rendered)
}

fn scan_placeholders(
    source: &str,
    definitions: &[PromptVariableDefinition],
) -> Result<BTreeSet<String>, PromptTemplateError> {
    let mut found = BTreeSet::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if remaining.starts_with("\\{{") {
            let name_start = cursor + 3;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(PromptTemplateError::UnclosedPlaceholder);
            };
            validate_variable_name(&source[name_start..name_start + relative_end])?;
            cursor = name_start + relative_end + 2;
            continue;
        }
        if remaining.starts_with("{{") {
            let name_start = cursor + 2;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(PromptTemplateError::UnclosedPlaceholder);
            };
            let name = &source[name_start..name_start + relative_end];
            validate_variable_name(name)?;
            if !definitions.iter().any(|definition| definition.name == name) {
                return Err(PromptTemplateError::UnknownVariable {
                    name: name.to_owned(),
                });
            }
            found.insert(name.to_owned());
            cursor = name_start + relative_end + 2;
            continue;
        }
        if remaining.starts_with("}}") {
            return Err(PromptTemplateError::UnexpectedClosingDelimiter);
        }
        cursor += remaining
            .chars()
            .next()
            .expect("cursor is always a UTF-8 boundary before source end")
            .len_utf8();
    }
    Ok(found)
}

fn validate_variable_name(name: &str) -> Result<(), PromptTemplateError> {
    let mut characters = name.bytes();
    let Some(first) = characters.next() else {
        return Err(PromptTemplateError::InvalidVariableName {
            name: name.to_owned(),
        });
    };
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == b'_')
    {
        return Err(PromptTemplateError::InvalidVariableName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

fn content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_have_declared_variables_and_render_to_stable_snapshots() {
        for key in PromptKey::ALL {
            let values = builtin_prompt_variables(key)
                .iter()
                .map(|definition| (definition.name.to_owned(), definition.example.to_owned()))
                .collect::<BTreeMap<_, _>>();
            let rendered = render_builtin_prompt(key, &values).unwrap();
            assert_eq!(rendered.key, key);
            assert!(!rendered.system_content_hash.is_empty());
            assert!(!rendered.user_content_hash.is_empty());
            assert_eq!(
                rendered,
                render_builtin_prompt(key, &values).expect("rendering must be deterministic")
            );
        }
    }

    #[test]
    fn template_renderer_is_non_recursive_and_supports_literal_placeholders() {
        let definitions = &[PromptVariableDefinition {
            name: "value",
            description: "测试值",
            required: true,
            example: "示例",
        }];
        let values = BTreeMap::from([("value".to_owned(), "{{not_expanded}}".to_owned())]);
        let rendered =
            render_template("值：{{value}}；字面量：\\{{value}}", &values, definitions).unwrap();
        assert_eq!(rendered, "值：{{not_expanded}}；字面量：{{value}}");
    }

    #[test]
    fn template_validation_rejects_unknown_missing_and_malformed_placeholders() {
        let error = validate_builtin_prompt_pair(
            PromptKey::PreparationGenerate,
            "系统 {{unknown}}",
            "用户 {{novel_brief}}",
        )
        .unwrap_err();
        assert!(matches!(error, PromptTemplateError::UnknownVariable { .. }));

        let error = validate_builtin_prompt_pair(
            PromptKey::ChapterOutline,
            "系统",
            "用户 {{chapter_plan}}",
        )
        .unwrap_err();
        assert_eq!(
            error,
            PromptTemplateError::MissingRequiredPlaceholder {
                name: "chapter_guidance".to_owned()
            }
        );

        let error = validate_builtin_prompt_pair(
            PromptKey::PreparationGenerate,
            "系统 {{novel_brief",
            "用户 {{novel_brief}}",
        )
        .unwrap_err();
        assert_eq!(error, PromptTemplateError::UnclosedPlaceholder);

        let error =
            render_builtin_prompt(PromptKey::PreparationGenerate, &BTreeMap::new()).unwrap_err();
        assert_eq!(
            error,
            PromptTemplateError::MissingRuntimeVariable {
                name: "novel_brief".to_owned()
            }
        );
    }
}
