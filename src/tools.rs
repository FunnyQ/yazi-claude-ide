use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;

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
        description: "Get the file yazi's cursor is on",
    },
    AdvertisedTool {
        name: "getLatestSelection",
        description: "Get the most recent file yazi's cursor was on",
    },
    AdvertisedTool {
        name: "getWorkspaceFolders",
        description: "Get the workspace folders yazi is browsing",
    },
    AdvertisedTool {
        name: "getOpenEditors",
        description: "Get the open editor tabs; yazi has none",
    },
];

/// `ADVERTISED` rendered as the JSON array `tools/list` returns.
pub fn advertised_json() -> Value {
    Value::Array(
        ADVERTISED
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": { "type": "object", "properties": {} },
                })
            })
            .collect(),
    )
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

pub fn exists(file_path: &str) -> bool {
    // Weaker than the regular-file test on purpose: a mention may name a directory,
    // and the CLI lists it. Every stat failure still means "not worth mentioning".
    fs::metadata(file_path).is_ok()
}

pub fn is_file(file_path: &str) -> bool {
    // Use stat rather than inspecting the link itself: a symlink to a regular file is a file (B5).
    // The error catch matters because ENOTDIR, ELOOP, and EACCES must not kill the
    // sidecar; C5 calls all of them "no active editor" (B5, C5, E5).
    fs::metadata(file_path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

pub fn selection_payload(file_path: Option<&str>) -> SelectionPayload {
    // The stat catches a focused directory or a path that vanished after focus was
    // set. Following symlinks lets the target decide whether it is a regular file,
    // while filePath remains the unresolved path yazi reported (B5, C3-C5).
    match file_path.filter(|path| is_file(path)) {
        Some(file_path) => SelectionPayload::Success {
            success: true,
            file_path: file_path.to_owned(),
            text: String::new(),
            selection: Selection {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
                is_empty: true,
            },
        },
        None => SelectionPayload::Failure {
            success: false,
            message: "No active editor found".to_owned(),
        },
    }
}

fn as_text(value: &Value) -> ToolResult {
    as_plain(&value.to_string())
}

fn as_plain(text: &str) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock {
            kind: "text".to_owned(),
            text: text.to_owned(),
        }],
    }
}

/// Reproduces the TypeScript's `${args.filePath}` template interpolation.
///
/// `Display for Value` is not that: it emits JSON, so a string would arrive at the
/// CLI quoted and an absent key as `null`. JS prints a string bare and a missing
/// property as `undefined`, which is what the F2 message must say. Composite values
/// stay JSON — JS would coerce them to `[object Object]`, and nothing on this path
/// sends one.
fn interpolate(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "undefined".to_owned(),
    }
}

pub fn call_tool(
    name: &str,
    args: &Map<String, Value>,
    ctx: &dyn ToolContext,
) -> Option<ToolResult> {
    match name {
        // C1: one payload builder, so both tools cannot drift apart.
        "getCurrentSelection" | "getLatestSelection" => Some(as_text(&json!(selection_payload(
            ctx.focused_file().as_deref()
        )))),
        "getWorkspaceFolders" => {
            let workspace_folders = ctx.workspace_folders();
            let folders: Vec<_> = workspace_folders
                .iter()
                .map(|path| {
                    json!({
                        "name": Path::new(path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(path),
                        "uri": format!("file://{path}"),
                        "path": path,
                    })
                })
                .collect();

            // The anchor is always first (B1, B6). Language-forced difference: with an
            // empty list the TypeScript's `folders[0]` is `undefined`, which
            // `JSON.stringify` drops the key for, while `Option::None` serialises as
            // `null`. Rust has no third state for "absent", and B1 guarantees the
            // anchor, so the list is never empty in practice.
            Some(as_text(&json!({
                "success": true,
                "folders": folders,
                "rootPath": workspace_folders.first(),
            })))
        }
        // Yazi has no editor tabs (C6).
        "getOpenEditors" => Some(as_text(&json!({ "tabs": [] }))),
        "closeAllDiffTabs" => Some(as_plain("CLOSED_0_DIFF_TABS")),
        "close_tab" => Some(as_plain("TAB_CLOSED")),
        "getDiagnostics" => Some(as_text(&json!([]))),
        // F2. Unadvertised, still called. Each answer is the honest one for a file
        // manager: nothing was open, so nothing was dirty, saved, or diagnosed.
        "checkDocumentDirty" | "saveDocument" => Some(as_text(&json!({
            "success": false,
            "message": format!("Document not open: {}", interpolate(args.get("filePath"))),
        }))),
        "openFile" => Some(match args.get("filePath").and_then(Value::as_str) {
            Some(path) => {
                ctx.reveal(path);
                as_plain(&format!("Opened file: {path}"))
            }
            None => as_text(&json!({
                "success": false,
                "message": "openFile requires filePath",
            })),
        }),
        // openDiff falls through to None deliberately. The CLI calls it before every
        // edit that needs confirming — in the four-tool list, not only when advertised —
        // and reads DIFF_REJECTED as the user rejecting the change, so the edit is
        // silently cancelled. DIFF_ACCEPTED is worse: it asserts an approval for a diff
        // the user was never shown. -32601 says what is true, that this IDE has no diff
        // view, and is measured to keep the CLI's own confirmation prompt, so the user
        // still holds the veto.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use tempfile::{NamedTempFile, TempDir};

    #[derive(Default)]
    struct MockContext {
        focused: Option<String>,
        folders: Vec<String>,
        reveals: RefCell<Vec<String>>,
    }

    impl ToolContext for MockContext {
        fn focused_file(&self) -> Option<String> {
            self.focused.clone()
        }

        fn workspace_folders(&self) -> Vec<String> {
            self.folders.clone()
        }

        fn reveal(&self, file_path: &str) {
            self.reveals.borrow_mut().push(file_path.to_owned());
        }
    }

    fn empty_args() -> Map<String, Value> {
        Map::new()
    }

    fn result_json(result: ToolResult) -> Value {
        serde_json::from_str(&result.content[0].text).unwrap()
    }

    fn call(name: &str, ctx: &MockContext) -> ToolResult {
        call_tool(name, &empty_args(), ctx).unwrap()
    }

    #[test]
    fn c1_both_selection_tools_return_same_payload() {
        let file = NamedTempFile::new().unwrap();
        let ctx = MockContext {
            focused: Some(file.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert_eq!(
            call("getCurrentSelection", &ctx),
            call("getLatestSelection", &ctx)
        );
    }

    #[test]
    fn c2_focused_file_yields_success_with_empty_selection() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        let value = serde_json::to_value(selection_payload(Some(&path))).unwrap();
        assert_eq!(
            value,
            json!({
                "success": true,
                "filePath": path,
                "text": "",
                "selection": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 },
                    "isEmpty": true,
                },
            })
        );
    }

    #[test]
    fn c3_file_path_unresolved_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, "data").unwrap();
        symlink(&target, &link).unwrap();
        let link = link.to_string_lossy().into_owned();
        let value = serde_json::to_value(selection_payload(Some(&link))).unwrap();
        assert_eq!(value["filePath"], link);
    }

    #[test]
    fn c4_text_field_empty_for_nonempty_file() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"secret contents").unwrap();
        let path = file.path().to_string_lossy();
        let value = serde_json::to_value(selection_payload(Some(&path))).unwrap();
        assert_eq!(value["text"], "");
    }

    #[test]
    fn c5_nothing_focused_returns_failure() {
        assert_eq!(
            serde_json::to_value(selection_payload(None)).unwrap()["success"],
            false
        );
    }

    #[test]
    fn c5_vanished_file_returns_failure() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        file.close().unwrap();
        assert_eq!(
            serde_json::to_value(selection_payload(Some(&path))).unwrap()["success"],
            false
        );
    }

    #[test]
    fn c5_metadata_fails_enotdir() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().join("child").to_string_lossy().into_owned();
        assert_eq!(
            serde_json::to_value(selection_payload(Some(&path))).unwrap()["success"],
            false
        );
    }

    #[test]
    fn c5_metadata_fails_eloop() {
        let dir = TempDir::new().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        symlink(&second, &first).unwrap();
        symlink(&first, &second).unwrap();
        let path = first.to_string_lossy().into_owned();
        assert_eq!(
            serde_json::to_value(selection_payload(Some(&path))).unwrap()["success"],
            false
        );
    }

    #[test]
    fn c5_directory_focused_returns_failure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy();
        assert_eq!(
            serde_json::to_value(selection_payload(Some(&path))).unwrap()["success"],
            false
        );
    }

    #[test]
    fn c6_get_open_editors_returns_empty_tabs() {
        assert_eq!(
            result_json(call("getOpenEditors", &MockContext::default())),
            json!({"tabs": []})
        );
    }

    #[test]
    fn b6_get_workspace_folders_with_single_folder() {
        let ctx = MockContext {
            folders: vec!["/tmp/workspace".into()],
            ..Default::default()
        };
        assert_eq!(
            result_json(call("getWorkspaceFolders", &ctx)),
            json!({
                "success": true,
                "folders": [{"name": "workspace", "uri": "file:///tmp/workspace", "path": "/tmp/workspace"}],
                "rootPath": "/tmp/workspace",
            })
        );
    }

    #[test]
    fn b6_get_workspace_folders_with_multiple_folders() {
        let ctx = MockContext {
            folders: vec!["/tmp/first".into(), "/tmp/second".into()],
            ..Default::default()
        };
        let value = result_json(call("getWorkspaceFolders", &ctx));
        assert_eq!(value["rootPath"], "/tmp/first");
        assert_eq!(value["folders"][1]["name"], "second");
    }

    #[test]
    fn b6_workspace_folders_empty_list() {
        let result = call("getWorkspaceFolders", &MockContext::default());
        let value = result_json(result.clone());
        assert_eq!(value["folders"], json!([]));
        // Indexing cannot tell a null value from an absent key, so pin the rendered
        // text: `rootPath` ships as an explicit null, the language-forced difference
        // the `getWorkspaceFolders` arm records.
        assert!(
            result.content[0].text.contains(r#""rootPath":null"#),
            "{}",
            result.content[0].text
        );
    }

    #[test]
    fn f1_exactly_four_advertised_tools_in_order() {
        assert_eq!(ADVERTISED.len(), 4);
        assert_eq!(
            ADVERTISED.map(|tool| tool.name),
            [
                "getCurrentSelection",
                "getLatestSelection",
                "getWorkspaceFolders",
                "getOpenEditors"
            ]
        );
    }

    #[test]
    fn f1_advertised_tools_have_descriptions_and_schema() {
        for tool in advertised_json().as_array().unwrap() {
            assert!(!tool["description"].as_str().unwrap().is_empty());
            assert_eq!(
                tool["inputSchema"],
                json!({"type": "object", "properties": {}})
            );
        }
    }

    #[test]
    fn f1_get_diagnostics_not_advertised() {
        assert!(!ADVERTISED.iter().any(|tool| tool.name == "getDiagnostics"));
    }

    #[test]
    fn f1_open_diff_not_advertised() {
        let name = concat!("open", "Diff");
        assert!(!ADVERTISED.iter().any(|tool| tool.name == name));
    }

    #[test]
    fn f2_unadvertised_tools_answer_verbatim() {
        let ctx = MockContext::default();
        let args = Map::from_iter([("filePath".to_owned(), json!("/tmp/doc.txt"))]);
        let text = |name: &str| {
            call_tool(name, &args, &ctx)
                .unwrap_or_else(|| panic!("{name} returned None"))
                .content[0]
                .text
                .clone()
        };

        // Raw text, not parsed JSON: a plain answer wrapped as JSON would reach the
        // CLI quoted, and that is the likeliest silent bug in this module.
        assert_eq!(text("closeAllDiffTabs"), "CLOSED_0_DIFF_TABS");
        assert_eq!(text("close_tab"), "TAB_CLOSED");
        assert_eq!(text("getDiagnostics"), "[]");

        let dirty: Value = serde_json::from_str(&text("checkDocumentDirty")).unwrap();
        assert_eq!(
            dirty,
            json!({"success": false, "message": "Document not open: /tmp/doc.txt"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&text("saveDocument")).unwrap(),
            dirty
        );
    }

    #[test]
    fn f2_document_tools_report_undefined_without_file_path() {
        let value = result_json(call("checkDocumentDirty", &MockContext::default()));
        assert_eq!(value["message"], "Document not open: undefined");
    }

    #[test]
    fn f3_open_file_reveals_and_returns_success() {
        let ctx = MockContext::default();
        let args = Map::from_iter([("filePath".to_owned(), json!("/tmp/file"))]);
        let result = call_tool("openFile", &args, &ctx).unwrap();
        assert_eq!(result.content[0].text, "Opened file: /tmp/file");
        assert_eq!(*ctx.reveals.borrow(), vec!["/tmp/file"]);
    }

    #[test]
    fn f3_open_file_without_file_path_returns_failure() {
        let value = result_json(call("openFile", &MockContext::default()));
        assert_eq!(
            value,
            json!({"success": false, "message": "openFile requires filePath"})
        );
    }

    #[test]
    fn f4_e2_unknown_tool_returns_none() {
        assert!(call_tool("unknown", &empty_args(), &MockContext::default()).is_none());
    }

    #[test]
    fn f5_open_diff_returns_none() {
        let name = concat!("open", "Diff");
        assert!(call_tool(name, &empty_args(), &MockContext::default()).is_none());
    }

    #[test]
    fn h6_exists_accepts_directory() {
        let dir = TempDir::new().unwrap();
        assert!(exists(&dir.path().to_string_lossy()));
    }

    #[test]
    fn h6_exists_rejects_vanished() {
        let dir = TempDir::new().unwrap();
        assert!(!exists(&dir.path().join("missing").to_string_lossy()));
    }

    #[test]
    fn f1_advertised_json_matches_advertised_constant() {
        let tools = advertised_json();
        for (value, advertised) in tools.as_array().unwrap().iter().zip(ADVERTISED.iter()) {
            assert_eq!(value["name"], advertised.name);
            assert_eq!(value["description"], advertised.description);
        }
    }
}
