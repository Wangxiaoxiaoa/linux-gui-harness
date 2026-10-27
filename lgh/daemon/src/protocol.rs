use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(crate = "serde")]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
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
