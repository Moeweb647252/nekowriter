//! Application settings (`app_settings`, spec/02 §4.1, spec/10 §2.1).
//!
//! Keys: `roles.defaults` (`RoleModels`), `book_defaults` (`BookSettings`), `search`,
//! `embedding`. Reads fall back to the documented defaults when a key is absent, so a fresh
//! database behaves like the spec describes without a seeding step.

use sea_orm::{ConnectionTrait, EntityTrait, Set};

use crate::api::dto::settings::{
    AppSettingsDto, AppSettingsPatch, EmbeddingSettingsDto, SearchSettingsDto,
};
use crate::db::entities::app_settings;
use crate::db::json::{BookSettings, RoleModels};
use crate::error::AppError;

use super::{json, now, parse_json};

/// `app_settings` key names (spec/02 §4.1).
const KEY_ROLES: &str = "roles.defaults";
const KEY_BOOK_DEFAULTS: &str = "book_defaults";
const KEY_SEARCH: &str = "search";
const KEY_EMBEDDING: &str = "embedding";

/// Letters kept unmasked at the end of a stored secret (spec/01 §10.3).
const MASK_TAIL: usize = 4;

/// Reads every setting, filling absent keys with their defaults.
pub async fn get(conn: &impl ConnectionTrait) -> Result<AppSettingsDto, AppError> {
    let rows = app_settings::Entity::find().all(conn).await?;
    let mut dto = AppSettingsDto {
        roles_defaults: RoleModels::default(),
        book_defaults: BookSettings::default(),
        search: SearchSettingsDto::default(),
        embedding: EmbeddingSettingsDto::default(),
    };
    for row in rows {
        match row.key.as_str() {
            KEY_ROLES => dto.roles_defaults = parse_json(KEY_ROLES, &row.value)?,
            KEY_BOOK_DEFAULTS => dto.book_defaults = parse_json(KEY_BOOK_DEFAULTS, &row.value)?,
            KEY_SEARCH => dto.search = parse_json(KEY_SEARCH, &row.value)?,
            KEY_EMBEDDING => dto.embedding = parse_json(KEY_EMBEDDING, &row.value)?,
            other => tracing::debug!(key = other, "ignoring an unknown app_settings key"),
        }
    }
    mask_search_secrets(&mut dto.search);
    Ok(dto)
}

/// The settings new books start with (spec/02 §4.1 `book_defaults`).
pub async fn book_defaults(conn: &impl ConnectionTrait) -> Result<BookSettings, AppError> {
    Ok(get(conn).await?.book_defaults)
}

/// Applies a patch inside the caller's transaction and returns the new settings.
pub async fn update(
    tx: &impl ConnectionTrait,
    patch: &AppSettingsPatch,
) -> Result<AppSettingsDto, AppError> {
    let current = get(tx).await?;

    let roles = match &patch.roles_defaults {
        Some(roles) => roles.clone(),
        None => current.roles_defaults.clone(),
    };
    let book_defaults = match &patch.book_defaults {
        Some(settings) => {
            validate_book_settings(settings)?;
            settings.clone()
        }
        None => current.book_defaults.clone(),
    };
    // `get` masks secrets, so merging a patch over it would persist the mask and destroy the
    // stored key. Read the raw section instead; both search secrets depend on this (D-127).
    let mut search = super::search_settings::stored(tx).await?;
    if let Some(patch_search) = &patch.search {
        if let Some(provider) = patch_search.provider {
            search.provider = provider;
        }
        // A masked value means "unchanged": the UI round-trips what it received.
        apply_secret(&patch_search.tavily_api_key, &mut search.tavily_api_key);
        apply_secret(&patch_search.deepseek_api_key, &mut search.deepseek_api_key);
        if let Some(base_url) = &patch_search.searxng_base_url {
            search.searxng_base_url = if base_url.is_empty() {
                None
            } else {
                Some(base_url.clone())
            };
        }
        if let Some(max_results) = patch_search.max_results {
            search.max_results = max_results;
        }
    }
    // `max_results` and the provider configuration follow spec/02 §4.1 and spec/05 §7 (a default
    // above the tool's cap would be silently clamped).
    super::search_settings::validate(&search)?;
    let embedding = match &patch.embedding {
        Some(embedding) => {
            // The key must name a supported model (crates/embed/models.toml, spec/07 §4).
            if nekowriter_embed::model(&embedding.model).is_none() {
                return Err(AppError::validation(format!(
                    "unknown embedding model {}",
                    embedding.model
                )));
            }
            embedding.clone()
        }
        None => current.embedding.clone(),
    };

    upsert(tx, KEY_ROLES, &json(&roles)?).await?;
    upsert(tx, KEY_BOOK_DEFAULTS, &json(&book_defaults)?).await?;
    upsert(tx, KEY_SEARCH, &json(&search)?).await?;
    upsert(tx, KEY_EMBEDDING, &json(&embedding)?).await?;

    let mut response = AppSettingsDto {
        roles_defaults: roles,
        book_defaults,
        search,
        embedding,
    };
    mask_search_secrets(&mut response.search);
    Ok(response)
}

/// Rejects settings that would break the writer's accounting.
fn validate_book_settings(settings: &BookSettings) -> Result<(), AppError> {
    if settings.target_words_min <= 0 {
        return Err(AppError::validation("target_words_min must be positive"));
    }
    if settings.target_words_max < settings.target_words_min {
        return Err(AppError::validation(
            "target_words_max must be at least target_words_min",
        ));
    }
    if settings.target_words_max > 60_000 {
        return Err(AppError::validation(
            "target_words_max must be at most 60000 words",
        ));
    }
    if settings.recent_summary_count < 0 {
        return Err(AppError::validation(
            "recent_summary_count must not be negative",
        ));
    }
    if settings.midterm_window < 0 {
        return Err(AppError::validation("midterm_window must not be negative"));
    }
    Ok(())
}

async fn upsert(tx: &impl ConnectionTrait, key: &str, value: &str) -> Result<(), AppError> {
    use sea_orm::sea_query::OnConflict;
    let model = app_settings::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(value.to_owned()),
        updated_at: Set(now()),
    };
    app_settings::Entity::insert(model)
        .on_conflict(
            OnConflict::column(app_settings::Column::Key)
                .update_columns([app_settings::Column::Value, app_settings::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(tx)
        .await?;
    Ok(())
}

/// Applies the secret fields of a patch: a masked value means "unchanged" and an empty value
/// clears the secret (spec/10 §2.1). Writing a round-tripped mask back verbatim would destroy the
/// stored key, which is why both secrets go through here (D-127).
fn apply_secret(patch: &Option<String>, current: &mut Option<String>) {
    if let Some(key) = patch
        && !is_masked(key)
    {
        *current = if key.is_empty() {
            None
        } else {
            Some(key.clone())
        };
    }
}

/// Masks every secret of the search settings on the way out (spec/01 §10.3).
fn mask_search_secrets(search: &mut SearchSettingsDto) {
    search.tavily_api_key = search.tavily_api_key.as_deref().map(mask_secret);
    search.deepseek_api_key = search.deepseek_api_key.as_deref().map(mask_secret);
}

/// Masks a stored secret as `…abcd` (spec/01 §10.3). Short secrets are fully masked.
pub fn mask_secret(secret: &str) -> String {
    if secret.starts_with("env:") {
        // Environment indirection is not a secret: it names one.
        return secret.to_owned();
    }
    let characters: Vec<char> = secret.chars().collect();
    if characters.len() <= MASK_TAIL {
        return "…".to_owned();
    }
    let tail: String = characters[characters.len() - MASK_TAIL..].iter().collect();
    format!("…{tail}")
}

/// True when a value looks like a masked secret, so it must not overwrite the stored one.
pub fn is_masked(value: &str) -> bool {
    value.starts_with('…') || value.contains('…')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_shows_only_the_tail() {
        assert_eq!(mask_secret("tvly-abcdefgh"), "…efgh");
        assert_eq!(mask_secret("env:TAVILY_KEY"), "env:TAVILY_KEY");
        assert_eq!(mask_secret("abc"), "…");
        assert_eq!(mask_secret(""), "…");
        assert!(is_masked("…efgh"));
        assert!(!is_masked("tvly-abcdefgh"));
    }

    #[test]
    fn book_settings_validation_table() {
        let cases: &[(BookSettings, bool)] = &[
            (BookSettings::default(), true),
            (
                BookSettings {
                    target_words_min: 0,
                    ..BookSettings::default()
                },
                false,
            ),
            (
                BookSettings {
                    target_words_min: 3000,
                    target_words_max: 2999,
                    ..BookSettings::default()
                },
                false,
            ),
            (
                BookSettings {
                    target_words_max: 60_001,
                    ..BookSettings::default()
                },
                false,
            ),
            (
                BookSettings {
                    recent_summary_count: -1,
                    ..BookSettings::default()
                },
                false,
            ),
            (
                BookSettings {
                    midterm_window: -1,
                    ..BookSettings::default()
                },
                false,
            ),
            (
                BookSettings {
                    granularity: crate::db::json::Granularity::Free,
                    auto_pause: true,
                    pause_after_outline: false,
                    target_words_min: 1000,
                    target_words_max: 1000,
                    recent_summary_count: 0,
                    midterm_window: 0,
                    auto_maintenance: false,
                },
                true,
            ),
        ];
        for (settings, valid) in cases {
            assert_eq!(
                validate_book_settings(settings).is_ok(),
                *valid,
                "settings {settings:?}"
            );
        }
    }
}
