//! 受限模板语言与用户提示词模板。
//!
//! 模板语言只有一次性的 `{{variable_name}}` 文本替换：不执行表达式、循环或包含，
//! 变量值中的 `{{...}}` 不会被递归展开。需要字面量占位符时写 `\{{...}}`，渲染
//! 后去掉转义符。内建提示词覆盖与用户模板共用这一份实现，因此两者的未知变量、
//! 不完整括号与必需占位符规则不会各自漂移。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{ValidationError, validate_document_content, validate_required_text};

/// 用户模板目前唯一支持的变量类型。
pub const TEMPLATE_VALUE_TYPE_TEXT: &str = "text";
/// 模板标题长度上限。
pub const MAX_TEMPLATE_TITLE_CHARS: usize = 200;
/// 模板正文长度上限。
pub const MAX_TEMPLATE_BODY_CHARS: usize = 200_000;
/// 单个模板允许声明的变量数量上限。
pub const MAX_TEMPLATE_VARIABLES: usize = 50;
/// 变量名长度上限。
pub const MAX_TEMPLATE_VARIABLE_NAME_CHARS: usize = 64;
/// 变量说明长度上限。
pub const MAX_TEMPLATE_VARIABLE_DESCRIPTION_CHARS: usize = 2_000;
/// 变量示例值长度上限。
pub const MAX_TEMPLATE_VARIABLE_EXAMPLE_CHARS: usize = 2_000;

/// 受限模板语言的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TemplateError {
    #[error("模板中存在不完整的占位符")]
    UnclosedPlaceholder,
    #[error("模板中存在没有起始标记的 }}")]
    UnexpectedClosingDelimiter,
    #[error("模板变量名 {name:?} 无效")]
    InvalidVariableName { name: String },
    #[error("模板变量 {name:?} 没有声明")]
    UnknownVariable { name: String },
    #[error("必需模板变量 {name:?} 没有出现在模板中")]
    MissingRequiredPlaceholder { name: String },
    #[error("渲染时缺少必需变量 {name:?}")]
    MissingRuntimeVariable { name: String },
}

/// 一项模板变量的声明。
///
/// 内建提示词与用户模板都使用这个形状：界面据此显示占位符的类型、是否必需、
/// 示例值与来源，不需要阅读 Rust 代码。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateVariable {
    pub name: String,
    pub description: String,
    /// 目前固定为 [`TEMPLATE_VALUE_TYPE_TEXT`]。
    pub value_type: String,
    pub required: bool,
    /// 预览时使用的示例值；空字符串表示没有示例，此时必需变量必须由调用方提供。
    #[serde(default)]
    pub example: String,
}

impl TemplateVariable {
    /// # Errors
    ///
    /// 变量名非法或超出各自长度上限、类型不是受支持的文本类型时返回错误。
    pub fn validate(&self) -> Result<(), PromptTemplateValidationError> {
        if !template_variable_name_is_valid(&self.name) {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量名 {:?} 无效，只能使用字母、数字和下划线，且不能以数字开头",
                self.name
            )));
        }
        let description = self.description.trim();
        if description.is_empty() {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量 {} 缺少说明",
                self.name
            )));
        }
        if self.value_type != TEMPLATE_VALUE_TYPE_TEXT {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量 {} 的类型 {:?} 不受支持，只能使用 {TEMPLATE_VALUE_TYPE_TEXT}",
                self.name, self.value_type
            )));
        }
        if description.chars().count() > MAX_TEMPLATE_VARIABLE_DESCRIPTION_CHARS {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量 {} 的说明不能超过 {MAX_TEMPLATE_VARIABLE_DESCRIPTION_CHARS} 个字符",
                self.name
            )));
        }
        if self.example.chars().count() > MAX_TEMPLATE_VARIABLE_EXAMPLE_CHARS {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量 {} 的示例不能超过 {MAX_TEMPLATE_VARIABLE_EXAMPLE_CHARS} 个字符",
                self.name
            )));
        }
        Ok(())
    }
}

/// 用户模板或变量声明不合法时的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PromptTemplateValidationError {
    #[error("{0}")]
    Field(#[from] ValidationError),
    #[error("{0}")]
    Template(#[from] TemplateError),
    #[error("模板变量声明无效：{0}")]
    Variable(String),
}

/// 一份可插入章节指导或意见草稿的用户模板。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    pub id: uuid::Uuid,
    pub title: String,
    pub body: String,
    pub variables: Vec<TemplateVariable>,
    #[serde(with = "super::decimal_i64")]
    pub revision: i64,
    #[serde(with = "super::decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "super::decimal_i64")]
    pub updated_at_ms: i64,
}

/// 新建或替换一份用户模板的输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewPromptTemplate {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
}

impl NewPromptTemplate {
    /// 校验标题、正文与变量声明。
    ///
    /// 未知变量、缺失必需占位符、不完整括号与重复或非法变量名都在这里被拒绝，
    /// 因此保存和预览使用同一份判定。
    ///
    /// # Errors
    ///
    /// 标题或正文为空/超长、变量声明非法、正文引用了未声明变量，或声明的必需
    /// 变量没有出现在正文中时返回错误。
    pub fn validated(self) -> Result<Self, PromptTemplateValidationError> {
        let title = validate_required_text("模板标题", &self.title, MAX_TEMPLATE_TITLE_CHARS)?;
        let body = validate_document_content("模板正文", &self.body, MAX_TEMPLATE_BODY_CHARS)?;
        validate_template_variables(&self.variables)?;
        let used = validate_template(&body, &self.variables)?;
        require_declared_placeholders(&used, &self.variables)?;
        Ok(Self {
            title,
            body,
            variables: self.variables,
        })
    }
}

/// 一次模板预览的输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewTemplateRequest {
    pub body: String,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
    /// 调用方提供的真实值；未提供的变量回落到声明的示例值。
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

/// 渲染后的模板预览结果。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTemplateResult {
    pub rendered: String,
    pub used_variables: Vec<String>,
}

/// 校验一段模板只引用已声明的变量，并返回它实际使用的变量名。
///
/// # Errors
///
/// 出现不完整括号、多余的 `}}`、非法变量名或未声明变量时返回
/// [`TemplateError`]。
pub fn validate_template(
    source: &str,
    variables: &[TemplateVariable],
) -> Result<BTreeSet<String>, TemplateError> {
    let mut found = BTreeSet::new();
    let mut sink = String::new();
    walk_template(
        source,
        &mut sink,
        &|name| variables.iter().any(|variable| variable.name == name),
        &mut |name| {
            found.insert(name.to_owned());
            Ok(String::new())
        },
    )?;
    Ok(found)
}

/// 确认所有必需变量都出现在模板中。
///
/// # Errors
///
/// 任一 `required` 变量没有出现时返回
/// [`TemplateError::MissingRequiredPlaceholder`]。
pub fn require_declared_placeholders(
    found: &BTreeSet<String>,
    variables: &[TemplateVariable],
) -> Result<(), TemplateError> {
    for variable in variables {
        if variable.required && !found.contains(&variable.name) {
            return Err(TemplateError::MissingRequiredPlaceholder {
                name: variable.name.clone(),
            });
        }
    }
    Ok(())
}

/// 用给定变量值渲染模板；变量值永不递归展开。
///
/// # Errors
///
/// 模板不合法或缺少某个占位符的值时返回 [`TemplateError`]。
pub fn render_template(
    source: &str,
    variables: &[TemplateVariable],
    values: &BTreeMap<String, String>,
) -> Result<String, TemplateError> {
    let mut rendered = String::with_capacity(source.len());
    walk_template(
        source,
        &mut rendered,
        &|name| variables.iter().any(|variable| variable.name == name),
        &mut |name| {
            values
                .get(name)
                .cloned()
                .ok_or_else(|| TemplateError::MissingRuntimeVariable {
                    name: name.to_owned(),
                })
        },
    )?;
    Ok(rendered)
}

/// 用调用方提供的值渲染模板，未提供的占位符回落到声明的示例值。
///
/// 这是保存前预览使用的渲染方式：它既能让用户看到真实值的效果，也能在没有
/// 真实值时用示例解释每个占位符。
///
/// # Errors
///
/// 模板不合法，或某个占位符既没有提供值也没有非空示例时返回
/// [`TemplateError`]。
pub fn render_template_preview(
    source: &str,
    variables: &[TemplateVariable],
    values: &BTreeMap<String, String>,
) -> Result<String, TemplateError> {
    let mut rendered = String::with_capacity(source.len());
    walk_template(
        source,
        &mut rendered,
        &|name| variables.iter().any(|variable| variable.name == name),
        &mut |name| {
            values
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| {
                    variables
                        .iter()
                        .find(|variable| variable.name == name)
                        .map(|variable| variable.example.clone())
                        .filter(|example| !example.is_empty())
                })
                .ok_or_else(|| TemplateError::MissingRuntimeVariable {
                    name: name.to_owned(),
                })
        },
    )?;
    Ok(rendered)
}

/// 校验一组变量声明：名字合法、互不重复且各自满足长度与类型要求。
///
/// # Errors
///
/// 声明过多、变量重复或单项无效时返回错误。
pub fn validate_template_variables(
    variables: &[TemplateVariable],
) -> Result<(), PromptTemplateValidationError> {
    if variables.len() > MAX_TEMPLATE_VARIABLES {
        return Err(PromptTemplateValidationError::Variable(format!(
            "一个模板最多声明 {MAX_TEMPLATE_VARIABLES} 个变量"
        )));
    }
    let mut seen = BTreeSet::new();
    for variable in variables {
        variable.validate()?;
        if variable.name.chars().count() > MAX_TEMPLATE_VARIABLE_NAME_CHARS {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量名 {:?} 过长",
                variable.name
            )));
        }
        if !seen.insert(variable.name.as_str()) {
            return Err(PromptTemplateValidationError::Variable(format!(
                "变量 {} 重复声明",
                variable.name
            )));
        }
    }
    Ok(())
}

/// 判断一个模板变量名是否合法。
#[must_use]
pub fn template_variable_name_is_valid(name: &str) -> bool {
    let mut characters = name.bytes();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == b'_')
}

/// 单次遍历模板：普通文本原样写入 `out`，占位符由 `resolve` 决定写入内容。
///
/// 扫描与渲染共用这一个函数，因此两者对括号、转义和变量名的判定不可能分叉。
fn walk_template(
    source: &str,
    out: &mut String,
    declares: &dyn Fn(&str) -> bool,
    resolve: &mut dyn FnMut(&str) -> Result<String, TemplateError>,
) -> Result<(), TemplateError> {
    let mut cursor = 0;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if remaining.starts_with("\\{{") {
            let name_start = cursor + 3;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(TemplateError::UnclosedPlaceholder);
            };
            let name = &source[name_start..name_start + relative_end];
            if !template_variable_name_is_valid(name) {
                return Err(TemplateError::InvalidVariableName {
                    name: name.to_owned(),
                });
            }
            out.push_str("{{");
            out.push_str(name);
            out.push_str("}}");
            cursor = name_start + relative_end + 2;
            continue;
        }
        if remaining.starts_with("{{") {
            let name_start = cursor + 2;
            let Some(relative_end) = source[name_start..].find("}}") else {
                return Err(TemplateError::UnclosedPlaceholder);
            };
            let name = &source[name_start..name_start + relative_end];
            if !template_variable_name_is_valid(name) {
                return Err(TemplateError::InvalidVariableName {
                    name: name.to_owned(),
                });
            }
            if !declares(name) {
                return Err(TemplateError::UnknownVariable {
                    name: name.to_owned(),
                });
            }
            out.push_str(&resolve(name)?);
            cursor = name_start + relative_end + 2;
            continue;
        }
        if remaining.starts_with("}}") {
            return Err(TemplateError::UnexpectedClosingDelimiter);
        }
        let character = remaining
            .chars()
            .next()
            .expect("cursor is always a UTF-8 boundary before source end");
        out.push(character);
        cursor += character.len_utf8();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, required: bool, example: &str) -> TemplateVariable {
        TemplateVariable {
            name: name.to_owned(),
            description: format!("{name} 的说明"),
            value_type: TEMPLATE_VALUE_TYPE_TEXT.to_owned(),
            required,
            example: example.to_owned(),
        }
    }

    #[test]
    fn renderer_replaces_once_and_keeps_escaped_literals() {
        let variables = [variable("tone", true, "克制")];
        let rendered = render_template_preview(
            "语气：{{tone}}；字面量：\\{{tone}}",
            &variables,
            &BTreeMap::from([("tone".to_owned(), "{{tone}}".to_owned())]),
        )
        .unwrap();
        assert_eq!(rendered, "语气：{{tone}}；字面量：{{tone}}");
    }

    #[test]
    fn preview_falls_back_to_examples_and_rejects_undeclared_variables() {
        let variables = [variable("topic", true, "旧报纸")];
        let rendered =
            render_template_preview("主题：{{topic}}", &variables, &BTreeMap::new()).unwrap();
        assert_eq!(rendered, "主题：旧报纸");

        let error = validate_template("主题：{{missing}}", &variables).unwrap_err();
        assert_eq!(
            error,
            TemplateError::UnknownVariable {
                name: "missing".to_owned()
            }
        );
    }

    #[test]
    fn template_validation_rejects_unclosed_and_missing_required_placeholders() {
        let variables = [variable("topic", true, "旧报纸")];
        assert_eq!(
            validate_template("主题：{{topic", &variables).unwrap_err(),
            TemplateError::UnclosedPlaceholder
        );
        assert_eq!(
            validate_template("主题：tone }}", &variables).unwrap_err(),
            TemplateError::UnexpectedClosingDelimiter
        );
        let used = validate_template("主题：{{topic}}", &variables).unwrap();
        assert!(require_declared_placeholders(&used, &variables).is_ok());
        let used = validate_template("主题：未使用占位符", &variables).unwrap();
        assert_eq!(
            require_declared_placeholders(&used, &variables).unwrap_err(),
            TemplateError::MissingRequiredPlaceholder {
                name: "topic".to_owned()
            }
        );
    }

    #[test]
    fn new_template_rejects_unknown_variables_and_duplicate_declarations() {
        let error = NewPromptTemplate {
            title: "  ".to_owned(),
            body: "正文".to_owned(),
            variables: Vec::new(),
        }
        .validated()
        .unwrap_err();
        assert!(matches!(error, PromptTemplateValidationError::Field(_)));

        let error = NewPromptTemplate {
            title: "模板".to_owned(),
            body: "{{unknown}}".to_owned(),
            variables: Vec::new(),
        }
        .validated()
        .unwrap_err();
        assert!(matches!(error, PromptTemplateValidationError::Template(_)));

        let duplicated = variable("topic", false, "");
        let error = NewPromptTemplate {
            title: "模板".to_owned(),
            body: "{{topic}}".to_owned(),
            variables: vec![duplicated.clone(), duplicated],
        }
        .validated()
        .unwrap_err();
        assert!(matches!(error, PromptTemplateValidationError::Variable(_)));
    }
}
