//! `AppState`: the shared handle every handler receives (spec/01 §3.1, §5 step 7).

use std::sync::Arc;

use crate::agent::SessionManager;
use crate::agent::tools::{ToolRegistry, spec::ToolSpecs};
use crate::config::Config;
use crate::db::Db;
use crate::events::EventBus;
use crate::prompts::PromptRegistry;
use crate::search::SearchService;
use crate::web::WebService;

/// Shared application state. Cheap to clone; every field is behind an `Arc`.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub db: Db,
    pub config: Config,
    /// Created at startup so `/health` can report it without a query.
    pub vec_version: String,
    /// The process-wide event bus (spec/10 §3).
    pub events: EventBus,
    /// The embedded prompt pack and the per-book resolved prompt sets (spec/09).
    pub prompts: Arc<PromptRegistry>,
    /// The tool contracts of `prompts/tools.toml`, compiled once (spec/05 §1.3).
    pub tool_specs: Arc<ToolSpecs>,
    /// The tool handlers of this build (spec/05 §1.2).
    pub tools: Arc<ToolRegistry>,
    /// One runner per running session (spec/03 §14).
    pub manager: SessionManager,
    /// The embedding model state, the runtime vec tables and the hybrid query (spec/07 §3–§5).
    pub search: Arc<SearchService>,
    /// The guarded `web_fetch` client and the converted-page cache (spec/07 §7).
    pub web: Arc<WebService>,
    /// A provider override for tests: when set, the runner uses this client instead of building
    /// one from the session's snapshot. Integration tests inject a `ScriptedProvider` here so no
    /// network is touched (spec/04 §7.1, AGENTS.md §5).
    #[cfg(any(test, feature = "testing"))]
    pub provider_override: Option<nekowriter_llm::ProviderClient>,
}

impl AppState {
    pub fn new(db: Db, config: Config, vec_version: String) -> Self {
        // The prompt pack and the tool contracts are startup invariants (spec/01 §5 step 6).
        let pack = PromptRegistry::embedded()
            .unwrap_or_else(|err| panic!("the embedded prompt pack is invalid: {err}"));
        let tool_specs = ToolSpecs::from_pack(pack.pack());
        // The debug session's test-only tools have schemas but no place in the shipped pack.
        let tool_specs = tool_specs.with_debug_tools();
        let events = EventBus::new();
        // `$HF_HOME` overrides `<data_dir>/models` for the fastembed cache (spec/01 §6).
        let cache_dir = config.model_cache_dir(std::env::var_os("HF_HOME").as_deref());
        let search = SearchService::new(db.clone(), cache_dir, events.clone());
        // Tests embed offline with the deterministic fake (spec/07 §4); the binary installs the
        // fastembed model in `prepare_state`.
        #[cfg(any(test, feature = "testing"))]
        {
            let dimension = nekowriter_embed::default_model().dimension;
            search.install(Arc::new(nekowriter_embed::FakeEmbedder::with_dimension(
                dimension,
            )));
        }
        Self {
            inner: Arc::new(AppStateInner {
                db,
                config,
                vec_version,
                events,
                // Startup invariant: the embedded pack is validated once per process, and a
                // failure is a build defect (spec/01 §5 step 6).
                prompts: Arc::new(pack),
                tool_specs: Arc::new(tool_specs),
                tools: Arc::new(ToolRegistry::new()),
                manager: SessionManager::new(),
                search: Arc::new(search),
                web: Arc::new(WebService::new()),
                #[cfg(any(test, feature = "testing"))]
                provider_override: None,
            }),
        }
    }

    /// Replaces the event bus (tests subscribe before the app is built).
    pub fn with_events(self, events: EventBus) -> Self {
        let search = Arc::new(self.inner.search.with_events(events.clone()));
        Self {
            inner: Arc::new(AppStateInner {
                db: self.inner.db.clone(),
                config: self.inner.config.clone(),
                vec_version: self.inner.vec_version.clone(),
                events,
                prompts: self.inner.prompts.clone(),
                tool_specs: self.inner.tool_specs.clone(),
                tools: self.inner.tools.clone(),
                manager: self.inner.manager.clone(),
                search,
                web: self.inner.web.clone(),
                #[cfg(any(test, feature = "testing"))]
                provider_override: self.inner.provider_override.clone(),
            }),
        }
    }

    /// Replaces the provider client used by every runner (tests inject `ScriptedProvider`).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_provider(self, provider: nekowriter_llm::ProviderClient) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db: self.inner.db.clone(),
                config: self.inner.config.clone(),
                vec_version: self.inner.vec_version.clone(),
                events: self.inner.events.clone(),
                prompts: self.inner.prompts.clone(),
                tool_specs: self.inner.tool_specs.clone(),
                tools: self.inner.tools.clone(),
                manager: self.inner.manager.clone(),
                search: self.inner.search.clone(),
                web: self.inner.web.clone(),
                provider_override: Some(provider),
            }),
        }
    }

    /// Replaces the search service (tests run with or without an embedder, spec/07 §5).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_search(self, search: SearchService) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db: self.inner.db.clone(),
                config: self.inner.config.clone(),
                vec_version: self.inner.vec_version.clone(),
                events: self.inner.events.clone(),
                prompts: self.inner.prompts.clone(),
                tool_specs: self.inner.tool_specs.clone(),
                tools: self.inner.tools.clone(),
                manager: self.inner.manager.clone(),
                search: Arc::new(search),
                web: self.inner.web.clone(),
                provider_override: self.inner.provider_override.clone(),
            }),
        }
    }

    /// The injected test provider, when there is one.
    #[cfg(any(test, feature = "testing"))]
    pub fn provider_override(&self) -> Option<&nekowriter_llm::ProviderClient> {
        self.inner.provider_override.as_ref()
    }

    pub fn db(&self) -> &Db {
        &self.inner.db
    }

    /// The pooled connection, for read-only queries (spec/01 §7).
    pub fn db_conn(&self) -> &sea_orm::DatabaseConnection {
        self.inner.db.conn()
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn vec_version(&self) -> &str {
        &self.inner.vec_version
    }

    pub fn events(&self) -> &EventBus {
        &self.inner.events
    }

    pub fn prompts(&self) -> &Arc<PromptRegistry> {
        &self.inner.prompts
    }

    /// The tool contracts of the embedded pack (spec/05 §1.3).
    pub fn tool_specs(&self) -> &Arc<ToolSpecs> {
        &self.inner.tool_specs
    }

    /// The registered tool handlers (spec/05 §1.2).
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.inner.tools
    }

    /// The runner manager (spec/03 §14).
    pub fn manager(&self) -> &SessionManager {
        &self.inner.manager
    }

    /// The embedding/index state and hybrid query (spec/07 §3–§5).
    pub fn search(&self) -> &SearchService {
        &self.inner.search
    }

    /// The guarded fetch client and the page cache (spec/07 §7).
    pub fn web(&self) -> &WebService {
        &self.inner.web
    }

    /// Replaces the web service (tests point a host at `wiremock`).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_web(self, web: WebService) -> Self {
        let web = Arc::new(web);
        Self {
            inner: Arc::new(AppStateInner {
                db: self.inner.db.clone(),
                config: self.inner.config.clone(),
                vec_version: self.inner.vec_version.clone(),
                events: self.inner.events.clone(),
                prompts: self.inner.prompts.clone(),
                tool_specs: self.inner.tool_specs.clone(),
                tools: self.inner.tools.clone(),
                manager: self.inner.manager.clone(),
                search: self.inner.search.clone(),
                web,
                provider_override: self.inner.provider_override.clone(),
            }),
        }
    }
}
