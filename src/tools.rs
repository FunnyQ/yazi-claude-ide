use serde::Serialize;
use serde_json::{Map, Value};

pub trait ToolContext {
    fn focused_file(&self) -> Option<String>;
    fn workspace_folders(&self) -> Vec<String>;
    fn reveal(&self, _file_path: &str);
}

pub struct AdvertisedTool {
    pub name: &'static str,
    pub description: &'static str,
}

/// The four tools of F1, in the order `tools/list` advertises them.
pub const ADVERTISED: [AdvertisedTool; 4] = [
    AdvertisedTool {
        name: "getCurrentSelection",
        description: "placeholder",
    },
    AdvertisedTool {
        name: "getLatestSelection",
        description: "placeholder",
    },
    AdvertisedTool {
        name: "getWorkspaceFolders",
        description: "placeholder",
    },
    AdvertisedTool {
        name: "getOpenEditors",
        description: "placeholder",
    },
];

/// `ADVERTISED` rendered as the JSON array `tools/list` returns.
pub fn advertised_json() -> Value {
    todo!()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub start: Position,
    pub end: Position,
    pub is_empty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SelectionPayload {
    #[serde(rename_all = "camelCase")]
    Success {
        success: bool,
        file_path: String,
        text: String,
        selection: Selection,
    },
    Failure {
        success: bool,
        message: String,
    },
}

/// Every tool result is a JSON string inside a single text block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
}

pub fn exists(_file_path: &str) -> bool {
    todo!()
}

pub fn selection_payload(_file_path: Option<&str>) -> SelectionPayload {
    todo!()
}

pub fn call_tool(
    _name: &str,
    _args: &Map<String, Value>,
    _ctx: &dyn ToolContext,
) -> Option<ToolResult> {
    todo!()
}
