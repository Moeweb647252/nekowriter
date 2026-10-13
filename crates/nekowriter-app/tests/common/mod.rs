//! A running server on a free port with a temporary data directory, and a client for it.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::path::Path;

use nekowriter_app::{Config, RunningServer};
use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

pub struct TestServer {
    pub server: RunningServer,
    pub client: Client,
    pub base: String,
    /// The temporary data directory the server owns, if it owns one.
    owned_data_dir: Option<TempDir>,
}

impl TestServer {
    /// Starts a server with a fresh data directory that is removed with the server.
    pub async fn start() -> Self {
        let data_dir = tempfile::tempdir().unwrap();
        let mut server = Self::start_in(data_dir.path()).await;
        server.owned_data_dir = Some(data_dir);
        server
    }

    /// Starts a server on an existing data directory, which outlives it.
    pub async fn start_in(data_dir: &Path) -> Self {
        let server = nekowriter_app::start(Config {
            data_dir: data_dir.to_owned(),
            log_dir: data_dir.join("logs"),
            port: 0,
        })
        .await
        .expect("the server starts");
        let base = format!("http://{}", server.address());
        Self {
            server,
            client: Client::new(),
            base,
            owned_data_dir: None,
        }
    }

    /// Shuts the server down and waits until it has released the data directory.
    pub async fn stop(self) {
        self.server.shutdown().trigger();
        self.server.wait().await.expect("the server stops cleanly");
    }

    pub fn get(&self, path: &str) -> RequestBuilder {
        self.client.get(format!("{}{path}", self.base))
    }

    pub fn post(&self, path: &str, body: &Value) -> RequestBuilder {
        self.client.post(format!("{}{path}", self.base)).json(body)
    }

    /// Creates a project and returns its JSON view.
    pub async fn create_project(&self, name: &str) -> Value {
        let response = self
            .post(
                "/api/projects",
                &json!({
                    "commandId": command_id(),
                    "name": name,
                    "novelTitle": "Rain Letters",
                    "writingLanguage": "en"
                }),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        response.json().await.unwrap()
    }
}

pub fn command_id() -> String {
    Uuid::now_v7().to_string()
}
