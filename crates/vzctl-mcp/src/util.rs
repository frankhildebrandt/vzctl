//! Shared helpers for MCP tool handlers.

use crate::api::ApiClient;
use serde_json::Value;

pub fn json_pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

pub fn api_json(client: &ApiClient, path: &str) -> Result<String, String> {
    Ok(json_pretty(&client.get_json(path)?))
}

pub fn api_post_json(client: &ApiClient, path: &str, body: Option<&Value>) -> Result<String, String> {
    Ok(json_pretty(&client.post_json(path, body)?))
}
