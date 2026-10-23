"""Local test shim in front of the opencode.ai "go" inference gateway.

Two things the gateway does not provide on its own:

* it requires an ``x-opencode-session`` header that NekoWriter cannot send;
* it streams a non-standard ``reasoning_content`` delta that only inflates the
  answer (and trips NekoWriter's 2 MiB answer limit), so it is stripped here.

Everything else, including SSE streaming and tool calls, is forwarded unchanged.
"""
import http.client
import http.server
import json
import sys
import time
import uuid

UPSTREAM = "opencode.ai"
PREFIX = "/inference/go"
SESSION = str(uuid.uuid4())
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 47830

HOP_BY_HOP = {
    "host",
    "connection",
    "accept-encoding",
    "content-length",
    "transfer-encoding",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
}


def strip_reasoning(payload: bytes, stats: dict) -> bytes:
    """Drop reasoning_content from a JSON chat payload; leave anything else alone."""
    try:
        data = json.loads(payload)
    except ValueError:
        return payload
    changed = False
    for choice in data.get("choices", []) or []:
        for holder in ("delta", "message"):
            block = choice.get(holder)
            if not isinstance(block, dict):
                continue
            if "reasoning_content" in block:
                stats["reasoning"] += len(block.get("reasoning_content") or "")
                del block["reasoning_content"]
                changed = True
            stats["content"] += len(block.get("content") or "")
            for call in block.get("tool_calls") or []:
                args = (call.get("function") or {}).get("arguments") or ""
                stats["tool_args"] += len(args)
    return json.dumps(data).encode() if changed else payload


def filter_sse(chunk: bytes, stats: dict) -> bytes:
    out = []
    for line in chunk.split(b"\n"):
        if line.startswith(b"data: ") and line != b"data: [DONE]":
            out.append(b"data: " + strip_reasoning(line[6:], stats))
        else:
            out.append(line)
    return b"\n".join(out)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("[shim] %s\n" % (fmt % args))
        sys.stderr.flush()

    def _forward(self):
        started = time.monotonic()
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else None
        model = "?"
        shape = ""
        if body:
            try:
                req = json.loads(body)
                model = req.get("model", "?")
                shape = "req_bytes=%d stream=%s tools=%d max_tokens=%s messages=%d" % (
                    len(body),
                    req.get("stream"),
                    len(req.get("tools") or []),
                    req.get("max_tokens"),
                    len(req.get("messages") or []),
                )
            except ValueError:
                pass
        headers = {
            k: v for k, v in self.headers.items() if k.lower() not in HOP_BY_HOP
        }
        headers["x-opencode-session"] = SESSION
        if body is not None:
            headers["Content-Length"] = str(len(body))
        path = PREFIX + self.path
        try:
            conn = http.client.HTTPSConnection(UPSTREAM, timeout=900)
            conn.request(self.command, path, body=body, headers=headers)
            resp = conn.getresponse()
        except Exception as exc:  # noqa: BLE001
            payload = ('{"error":{"message":"upstream failed: %s"}}' % exc).encode()
            self.send_response(502)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        stats = {"content": 0, "reasoning": 0, "tool_args": 0}
        self.send_response(resp.status)
        ctype = resp.getheader("Content-Type")
        if ctype:
            self.send_header("Content-Type", ctype)
        streaming = "text/event-stream" in (ctype or "").lower()
        if streaming:
            self.send_header("Cache-Control", "no-cache")
        declared = resp.getheader("Content-Length")
        sent = 0
        if declared is not None:
            payload = resp.read()
            if self.command != "HEAD":
                payload = strip_reasoning(payload, stats)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            sent = len(payload)
        else:
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            pending = b""
            try:
                while True:
                    raw = resp.read(4096)
                    if not raw:
                        break
                    if streaming:
                        # Keep partial lines back so JSON is never cut in half.
                        pending += raw
                        cut = pending.rfind(b"\n")
                        if cut == -1:
                            continue
                        out, pending = filter_sse(pending[: cut + 1], stats), pending[cut + 1 :]
                    else:
                        out = raw
                    if not out:
                        continue
                    self.wfile.write(b"%x\r\n" % len(out) + out + b"\r\n")
                    self.wfile.flush()
                    sent += len(out)
                if pending:
                    self.wfile.write(b"%x\r\n" % len(pending) + pending + b"\r\n")
                    sent += len(pending)
                self.wfile.write(b"0\r\n\r\n")
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
        conn.close()
        sys.stderr.write(
            "[shim] %s %s model=%s %s status=%d stream=%s ctype=%s bytes=%d content=%d reasoning=%d tool_args=%d %.1fs\n"
            % (
                self.command,
                self.path,
                model,
                shape,
                resp.status,
                streaming,
                ctype,
                sent,
                stats["content"],
                stats["reasoning"],
                stats["tool_args"],
                time.monotonic() - started,
            )
        )
        sys.stderr.flush()

    do_GET = _forward
    do_POST = _forward
    do_PUT = _forward
    do_DELETE = _forward
    do_PATCH = _forward


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


if __name__ == "__main__":
    sys.stderr.write(
        "[shim] listening on 127.0.0.1:%d -> %s%s (reasoning stripped)\n"
        % (PORT, UPSTREAM, PREFIX)
    )
    sys.stderr.flush()
    Server(("127.0.0.1", PORT), Handler).serve_forever()
