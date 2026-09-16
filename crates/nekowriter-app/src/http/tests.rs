//! 应用层测试装置，以及需要整个应用（真实数据目录 + 真实路由）才能成立的契约。
//!
//! 从 `app/src/main.rs` 的 `#[cfg(test)] mod tests` 整块搬来（`docs/08` 的 P1.10）：测试函数名、
//! 函数体与断言一行未改，只按新模块位置重写了导入表。装置（`accumulation_app`、`spawn_app`、
//! `spawn_app_with_web_dir`）是跨模块共用的，所以是 `pub(crate)`，`gate`、`static_files`、
//! `events`、`system` 的测试模块都从这里取；`new_accumulation_space` 与两个探针引擎只被本文件
//! 的测试使用，保持私有。

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::StatusCode;
use futures_util::future::BoxFuture;
use nekowriter_agent::engine::{
    ACCUMULATION_ENGINES, AccumulatedEntry, AccumulationEngine, AccumulationOutcome,
    AccumulationRequest, EngineDescriptor, EngineError,
};
use nekowriter_store::{CredentialPatch, KnowledgeSpaceInput, NewEmbeddingSettings, Store};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::{Mutex, Semaphore, broadcast};
use uuid::Uuid;

use crate::http::router;
use crate::http::state::AppState;
use crate::lifecycle::Shutdown;
use crate::scheduler;

/// 契约测试用的积累引擎：返回两条候选，并记录被调用的次数。
struct StubAccumulationEngine;
/// 引擎调用失败时必须什么都不写。
struct FailingAccumulationEngine;

static STUB_ACCUMULATION_ENGINES: &[&dyn AccumulationEngine] = &[&StubAccumulationEngine];
static FAILING_ACCUMULATION_ENGINES: &[&dyn AccumulationEngine] = &[&FailingAccumulationEngine];
static STUB_CALLS: AtomicUsize = AtomicUsize::new(0);

impl AccumulationEngine for StubAccumulationEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            id: "stub-accumulation",
            display_name: "探针积累引擎",
        }
    }

    fn accumulate<'a>(
        &'a self,
        request: &'a AccumulationRequest,
    ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>> {
        STUB_CALLS.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(AccumulationOutcome {
                entries: vec![
                    AccumulatedEntry {
                        title: "灯塔线索".to_owned(),
                        keywords: vec!["线索".to_owned()],
                        content: format!("{}：灯塔在雨夜里反复出现。", request.clues),
                        source: Some("探针引擎".to_owned()),
                    },
                    AccumulatedEntry {
                        title: "码头线索".to_owned(),
                        keywords: Vec::new(),
                        content: "码头在第三章被烧毁。".to_owned(),
                        source: None,
                    },
                ],
            })
        })
    }
}

impl AccumulationEngine for FailingAccumulationEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            id: "failing-accumulation",
            display_name: "总是失败的探针引擎",
        }
    }

    fn accumulate<'a>(
        &'a self,
        _request: &'a AccumulationRequest,
    ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>> {
        Box::pin(async move {
            Err(EngineError::Failed {
                message: "探针引擎无法访问上游资料".to_owned(),
            })
        })
    }
}

/// 一个测试应用：真实临时数据目录、指定注册表的应用状态。
pub(crate) async fn accumulation_app(
    engines: &'static [&'static dyn AccumulationEngine],
) -> (TempDir, AppState) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let settings_write_lock = Arc::new(Mutex::new(()));
    let model_request_permit = Arc::new(Semaphore::new(1));
    let (preview_deltas, _) = broadcast::channel(256);
    let state = AppState {
        scheduler: scheduler::WorkflowScheduler::with_preview_deltas(
            store.clone(),
            model_request_permit.clone(),
            settings_write_lock.clone(),
            preview_deltas.clone(),
        ),
        store,
        allowed_write_origins: Arc::new(HashSet::from(["http://127.0.0.1:47822".to_owned()])),
        settings_write_lock,
        model_request_permit,
        preview_deltas,
        shutdown: Shutdown::new(),
        accumulation_engines: engines,
    };
    (dir, state)
}

/// 在回环地址的真实端口上启动真实路由，返回基地址。
pub(crate) fn spawn_app(state: AppState) -> String {
    spawn_app_with_web_dir(state, &std::env::temp_dir())
}

pub(crate) fn spawn_app_with_web_dir(state: AppState, web_dir: &Path) -> String {
    let app = router(state, web_dir);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{address}")
}

async fn new_accumulation_space(state: &AppState) -> Uuid {
    state
        .store
        .create_knowledge_space(
            Uuid::new_v4(),
            KnowledgeSpaceInput {
                title: "积累".to_owned(),
            },
        )
        .await
        .unwrap()
        .id
}

/// 有注册表的引擎时，积累命令真的写入条目；重放不再次调用引擎。
#[tokio::test]
async fn accumulation_command_writes_candidates_and_replays_without_a_second_engine_call() {
    let (_dir, state) = accumulation_app(STUB_ACCUMULATION_ENGINES).await;
    let space_id = new_accumulation_space(&state).await;
    state
        .store
        .save_embedding_settings(
            Uuid::new_v4(),
            0,
            NewEmbeddingSettings {
                api_base_url: "https://embed.example/v1".to_owned(),
                model_id: "test-embed".to_owned(),
                dimension: 2,
                api_key: CredentialPatch::Keep,
            },
        )
        .await
        .unwrap();
    let base = spawn_app(state.clone());
    let client = reqwest::Client::new();
    let command_id = Uuid::new_v4();
    let body = json!({
        "commandId": command_id,
        "spaceId": space_id,
        "clues": "雨夜灯塔",
    });

    let response = client
        .post(format!("{base}/api/knowledge/accumulate"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created: Value = response.json().await.unwrap();
    assert_eq!(created.as_array().unwrap().len(), 2);
    assert_eq!(STUB_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(
        state
            .store
            .list_knowledge_entries(Some(space_id), None)
            .await
            .unwrap()
            .len(),
        2
    );
    // 每条候选都在同一事务里建立了索引任务。
    assert_eq!(
        state
            .store
            .knowledge_index_coverage(&[space_id])
            .await
            .unwrap()
            .pending_entries,
        2
    );

    // 同一条命令重试：返回原结果，不再次调用引擎，也不重复写入。
    let replay = client
        .post(format!("{base}/api/knowledge/accumulate"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let replayed: Value = replay.json().await.unwrap();
    assert_eq!(replayed, created);
    assert_eq!(STUB_CALLS.load(Ordering::SeqCst), 1, "重放不再次调用引擎");
    assert_eq!(
        state
            .store
            .list_knowledge_entries(Some(space_id), None)
            .await
            .unwrap()
            .len(),
        2
    );

    // 同一个 command_id 换一组线索是幂等冲突，不写入。
    let conflict = client
        .post(format!("{base}/api/knowledge/accumulate"))
        .json(&json!({
            "commandId": command_id,
            "spaceId": space_id,
            "clues": "另一次积累",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict_body: Value = conflict.json().await.unwrap();
    assert_eq!(conflict_body["error"]["code"], "IDEMPOTENCY_CONFLICT");
}

/// 引擎执行失败时不得写入任何条目，也不留下可重放的回执。
#[tokio::test]
async fn accumulation_engine_failure_writes_nothing() {
    let (_dir, state) = accumulation_app(FAILING_ACCUMULATION_ENGINES).await;
    let space_id = new_accumulation_space(&state).await;
    let base = spawn_app(state.clone());
    let client = reqwest::Client::new();
    let command_id = Uuid::new_v4();

    let response = client
        .post(format!("{base}/api/knowledge/accumulate"))
        .json(&json!({
            "commandId": command_id,
            "spaceId": space_id,
            "clues": "雨夜灯塔",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "ENGINE_FAILED");
    assert_eq!(
        state
            .store
            .list_knowledge_entries(Some(space_id), None)
            .await
            .unwrap(),
        [] as [nekowriter_core::KnowledgeEntrySummary; 0]
    );
    assert!(
        state
            .store
            .find_accumulation_result(command_id, space_id, "雨夜灯塔")
            .await
            .unwrap()
            .is_none()
    );
}

/// MVP 的生产注册表为空：能力查询与积累入口读同一份，都不产生写入。
#[tokio::test]
async fn accumulation_without_a_production_engine_reports_not_available_and_writes_nothing() {
    let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
    let space_id = new_accumulation_space(&state).await;
    let base = spawn_app(state.clone());
    let client = reqwest::Client::new();

    let capabilities: Value = client
        .get(format!("{base}/api/capabilities"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(capabilities["knowledgeAccumulationEngines"], json!([]));
    assert!(capabilities["knowledgeAccumulationUnavailableReason"].is_string());

    let response = client
        .post(format!("{base}/api/knowledge/accumulate"))
        .json(&json!({
            "commandId": Uuid::new_v4(),
            "spaceId": space_id,
            "clues": "雨夜灯塔",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "ENGINE_NOT_AVAILABLE");
    assert_eq!(
        state
            .store
            .list_knowledge_entries(Some(space_id), None)
            .await
            .unwrap(),
        [] as [nekowriter_core::KnowledgeEntrySummary; 0]
    );
}

/// 界面能力必须真的可达：组件文件存在、却没有任何挂载点时，界面只会在点击那一刻
/// 表现为「无反应」，既不会有编译错误，也不会让任何请求失败。
///
/// 它只能在源码层对账——手法与事件清单相同。本轮之前 `ShortcutHelpPanel` 与
/// `ServicePanel` 就是这样丢掉了挂载点。
#[test]
fn restored_ui_capabilities_are_mounted_in_the_app_shell() {
    let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../webui/src");
    for (file, marker) in [
        ("App.tsx", "<ShortcutHelpPanel"),
        ("pages/SettingsPage.tsx", "<ServicePanel"),
        ("components/AppFrame.tsx", "<ServiceStoppedNotice"),
        ("pages/WorkspacePage.tsx", "<ServiceStoppedNotice"),
    ] {
        let path = frontend.join(file);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
        assert!(
            source.contains(marker),
            "{} 不再挂载 {marker}：界面能力会静默消失",
            path.display()
        );
    }
}
