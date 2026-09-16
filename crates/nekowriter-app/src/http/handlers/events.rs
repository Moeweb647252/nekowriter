//! 事件面 handler：游标端点与全局可恢复事件流（SSE）。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.9），代码一行未改。`http::router` 要挂
//! `/events` 与 `/events/stream` 两条路由，所以两个 handler 是 `pub(crate)`；`next_sse_event`
//! 与四个 helper 仍然私有。
//!
//! 两个例外是给还没搬走的测试用的：`event_cursor` 与 `stream_reset_required` 是 `pub(crate)`，
//! 因为 `app/src/main.rs` 的 `#[cfg(test)] mod tests` 仍在调用它们（P1.10 把那些测试搬进本文件
//! 之后，这两个名字应当退回私有）。同一条注释也写在 §9 的 P1.9 记录里。
//! P1.10 已把这两个测试搬进本文件的 `#[cfg(test)] mod tests`，因此 `event_cursor` 与
//! `stream_reset_required` 已退回私有。

use std::{collections::VecDeque, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream;
use nekowriter_store::{EventStreamBounds, PersistedEvent};
use tokio::sync::broadcast;
use tracing::warn;

use crate::http::dto::{EventCursorResponse, EventsQuery};
use crate::http::error::ApiError;
use crate::http::state::{AppState, EventStreamState};
use crate::lifecycle::Shutdown;
use crate::scheduler;

/// 返回全局可恢复事件流的当前水位，供单页应用在创建 SSE 连接前取得起点。
pub(crate) async fn get_event_cursor(
    State(state): State<AppState>,
) -> Result<Json<EventCursorResponse>, ApiError> {
    let bounds = state.store.event_stream_bounds().await?;
    Ok(Json(EventCursorResponse {
        event_watermark: bounds.watermark.to_string(),
    }))
}

pub(crate) async fn stream_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Response, ApiError> {
    let cursor = event_cursor(&headers, query.after.as_deref())?;
    // 先订阅再检查游标边界。持久事件不会在“查历史到订阅实时”之间丢失；临时预览
    // 本来就不可恢复，只从连接建立后开始消费。
    let receiver = state.store.subscribe_events();
    let preview_receiver = state.preview_deltas.subscribe();
    let bounds = state.store.event_stream_bounds().await?;
    if stream_reset_required(cursor, bounds) {
        return Ok(Sse::new(stream::once(async move {
            Ok::<_, Infallible>(stream_reset_event(cursor, bounds))
        }))
        .into_response());
    }

    let stream_state = EventStreamState {
        receiver,
        preview_receiver,
        shutdown: state.shutdown.subscribe(),
        store: state.store,
        cursor,
        pending: VecDeque::new(),
        send_connection_comment: true,
    };
    let stream = stream::unfold(stream_state, next_sse_event);
    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response())
}

async fn next_sse_event(
    mut state: EventStreamState,
) -> Option<(Result<Event, Infallible>, EventStreamState)> {
    loop {
        // 退出服务时先结束持久连接，否则优雅退出会一直等 SSE 客户端断开。
        if Shutdown::requested(&state.shutdown) {
            return None;
        }
        if state.send_connection_comment {
            state.send_connection_comment = false;
            return Some((Ok(Event::default().comment("connected")), state));
        }
        if let Some(event) = state.pending.pop_front() {
            state.cursor = event
                .seq
                .parse()
                .expect("持久事件序列号始终由 SQLite 整数转换而来");
            return Some((Ok(to_sse_event(event)), state));
        }

        match state.store.events_after(state.cursor).await {
            Ok(events) if !events.is_empty() => {
                state.pending = events.into();
            }
            Ok(_) => {
                let mut shutdown = state.shutdown.clone();
                tokio::select! {
                    result = state.receiver.recv() => match result {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    result = state.preview_receiver.recv() => match result {
                        Ok(delta) => return Some((Ok(to_preview_delta_sse_event(&delta)), state)),
                        // 预览是明确可丢弃的临时层；落后时直接等待下一段或持久状态。
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    _ = shutdown.changed() => return None,
                }
            }
            Err(error) => {
                warn!(cursor = state.cursor, %error, "读取 SSE 持久事件失败，关闭当前连接");
                return None;
            }
        }
    }
}

fn to_sse_event(event: PersistedEvent) -> Event {
    let data =
        serde_json::to_string(&event).expect("持久事件的 JSON 值和 UUID 始终可以序列化为 SSE 数据");
    Event::default()
        .id(event.seq)
        .event(event.event_type)
        .data(data)
}

/// 临时预览故意不设置 SSE `id`。浏览器可能仍在 `lastEventId` 上保留上一条持久
/// 事件的值，前端因此只按 `request/tool/offset` 处理这类事件。
fn to_preview_delta_sse_event(delta: &scheduler::PreviewDelta) -> Event {
    let data = serde_json::to_string(&delta).expect("临时预览只包含可序列化的 UUID、标量与字符串");
    Event::default().event("preview.delta").data(data)
}

fn event_cursor(headers: &HeaderMap, query_after: Option<&str>) -> Result<i64, ApiError> {
    let header_cursor = headers
        .get("last-event-id")
        .map(|value| value.to_str().map_err(|_| ApiError::InvalidEventCursor))
        .transpose()?;
    let cursor_text = header_cursor
        .filter(|cursor| !cursor.is_empty())
        .or(query_after)
        .unwrap_or("0");
    if cursor_text.is_empty() || !cursor_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::InvalidEventCursor);
    }
    let cursor = cursor_text
        .parse::<i64>()
        .map_err(|_| ApiError::InvalidEventCursor)?;
    Ok(cursor)
}

fn stream_reset_required(cursor: i64, bounds: EventStreamBounds) -> bool {
    cursor < bounds.minimum_cursor || cursor > bounds.watermark
}

fn stream_reset_event(cursor: i64, bounds: EventStreamBounds) -> Event {
    let reason = if cursor > bounds.watermark {
        "cursor_ahead"
    } else {
        "cursor_expired"
    };
    Event::default().event("stream.reset_required").data(
        serde_json::json!({
            "reason": reason,
            "watermark": bounds.watermark.to_string(),
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use futures_util::StreamExt;
    use nekowriter_agent::engine::ACCUMULATION_ENGINES;
    use nekowriter_core::NewProject;
    use serde_json::Value;
    use uuid::Uuid;

    use crate::http::tests::{accumulation_app, spawn_app};

    use super::*;

    #[test]
    fn event_cursor_prefers_last_event_id_and_rejects_invalid_values() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "42".parse().unwrap());
        assert_eq!(event_cursor(&headers, Some("7")).unwrap(), 42);
        assert_eq!(event_cursor(&HeaderMap::new(), Some("7")).unwrap(), 7);

        assert!(matches!(
            event_cursor(&HeaderMap::new(), Some("+7")),
            Err(ApiError::InvalidEventCursor)
        ));
        assert!(matches!(
            event_cursor(&HeaderMap::new(), Some("")),
            Err(ApiError::InvalidEventCursor)
        ));

        headers.insert("last-event-id", "-1".parse().unwrap());
        assert!(matches!(
            event_cursor(&headers, None),
            Err(ApiError::InvalidEventCursor)
        ));
    }

    #[tokio::test]
    async fn event_cursor_endpoint_reports_the_retained_global_watermark() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        state
            .store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("事件游标", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let expected = state.store.event_stream_bounds().await.unwrap().watermark;
        let base = spawn_app(state);
        let response: Value = reqwest::Client::new()
            .get(format!("{base}/api/events/cursor"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(response["eventWatermark"], expected.to_string());
    }

    #[test]
    fn event_stream_requires_a_snapshot_reset_outside_its_retained_window() {
        let bounds = EventStreamBounds {
            minimum_cursor: 12,
            watermark: 21,
        };
        assert!(stream_reset_required(11, bounds));
        assert!(!stream_reset_required(12, bounds));
        assert!(!stream_reset_required(21, bounds));
        assert!(stream_reset_required(22, bounds));
    }

    #[tokio::test]
    async fn preview_delta_shares_sse_without_a_persistent_id() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let version_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let base = spawn_app(state.clone());
        let response = reqwest::Client::new()
            .get(format!("{base}/api/events?after=0"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.bytes_stream();
        // 先消费连接注释，确保服务端已经订阅临时广播，再发出预览片段。
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("SSE 应先发送连接注释")
            .expect("SSE 不应在连接后立即关闭")
            .expect("SSE 连接注释不应写入错误");
        let _ = state.preview_deltas.send(scheduler::PreviewDelta {
            version_id,
            request_id,
            tool: Some(scheduler::PreviewTool {
                index: 0,
                id: Some("preview-tool-1".to_owned()),
                name: Some("append_content".to_owned()),
            }),
            offset: 0,
            content: "{\"content\":\"半截".to_owned(),
        });

        let mut wire = String::from_utf8(first.to_vec()).unwrap();
        for _ in 0..4 {
            if wire.contains("event: preview.delta") {
                break;
            }
            let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("预览片段应在同一 SSE 连接中到达")
                .expect("SSE 不应在预览前关闭")
                .expect("SSE 预览不应写入错误");
            wire.push_str(core::str::from_utf8(&next).unwrap());
        }
        let preview_block = wire
            .split("\n\n")
            .find(|block| block.contains("event: preview.delta"))
            .expect("应该收到 preview.delta");
        assert!(preview_block.contains(&format!("\"requestId\":\"{request_id}\"")));
        assert!(preview_block.contains("\"offset\":\"0\""));
        assert!(preview_block.contains("\"append_content\""));
        assert!(
            !preview_block.lines().any(|line| line.starts_with("id:")),
            "临时预览不能携带可恢复的 SSE id：{preview_block}"
        );
        assert!(state.store.events_after(0).await.unwrap().is_empty());
    }
}
