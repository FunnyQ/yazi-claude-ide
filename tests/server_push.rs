mod common;

use common::{Client, fixture};
use serde_json::{Value, json};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;
use yazi_claude_ide::server::Sidecar;

const TOKEN: &str = "push-test-token";
const WAIT: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(150);

async fn sidecar() -> Sidecar {
    common::sidecar(TOKEN, Vec::new()).await
}

async fn connect(sidecar: &Sidecar) -> Client {
    Client::connect(sidecar, TOKEN).await.expect("connect")
}

async fn next(client: &mut Client) -> Value {
    client.next(WAIT).await.expect("expected push frame")
}

fn file_path(frame: &Value) -> &str {
    frame["params"]["filePath"].as_str().expect("filePath")
}

#[tokio::test]
async fn d3_connecting_pushes_the_current_file_once() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));

    let mut client = connect(&server).await;
    assert_eq!(file_path(&next(&mut client).await), path);
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn d3_a_second_connection_is_pushed_while_the_first_is_still_open() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));
    let mut first = connect(&server).await;
    assert_eq!(file_path(&next(&mut first).await), path);

    let mut second = connect(&server).await;
    assert_eq!(file_path(&next(&mut second).await), path);
    assert_eq!(first.silence(QUIET).await, None);
    assert_eq!(second.silence(QUIET).await, None);
}

#[tokio::test]
async fn d4_a_focus_change_after_a_second_connection_reaches_both() {
    let server = sidecar().await;
    let first_path = fixture("Cargo.toml");
    let second_path = fixture("README.md");
    server.set_focus(Some(&first_path));
    let mut first = connect(&server).await;
    let _ = next(&mut first).await;
    let mut second = connect(&server).await;
    let _ = next(&mut second).await;

    server.set_focus(Some(&second_path));
    assert_eq!(file_path(&next(&mut first).await), second_path);
    assert_eq!(file_path(&next(&mut second).await), second_path);
}

#[tokio::test]
async fn d1_the_push_is_a_notification_with_no_id() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));
    let mut client = connect(&server).await;

    let frame = next(&mut client).await;
    assert_eq!(frame["jsonrpc"], "2.0");
    assert_eq!(frame["method"], "selection_changed");
    assert!(frame.get("params").is_some());
    assert!(
        !frame
            .as_object()
            .expect("notification object")
            .contains_key("id")
    );
}

#[tokio::test]
async fn d2_the_push_carries_path_url_empty_text_and_an_empty_selection() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));
    let mut client = connect(&server).await;

    let frame = next(&mut client).await;
    assert_eq!(frame["params"]["filePath"], path);
    assert_eq!(frame["params"]["fileUrl"], format!("file://{path}"));
    assert_eq!(frame["params"]["text"], "");
    assert_eq!(
        frame["params"]["selection"],
        json!({
            "start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 0},
            "isEmpty": true,
        })
    );
}

#[tokio::test]
async fn d5_focus_landing_on_a_directory_pushes_nothing_and_the_previous_file_stands() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));
    let mut client = connect(&server).await;
    let _ = next(&mut client).await;

    server.set_focus(Some(env!("CARGO_MANIFEST_DIR")));
    assert_eq!(client.silence(QUIET).await, None);
    assert_eq!(server.focused_file(), None);

    server.set_focus(Some(&path));
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn d5_a_path_that_does_not_stat_pushes_nothing() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("gone.txt");
    std::fs::write(&path, "gone").expect("write fixture");
    std::fs::remove_file(&path).expect("remove fixture");

    server.set_focus(path.to_str());
    assert_eq!(client.silence(QUIET).await, None);
    assert_eq!(server.focused_file(), None);
}

#[tokio::test]
async fn d6_the_same_path_is_never_pushed_twice_in_a_row() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    let mut client = connect(&server).await;
    server.set_focus(Some(&path));
    assert_eq!(file_path(&next(&mut client).await), path);

    server.set_focus(Some(&path));
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn d7_a_focus_change_with_no_connection_open_is_not_queued() {
    let server = sidecar().await;
    let first = fixture("Cargo.toml");
    let second = fixture("README.md");
    server.set_focus(Some(&first));
    server.set_focus(Some(&second));

    let mut client = connect(&server).await;
    assert_eq!(file_path(&next(&mut client).await), second);
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn d7_the_next_connection_after_the_last_close_is_pushed_again() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.set_focus(Some(&path));
    let mut first = connect(&server).await;
    assert_eq!(file_path(&next(&mut first).await), path);
    first.close();
    sleep(QUIET).await;

    let mut second = connect(&server).await;
    assert_eq!(file_path(&next(&mut second).await), path);
    assert_eq!(second.silence(QUIET).await, None);
}

#[tokio::test]
async fn d8_a_focus_change_reaches_every_open_connection() {
    let server = sidecar().await;
    let mut first = connect(&server).await;
    let mut second = connect(&server).await;
    let mut third = connect(&server).await;
    let path = fixture("Cargo.toml");

    server.set_focus(Some(&path));
    let first_frame = next(&mut first).await;
    assert_eq!(next(&mut second).await, first_frame);
    assert_eq!(next(&mut third).await, first_frame);
}

#[tokio::test]
async fn h4_a_mention_is_a_notification_carrying_only_file_path() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention(std::slice::from_ref(&path));

    let frame = next(&mut client).await;
    assert_eq!(frame["jsonrpc"], "2.0");
    assert_eq!(frame["method"], "at_mentioned");
    assert!(
        !frame
            .as_object()
            .expect("notification object")
            .contains_key("id")
    );
    let params = frame["params"].as_object().expect("params object");
    assert_eq!(params.len(), 1);
    assert_eq!(params["filePath"], path);
}

#[tokio::test]
async fn h5_mentions_go_out_in_the_order_given() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let paths = vec![
        fixture("Cargo.toml"),
        fixture("README.md"),
        fixture("src/lib.rs"),
    ];
    server.mention(&paths);

    for path in paths {
        assert_eq!(file_path(&next(&mut client).await), path);
    }
}

#[tokio::test]
async fn h6_a_directory_is_mentioned_like_any_other_path() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().to_string_lossy().into_owned();
    server.mention(std::slice::from_ref(&path));

    assert_eq!(file_path(&next(&mut client).await), path);
}

#[tokio::test]
async fn h6_a_path_that_does_not_stat_is_skipped() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let gone = temp.path().join("gone.txt");
    std::fs::write(&gone, "gone").expect("write fixture");
    std::fs::remove_file(&gone).expect("remove fixture");
    let paths = vec![
        fixture("Cargo.toml"),
        gone.to_string_lossy().into_owned(),
        fixture("README.md"),
    ];

    server.mention(&paths);
    assert_eq!(file_path(&next(&mut client).await), paths[0]);
    assert_eq!(file_path(&next(&mut client).await), paths[2]);
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn h7_an_empty_set_falls_back_to_the_hovered_path_including_a_directory() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().to_string_lossy().into_owned();
    server.set_focus(Some(&path));
    assert_eq!(server.focused_file(), None);

    server.mention(&[]);
    assert_eq!(file_path(&next(&mut client).await), path);
    assert_eq!(server.focused_file(), None);
}

#[tokio::test]
async fn h7_an_empty_set_with_nothing_hovered_sends_nothing() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    server.mention(&[]);
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn h8_a_mention_with_no_connection_open_sends_nothing() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.mention(std::slice::from_ref(&path));

    let mut client = connect(&server).await;
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn h9_pressing_twice_sends_twice_with_no_dedupe() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention(std::slice::from_ref(&path));
    server.mention(std::slice::from_ref(&path));

    let first = next(&mut client).await;
    assert_eq!(next(&mut client).await, first);
}

#[tokio::test]
async fn i5_a_range_mention_carries_zero_based_lines() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention_range(&path, 10, 20);

    let frame = next(&mut client).await;
    assert_eq!(frame["jsonrpc"], "2.0");
    assert_eq!(frame["method"], "at_mentioned");
    assert!(
        !frame
            .as_object()
            .expect("notification object")
            .contains_key("id")
    );
    let params = frame["params"].as_object().expect("params object");
    assert_eq!(params.len(), 3);
    assert_eq!(params["filePath"], path);
    // I4. The editor counts from 1 and the CLI renders `0` as `#L1`, so the pair
    // that reaches the wire is one below the pair the editor published.
    assert_eq!(params["lineStart"], 9);
    assert_eq!(params["lineEnd"], 19);
}

#[tokio::test]
async fn i5_a_single_line_range_is_a_range_of_one() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention_range(&path, 1, 1);

    let frame = next(&mut client).await;
    assert_eq!(frame["params"]["lineStart"], 0);
    assert_eq!(frame["params"]["lineEnd"], 0);
}

#[tokio::test]
async fn i6_a_range_over_a_directory_is_not_mentioned() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    server.mention_range(&temp.path().to_string_lossy(), 1, 2);

    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn i6_a_range_over_a_path_that_does_not_stat_is_not_mentioned() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let gone = temp.path().join("gone.txt");
    std::fs::write(&gone, "gone").expect("write fixture");
    std::fs::remove_file(&gone).expect("remove fixture");
    server.mention_range(&gone.to_string_lossy(), 1, 2);

    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn i7_a_range_with_no_connection_open_sends_nothing() {
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    server.mention_range(&path, 10, 20);

    let mut client = connect(&server).await;
    assert_eq!(client.silence(QUIET).await, None);
}

#[tokio::test]
async fn i7_repeating_the_gesture_sends_twice_with_no_dedupe() {
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention_range(&path, 10, 20);
    server.mention_range(&path, 10, 20);

    let first = next(&mut client).await;
    assert_eq!(next(&mut client).await, first);
}

#[tokio::test]
async fn i7_a_range_reaches_every_open_connection() {
    let server = sidecar().await;
    let mut first = connect(&server).await;
    let mut second = connect(&server).await;
    let path = fixture("Cargo.toml");
    server.mention_range(&path, 10, 20);

    for client in [&mut first, &mut second] {
        let frame = next(client).await;
        assert_eq!(frame["method"], "at_mentioned");
        assert_eq!(frame["params"]["lineStart"], 9);
    }
}

#[tokio::test]
async fn i1_a_range_does_not_disturb_the_selection_push() {
    // D6 dedupes on the last pushed path. A range mention that leaked into that
    // state would silence the next real focus change onto the same file.
    let server = sidecar().await;
    let path = fixture("Cargo.toml");
    let other = fixture("README.md");
    server.set_focus(Some(&path));
    let mut client = connect(&server).await;
    assert_eq!(file_path(&next(&mut client).await), path);

    server.mention_range(&other, 3, 4);
    assert_eq!(next(&mut client).await["method"], "at_mentioned");

    server.set_focus(Some(&other));
    let frame = next(&mut client).await;
    assert_eq!(frame["method"], "selection_changed");
    assert_eq!(file_path(&frame), other);
}

#[tokio::test]
async fn h5_h9_a_marked_set_larger_than_any_queue_bound_arrives_whole_and_in_order() {
    // `mention` sends a whole marked set in one synchronous loop, so nothing on the
    // connection side gets to drain between frames. A bounded queue would drop the
    // earliest mentions here and H5 (order) plus H9 (no dedupe) would both silently
    // fail on a large selection.
    let server = sidecar().await;
    let mut client = connect(&server).await;
    let temp = TempDir::new().expect("tempdir");
    let paths: Vec<String> = (0..64)
        .map(|index| {
            let path = temp.path().join(format!("marked-{index:02}.txt"));
            std::fs::write(&path, "marked").expect("write fixture");
            path.to_string_lossy().into_owned()
        })
        .collect();

    server.mention(&paths);
    for path in &paths {
        assert_eq!(file_path(&next(&mut client).await), path);
    }
    assert_eq!(client.silence(QUIET).await, None);
}
