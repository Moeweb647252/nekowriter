//! Configuration from CLI flags, environment variables and (in debug builds) `.env`.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// The listen address default (spec/01 §6).
pub const DEFAULT_BIND: &str = "127.0.0.1:7878";

#[derive(Debug, Clone, Parser)]
#[command(
    name = "nekowriter",
    version,
    about = "A local-first AI long-form fiction studio"
)]
pub struct Config {
    /// Listen address.
    #[arg(long, env = "NW_BIND", default_value = DEFAULT_BIND)]
    pub bind: SocketAddr,

    /// Data directory: database, models, logs.
    #[arg(long, env = "NW_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Serve SPA files from this directory instead of the embedded assets.
    #[arg(long, env = "NW_WEB_DIR")]
    pub web_dir: Option<PathBuf>,

    /// Allow the `mock` protocol for providers.
    #[arg(long, env = "NW_ENABLE_DEMO_PROVIDER")]
    pub enable_demo_provider: bool,

    /// Allow binding to a non-loopback address.
    #[arg(long)]
    pub allow_remote: bool,
}

impl Config {
    /// Parses the process arguments and, in debug builds, loads `.env` first.
    pub fn load() -> anyhow::Result<Self> {
        if cfg!(debug_assertions) {
            // A missing `.env` is not an error.
            let _ = dotenvy::dotenv();
        }
        Ok(Self::parse())
    }

    /// Resolves the data directory, creating it (and `models/` and `logs/`) when needed.
    pub fn resolve_data_dir(&self) -> anyhow::Result<PathBuf> {
        let dir = match &self.data_dir {
            Some(dir) => dir.clone(),
            None => default_data_dir()?,
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("cannot create data directory {}: {e}", dir.display()))?;
        for sub in ["models", "logs"] {
            let path = dir.join(sub);
            std::fs::create_dir_all(&path).map_err(|e| {
                anyhow::anyhow!("cannot create data directory {}: {e}", path.display())
            })?;
        }
        Ok(dir)
    }

    /// True when the demo provider should be available: the flag is set, or this is a debug
    /// build (spec/01 §6).
    pub fn demo_provider_enabled(&self) -> bool {
        self.enable_demo_provider || cfg!(debug_assertions)
    }

    /// Refuses a non-loopback bind address unless `--allow-remote` was passed (spec/01 §5).
    pub fn check_bind_allowed(&self) -> anyhow::Result<()> {
        if self.bind.ip().is_loopback() || self.allow_remote {
            if self.allow_remote && !self.bind.ip().is_loopback() {
                tracing::warn!(
                    bind = %self.bind,
                    "binding to a non-loopback address; the API has no authentication"
                );
            }
            return Ok(());
        }
        anyhow::bail!(
            "refusing to bind to the non-loopback address {}; the API has no authentication. \
             Pass --allow-remote to override.",
            self.bind
        )
    }
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "NekoWriter")
        .ok_or_else(|| anyhow::anyhow!("cannot determine the OS data directory"))?;
    Ok(dirs.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_bind(bind: &str, allow_remote: bool) -> Config {
        Config {
            bind: bind.parse().expect("valid socket address"),
            data_dir: None,
            web_dir: None,
            enable_demo_provider: false,
            allow_remote,
        }
    }

    #[test]
    fn loopback_bind_is_allowed() {
        assert!(
            config_with_bind("127.0.0.1:7878", false)
                .check_bind_allowed()
                .is_ok()
        );
        assert!(
            config_with_bind("[::1]:7878", false)
                .check_bind_allowed()
                .is_ok()
        );
    }

    #[test]
    fn non_loopback_bind_is_refused_without_the_flag() {
        let err = config_with_bind("0.0.0.0:7878", false)
            .check_bind_allowed()
            .expect_err("non-loopback bind must be refused");
        assert!(err.to_string().contains("--allow-remote"));
    }

    #[test]
    fn non_loopback_bind_is_allowed_with_the_flag() {
        assert!(
            config_with_bind("0.0.0.0:7878", true)
                .check_bind_allowed()
                .is_ok()
        );
    }

    #[test]
    fn demo_provider_defaults_on_in_debug_builds() {
        let config = config_with_bind("127.0.0.1:7878", false);
        assert_eq!(config.demo_provider_enabled(), cfg!(debug_assertions));
    }
}
