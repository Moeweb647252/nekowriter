//! A running server on a free port with a temporary data directory, and a client for it.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use nekowriter_app::{Config, RunningServer};
use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

pub struct TestServer {
    pub server: RunningServer,
    pub client: Client,
    pub base: String,
    _data_dir: TempDir,
}

impl TestServer {
    pub async fn start() -> Self {
        let data_dir = tempfile::tempdir().unwrap();
        let server = nekowriter_app::start(Config {
            data_dir: data_dir.path().to_owned(),
            log_dir: data_dir.path().join("logs"),
            port: 0,
        })
        .await
        .expect("the server starts");
        let base = format!("http://{}", server.address());
        Self {
            server,
            client: Client::new(),
            base,
            _data_dir: data_dir,
        }
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
