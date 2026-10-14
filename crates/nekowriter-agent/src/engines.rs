//! The engine registries (`docs/product-spec.md` §12.3, D11). Research and accumulation
//! engines plug in here. This build registers none, so no run declares `research_web` and
//! nothing offers accumulation; registering an engine adds its invocation path.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use nekowriter_core::engine::{
    AccumulationRequest, Candidate, EngineInfo, EngineList, EngineUnavailable, ResearchReport,
    ResearchRequest,
};
use thiserror::Error;

/// Why an engine could not finish.
#[derive(Debug, Error)]
#[error("the engine failed: {0}")]
pub struct EngineError(pub String);

/// Answers a question from the web with findings and their sources.
pub trait ResearchEngine: Send + Sync {
    fn info(&self) -> EngineInfo;
    fn research<'a>(
        &'a self,
        request: &'a ResearchRequest,
    ) -> BoxFuture<'a, Result<ResearchReport, EngineError>>;
}

/// Proposes knowledge entries, with their sources, from the user's hints.
pub trait AccumulationEngine: Send + Sync {
    fn info(&self) -> EngineInfo;
    fn accumulate<'a>(
        &'a self,
        request: &'a AccumulationRequest,
    ) -> BoxFuture<'a, Result<Vec<Candidate>, EngineError>>;
}

/// The engines the application can use.
#[derive(Clone, Default)]
pub struct Engines {
    research: Vec<Arc<dyn ResearchEngine>>,
    accumulation: Vec<Arc<dyn AccumulationEngine>>,
}

impl Engines {
    /// The engines this build provides: none (D11).
    #[must_use]
    pub fn builtin() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn research(&self) -> EngineList {
        list(self.research.iter().map(|engine| engine.info()))
    }

    #[must_use]
    pub fn accumulation(&self) -> EngineList {
        list(self.accumulation.iter().map(|engine| engine.info()))
    }
}

fn list(engines: impl Iterator<Item = EngineInfo>) -> EngineList {
    let engines: Vec<EngineInfo> = engines.collect();
    let unavailable = engines
        .is_empty()
        .then_some(EngineUnavailable::NoneRegistered);
    EngineList {
        engines,
        unavailable,
    }
}
