//! Shared helpers for the adapter tests: a raw TCP server that sends one SSE chunk and then
//! stalls, so the idle-timeout path can be exercised without waiting 300 seconds.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Starts a server that answers the first request with `headers` and `body`, then stays silent.
///
/// The body has no `content-length`, so the response is read until the connection closes; the
/// open connection is exactly the "provider went quiet" situation the idle timeout covers.
pub async fn stalling_sse_server(body: &'static str) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buffer = vec![0u8; 8192];
        let _ = socket.read(&mut buffer).await;
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
        socket.write_all(head.as_bytes()).await.expect("head");
        socket.write_all(body.as_bytes()).await.expect("body");
        socket.flush().await.expect("flush");
        // Stay silent far longer than any test's timeout.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    });
    addr
}
