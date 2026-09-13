//! 内建提示词目录、变量声明与上下文编排。受限模板语言在
//! `nekowriter_core::template` 中只实现一次，用户模板与内建覆盖共用它。
//!
//! 模板语言刻意只支持一次性的 `{{variable_name}}` 文本替换。它不执行表达式、
//! 循环、包含或递归替换，因此用户内容即使包含 `{{...}}` 也不会被再次解释。

use std::collections::BTreeMap;

use nekowriter_core::{
    TemplateError, TemplateVariable, render_template, require_declared_placeholders,
    validate_template,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{BuiltinPrompt, PromptKey, builtin_prompt};

/// 一项内建提示词变量的公开说明。
///
/// 变量说明是界面的一部分：用户修改提示词覆盖时必须能看到每个占位符的类型、
/// 是否必需、示例值以及运行值来自哪里，而不必阅读 Rust 代码。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptVariableDefinition {
    pub name: &'static str,
    pub description: &'static str,
    /// 受限模板语言目前只支持文本变量。
    pub value_type: &'static str,
    pub required: bool,
    pub example: &'static str,
    /// 运行时值的来源，用于向用户解释占位符会被什么替换。
    pub source: &'static str,
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

const TEXT_VALUE_TYPE: &str = "text";

const NOVEL_BRIEF: PromptVariableDefinition = PromptVariableDefinition {
    name: "novel_brief",
    description: "当前稿本的小说目标。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "一部关于海港城旧报纸的悬疑小说",
    source: "当前稿本的小说目标修订",
};
const CHAPTER_PLAN: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_plan",
    description: "当前章节的已锁定计划项（标题和目标）。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "第 1 章：雨夜的来客\n主角在码头收到匿名报纸。",
    source: "本稿本已审核通过的粗纲章节计划",
};
const CHAPTER_GUIDANCE: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_guidance",
    description: "用户为当前章节提交的指导或本轮意见。空字符串表示用户未额外填写。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "突出雨夜和主角的犹豫。",
    source: "用户保存的本章章节目指导",
};
const CHAPTER_OUTLINE: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_outline",
    description: "当前章节已经审核通过的细纲全文。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "林舟抵达码头，发现旧报纸上的名字。",
    source: "本章已审核通过的细纲修订",
};
const CHAPTER_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_content",
    description: "当前章节已接受的正文全文。空字符串表示尚无正文。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "雨落在旧码头的铁皮屋顶上。",
    source: "本章当前已接受的正文修订",
};
const TARGET_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "target_content",
    description: "本次待审核的目标产物或正文。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "待审核的产物全文。",
    source: "本次委托审核的目标修订",
};
const APPROVED_CONTEXT: PromptVariableDefinition = PromptVariableDefinition {
    name: "approved_context",
    description: "与当前审核目标相关、已经审核通过的上游产物全文。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "已通过的背景与资料。",
    source: "与目标修订存在依赖关系的已通过产物",
};
const CHAPTER_CONTEXT: PromptVariableDefinition = PromptVariableDefinition {
    name: "chapter_context",
    description: "当前章节的锁定计划项和用户指导。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "第 1 章目标：在码头收到匿名报纸。",
    source: "本章锁定计划项与用户章节目指导",
};
const BASE_CONTENT: PromptVariableDefinition = PromptVariableDefinition {
    name: "base_content",
    description: "正文编辑提案所基于的正式正文。",
    value_type: TEXT_VALUE_TYPE,
    required: true,
    example: "雨落在旧码头的铁皮屋顶上。",
    source: "编辑提案所基于的正式正文修订",
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

/// 把一个内建提示词的静态变量目录转换成受限模板语言的声明形状。
#[must_use]
pub fn builtin_template_variables(key: PromptKey) -> Vec<TemplateVariable> {
    builtin_prompt_variables(key)
        .iter()
        .map(|definition| TemplateVariable {
            name: definition.name.to_owned(),
            description: definition.description.to_owned(),
            value_type: definition.value_type.to_owned(),
            required: definition.required,
            example: definition.example.to_owned(),
        })
        .collect()
}

/// 返回渲染某内建提示词所需的示例变量值。
///
/// 保存前预览使用这些示例值，因此预览不需要真实稿本、密钥或正在运行的 Agent。
#[must_use]
pub fn builtin_prompt_examples(key: PromptKey) -> BTreeMap<String, String> {
    builtin_prompt_variables(key)
        .iter()
        .map(|definition| (definition.name.to_owned(), definition.example.to_owned()))
        .collect()
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
) -> Result<(), TemplateError> {
    let variables = builtin_template_variables(key);
    let mut placeholders = validate_template(system, &variables)?;
    placeholders.extend(validate_template(user, &variables)?);
    require_declared_placeholders(&placeholders, &variables)
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
) -> Result<RenderedBuiltinPrompt, TemplateError> {
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
) -> Result<RenderedBuiltinPrompt, TemplateError> {
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
) -> Result<RenderedBuiltinPrompt, TemplateError> {
    validate_builtin_prompt_pair(key, system, user)?;
    let definitions = builtin_template_variables(key);
    for definition in &definitions {
        if definition.required && !variables.contains_key(&definition.name) {
            return Err(TemplateError::MissingRuntimeVariable {
                name: definition.name.clone(),
            });
        }
    }
    let system = render_template(system, &definitions, variables)?;
    let user = render_template(user, &definitions, variables)?;
    Ok(RenderedBuiltinPrompt {
        key,
        system_content_hash: content_hash(&system),
        user_content_hash: content_hash(&user),
        system,
        user,
    })
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
            let values = builtin_prompt_examples(key);
            let rendered = render_builtin_prompt(key, &values).unwrap();
            assert_eq!(rendered.key, key);
            assert_ne!(rendered.system_content_hash, "");
            assert_ne!(rendered.user_content_hash, "");
            assert_eq!(
                rendered,
                render_builtin_prompt(key, &values).expect("rendering must be deterministic")
            );
        }
    }

    #[test]
    fn builtin_prompt_pair_reuses_the_shared_template_rules() {
        let error = validate_builtin_prompt_pair(
            PromptKey::PreparationGenerate,
            "系统 {{unknown}}",
            "用户 {{novel_brief}}",
        )
        .unwrap_err();
        assert!(matches!(error, TemplateError::UnknownVariable { .. }));

        let error = validate_builtin_prompt_pair(
            PromptKey::ChapterOutline,
            "系统",
            "用户 {{chapter_plan}}",
        )
        .unwrap_err();
        assert_eq!(
            error,
            TemplateError::MissingRequiredPlaceholder {
                name: "chapter_guidance".to_owned()
            }
        );

        let error = validate_builtin_prompt_pair(
            PromptKey::PreparationGenerate,
            "系统 {{novel_brief",
            "用户 {{novel_brief}}",
        )
        .unwrap_err();
        assert_eq!(error, TemplateError::UnclosedPlaceholder);

        let error =
            render_builtin_prompt(PromptKey::PreparationGenerate, &BTreeMap::new()).unwrap_err();
        assert_eq!(
            error,
            TemplateError::MissingRuntimeVariable {
                name: "novel_brief".to_owned()
            }
        );
    }
}
