//! Postman v2.1 collection structs. Unknown fields are preserved for round-trip safety.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Collection {
    pub info: Info,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable: Vec<Variable>,
    #[serde(default)]
    pub item: Vec<Item>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Variable {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A tree node: a folder if `item` is set, a request if `request` is set.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Item>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event: Vec<Event>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.item.is_some()
    }
    pub fn is_request(&self) -> bool {
        self.request.is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub header: Vec<Header>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    pub url: Url,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Url {
    Raw(String),
    Parsed {
        raw: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

impl Url {
    pub fn raw(&self) -> &str {
        match self {
            Url::Raw(s) => s,
            Url::Parsed { raw, .. } => raw,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Header {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Body {
    /// Postman body mode ("raw" | "graphql" | "formdata" | ...). Optional: real
    /// extension exports frequently emit an empty `"body": {}` with no mode;
    /// default "" round-trips back to `{}` and `build_body` sends no body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// Can be a JSON string (typical) or a JSON object (some editors serialize the parsed body).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql: Option<Graphql>,
    /// Multipart form-data fields (Postman body.mode = "formdata").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formdata: Vec<FormField>,
}

/// A multipart/form-data field. `kind` is "text" (use `value`) or "file" (use `src`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormField {
    pub key: String,
    #[serde(rename = "type", default = "default_field_type")]
    pub kind: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

fn default_field_type() -> String {
    "text".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Graphql {
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub listen: String, // "prerequest" | "test"
    pub script: Script,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Script {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub script_type: Option<String>,
    #[serde(default)]
    pub exec: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_script_type_variable_type_and_absent_graphql_vars() {
        let json = r#"{
          "info": { "name": "RT", "_postman_id": "abc", "schema": "s" },
          "variable": [{ "key": "k", "value": "v", "type": "string" }],
          "item": [
            { "name": "r", "request": {
                "method": "POST",
                "url": { "raw": "https://x/g", "host": ["x"], "path": ["g"] },
                "header": [{ "key": "H", "value": "1", "type": "text" }],
                "body": { "mode": "graphql", "graphql": { "query": "{a}" } }
              },
              "event": [{ "listen": "test", "script": { "exec": ["x"], "type": "text/javascript" } }],
              "response": []
            }
          ]
        }"#;
        let v_in: serde_json::Value = serde_json::from_str(json).unwrap();
        let c: Collection = serde_json::from_str(json).unwrap();
        let out = serde_json::to_string(&c).unwrap();
        let v_out: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v_in, v_out,
            "re-serialized JSON must be value-equal to input"
        );
    }

    #[test]
    fn parses_minimal_collection_with_folder_and_request() {
        let json = r#"{
          "info": { "name": "Sample" },
          "variable": [{ "key": "base", "value": "https://api.test" }],
          "item": [
            { "name": "auth", "item": [
              { "name": "login", "request": {
                  "method": "POST",
                  "url": "{{base}}/login",
                  "header": [{ "key": "Content-Type", "value": "application/json" }],
                  "body": { "mode": "raw", "raw": "{\"u\":\"x\"}" }
              }}
            ]}
          ]
        }"#;
        let c: Collection = serde_json::from_str(json).unwrap();
        assert_eq!(c.info.name, "Sample");
        assert_eq!(c.variable[0].key, "base");
        let folder = &c.item[0];
        assert_eq!(folder.name, "auth");
        assert!(folder.is_folder());
        let req_item = &folder.item.as_ref().unwrap()[0];
        assert!(req_item.is_request());
        let req = req_item.request.as_ref().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url.raw(), "{{base}}/login");
        assert_eq!(req.header[0].key, "Content-Type");
    }

    #[test]
    fn url_can_be_object() {
        let json = r#"{"method":"GET","url":{"raw":"https://x/y","host":["x"],"path":["y"]}}"#;
        let r: Request = serde_json::from_str(json).unwrap();
        assert_eq!(r.url.raw(), "https://x/y");
    }

    #[test]
    fn parses_repo_sample_collection() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../collections/sample-collection.json"
        );
        let raw = std::fs::read_to_string(path).expect("sample collection present");
        let c: Collection = serde_json::from_str(&raw).expect("parses");
        assert!(!c.item.is_empty());
        // round-trip serializes without panicking
        let _ = serde_json::to_string(&c).unwrap();
    }

    #[test]
    fn parses_empty_body_object_and_round_trips_to_empty() {
        // Real extension exports frequently emit `"body": {}` with no `mode`.
        // The model must accept it (mode defaults to "") and round-trip back to `{}`.
        let req: Request =
            serde_json::from_str(r#"{"method":"GET","url":"https://x/y","body":{}}"#)
                .expect("body without mode must parse");
        let body = req.body.expect("body present");
        assert_eq!(body.mode, "");
        let out = serde_json::to_value(&body).unwrap();
        assert_eq!(
            out,
            serde_json::json!({}),
            "empty body must re-serialize as {{}}"
        );
    }
}
