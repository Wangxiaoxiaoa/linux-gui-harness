use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Session identity attached by the MCP proxy to every request.
///
/// `thread_id` is the `_meta.threadId` (or `sessionId`) the host sent with
/// the call; hosts without such metadata fall back to the proxy's own PID,
/// which is unique per session because each session spawns its own proxy.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(crate = "serde")]
pub struct SessionInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub proxy_pid: u32,
}

impl SessionInfo {
    /// The stable key this session is tracked by: the host-provided thread
    /// id when present, otherwise the proxy PID.
    pub fn key(&self) -> String {
        self.thread_id
            .clone()
            .unwrap_or_else(|| format!("pid:{}", self.proxy_pid))
    }
}

#[derive(Debug, Deserialize)]
#[serde(crate = "serde")]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// Injected by the proxy. Absent when an un-upgraded proxy (or a
    /// direct socket client) talks to the daemon; such requests run with
    /// peer-connection identity only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionInfo>,
}

impl Request {
    /// Parse one daemon-side line. Accepts both shapes:
    /// - enveloped: `{"session": {...}, "inner": {jsonrpc request}}`
    ///   (sent by the session-aware proxy)
    /// - bare: the raw JSON-RPC request (older proxies / direct clients)
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        let mut value: Value = serde_json::from_str(line)?;
        if value.get("inner").is_some() {
            let session = value
                .get("session")
                .cloned()
                .and_then(|s| SessionInfo::deserialize(s).ok());
            let inner = value.get_mut("inner").expect("checked above");
            let mut req = Request::deserialize(inner.take())?;
            if req.session.is_none() {
                req.session = session;
            }
            Ok(req)
        } else {
            Request::deserialize(value)
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(crate = "serde")]
pub struct Response {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetail>,
}

#[derive(Debug, Serialize)]
#[serde(crate = "serde")]
pub struct ErrorDetail {
    pub code: i32,
    pub message: String,
}

impl Response {
    pub fn result(id: Option<Value>, value: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(value),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(ErrorDetail {
                code,
                message: message.into(),
            }),
        }
    }
}
