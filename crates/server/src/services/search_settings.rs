//! Web search settings (spec/02 §4.1, spec/07 §6): reading the stored configuration, resolving an
//! `env:NAME` key at call time, and the `POST /app-settings/search/test` probe.
//!
//! No credential leaves this module: the DTO the API returns is masked elsewhere, the config used
//! for a request carries the resolved secret, and nothing is logged.

use sea_orm::{ConnectionTrait, EntityTrait};

use crate::api::dto::search::{SearchHitDto, SearchTestResponse};
use crate::api::dto::settings::{SearchProvider, SearchSettingsDto};
use crate::db::entities::app_settings;
use crate::error::{AppError, ErrorCode};
use crate::web::search::{self, SearchConfig, SearchFailure, SearchProviderKind};
use crate::web::{DEFAULT_RESULTS, MAX_RESULTS_CAP};

/// The `app_settings` key of the search configuration (spec/02 §4.1).
const KEY_SEARCH: &str = "search";

/// The stored configuration, **unmasked** (the API masks it on the way out; this is what a
/// request needs).
pub async fn stored(conn: &impl ConnectionTrait) -> Result<SearchSettingsDto, AppError> {
    let row = app_settings::Entity::find_by_id(KEY_SEARCH)
        .one(conn)
        .await?;
    match row {
        Some(row) => Ok(super::parse_json(KEY_SEARCH, &row.value)?),
        None => Ok(SearchSettingsDto::default()),
    }
}

/// The stored configuration as the provider layer wants it, with an `env:NAME` key resolved
/// (spec/01 §10.3). A missing environment variable is `provider.auth` naming the variable.
pub async fn config(state: &crate::state::AppState) -> Result<SearchConfig, AppError> {
    let settings = stored(state.db_conn()).await?;
    let mut config = to_config(&settings)?;
    // The provider hosts are fixed by spec/07 §6; only a test build overrides them.
    config.tavily_base_url = state.web().tavily_base_url().to_owned();
    config.deepseek_base_url = state.web().deepseek_base_url().to_owned();
    Ok(config)
}

/// Resolves `env:NAME` and maps the DTO onto [`SearchConfig`].
pub fn to_config(settings: &SearchSettingsDto) -> Result<SearchConfig, AppError> {
    let provider = match settings.provider {
        SearchProvider::None => SearchProviderKind::None,
        SearchProvider::Tavily => SearchProviderKind::Tavily,
        SearchProvider::Deepseek => SearchProviderKind::Deepseek,
        SearchProvider::Searxng => SearchProviderKind::Searxng,
    };
    let tavily_api_key = resolve_secret(settings.tavily_api_key.as_deref())?;
    let deepseek_api_key = resolve_secret(settings.deepseek_api_key.as_deref())?;
    Ok(SearchConfig {
        provider,
        tavily_api_key,
        tavily_base_url: search::TAVILY_BASE_URL.to_owned(),
        deepseek_api_key,
        deepseek_base_url: search::DEEPSEEK_BASE_URL.to_owned(),
        searxng_base_url: settings
            .searxng_base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned),
        max_results: usize::try_from(settings.max_results)
            .unwrap_or(DEFAULT_RESULTS)
            .clamp(1, MAX_RESULTS_CAP),
    })
}

/// Resolves one stored secret: a literal is used as it is and `env:NAME` is read at call time
/// (spec/01 §10.3). A missing environment variable is `provider.auth` naming the variable.
fn resolve_secret(value: Option<&str>) -> Result<Option<String>, AppError> {
    match value {
        Some(key) if key.starts_with("env:") => {
            let name = &key["env:".len()..];
            match std::env::var(name) {
                Ok(value) if !value.is_empty() => Ok(Some(value)),
                _ => Err(AppError::new(
                    ErrorCode::ProviderAuth,
                    format!("environment variable {name} is not set"),
                )
                .with_details(serde_json::json!({ "env": name }))),
            }
        }
        Some(key) if !key.is_empty() => Ok(Some(key.to_owned())),
        _ => Ok(None),
    }
}

/// Validates the search section of a settings patch (spec/02 §4.1, spec/05 §7).
///
/// The tool can never return more than [`MAX_RESULTS_CAP`] results, so a default above it would be
/// silently clamped; refusing it keeps the stored setting honest.
pub fn validate(settings: &SearchSettingsDto) -> Result<(), AppError> {
    if !(1..=MAX_RESULTS_CAP as i32).contains(&settings.max_results) {
        return Err(AppError::invalid_fields(serde_json::Map::from_iter([(
            "search.max_results".to_owned(),
            serde_json::Value::String(format!(
                "max_results must be between 1 and {MAX_RESULTS_CAP}"
            )),
        )])));
    }
    if settings.provider == SearchProvider::Searxng && settings.searxng_base_url.is_none() {
        return Err(AppError::validation(
            "searxng needs a base URL when it is the selected provider",
        ));
    }
    if settings.provider == SearchProvider::Tavily && settings.tavily_api_key.is_none() {
        return Err(AppError::validation(
            "tavily needs an API key when it is the selected provider",
        ));
    }
    if settings.provider == SearchProvider::Deepseek && settings.deepseek_api_key.is_none() {
        return Err(AppError::validation(
            "deepseek needs an API key when it is the selected provider",
        ));
    }
    Ok(())
}

/// `POST /api/app-settings/search/test` (spec/10 §2.1): one real search with the stored provider.
///
/// The mapping is the one D-058 fixes: no provider is `search.not_configured` (409), a rejected key
/// is `provider.auth`, and every other failure is `provider.unavailable`.
pub async fn test_query(
    state: &crate::state::AppState,
    query: &str,
) -> Result<SearchTestResponse, AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::invalid_fields(serde_json::Map::from_iter([(
            "query".to_owned(),
            serde_json::Value::String("a test query must not be empty".to_owned()),
        )])));
    }
    let config = config(state).await?;
    if !search::is_configured(&config) {
        return Err(AppError::new(
            ErrorCode::SearchNotConfigured,
            "no web search provider is configured",
        ));
    }
    let client = search::shared_client()
        .map_err(|error| AppError::internal(format!("cannot build the search client: {error}")))?;
    let limit = config.result_count(None);
    match search::search(client, &config, query, limit).await {
        Ok(hits) => Ok(SearchTestResponse {
            hits: hits
                .into_iter()
                .map(|hit| SearchHitDto {
                    title: hit.title,
                    url: hit.url,
                    snippet: hit.snippet,
                    published: hit.published,
                })
                .collect(),
        }),
        Err(error) => Err(match error.reason {
            SearchFailure::Auth => AppError::new(
                ErrorCode::ProviderAuth,
                "the search provider rejected the key",
            ),
            other => AppError::new(
                ErrorCode::ProviderUnavailable,
                format!("the search provider failed ({})", other.as_str()),
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_patch_must_fit_the_tool_cap() {
        let mut settings = SearchSettingsDto::default();
        assert!(validate(&settings).is_ok());
        settings.max_results = 0;
        assert_eq!(
            validate(&settings).expect_err("zero").code,
            ErrorCode::ValidationFailed
        );
        settings.max_results = 11;
        assert_eq!(
            validate(&settings).expect_err("eleven").code,
            ErrorCode::ValidationFailed
        );
        settings.max_results = 10;
        assert!(validate(&settings).is_ok());
    }

    #[test]
    fn a_selected_provider_demands_its_configuration() {
        let searxng = SearchSettingsDto {
            provider: SearchProvider::Searxng,
            ..SearchSettingsDto::default()
        };
        assert!(validate(&searxng).is_err());
        let tavily = SearchSettingsDto {
            provider: SearchProvider::Tavily,
            ..SearchSettingsDto::default()
        };
        assert!(validate(&tavily).is_err());
        let deepseek = SearchSettingsDto {
            provider: SearchProvider::Deepseek,
            ..SearchSettingsDto::default()
        };
        assert!(validate(&deepseek).is_err());
    }

    #[test]
    fn the_stored_shape_maps_onto_the_provider_config() {
        let settings = SearchSettingsDto {
            provider: SearchProvider::Searxng,
            searxng_base_url: Some("https://searx.example/".to_owned()),
            max_results: 4,
            ..SearchSettingsDto::default()
        };
        let config = to_config(&settings).expect("config");
        assert_eq!(config.provider, SearchProviderKind::Searxng);
        assert_eq!(
            config.searxng_base_url.as_deref(),
            Some("https://searx.example/")
        );
        assert_eq!(config.result_count(None), 4);
    }

    #[test]
    fn the_deepseek_key_maps_onto_the_provider_config() {
        let settings = SearchSettingsDto {
            provider: SearchProvider::Deepseek,
            deepseek_api_key: Some("sk-deepseek-test".to_owned()),
            ..SearchSettingsDto::default()
        };
        let config = to_config(&settings).expect("config");
        assert_eq!(config.provider, SearchProviderKind::Deepseek);
        assert_eq!(config.deepseek_api_key.as_deref(), Some("sk-deepseek-test"));
        assert_eq!(config.deepseek_base_url, search::DEEPSEEK_BASE_URL);
        assert!(search::is_configured(&config));
    }

    #[test]
    fn an_unset_environment_key_is_provider_auth() {
        let settings = SearchSettingsDto {
            provider: SearchProvider::Tavily,
            tavily_api_key: Some("env:NEKOWRITER_M8_MISSING_KEY".to_owned()),
            ..SearchSettingsDto::default()
        };
        let error = to_config(&settings).expect_err("missing env var");
        assert_eq!(error.code, ErrorCode::ProviderAuth);
        assert_eq!(
            error.details.expect("details")["env"],
            "NEKOWRITER_M8_MISSING_KEY"
        );
    }
}
