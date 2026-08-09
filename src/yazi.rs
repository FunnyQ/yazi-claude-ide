use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Map;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

/// The kind the plugin publishes the marked set under (H2, H3).
pub const MARKED_KIND: &str = "claude-marked";

/// The kind the editor publishes a line range under (I2).
pub const SELECTION_KIND: &str = "claude-selection";

/// `claude-marked` is ours because yazi has no event for marked-set changes (H1);
/// `claude-selection` is ours because the editor is not a yazi event source at all.
pub const KINDS: &str = "hover,cd,claude-marked,claude-selection";

/// Each liveness probe costs about 7ms.
pub const POLL_MS: u64 = 2_000;

/// A probe failure means DDS could not route to the id, which is not quite the
/// same as yazi being gone. Measured: when the instance acting as DDS server
/// exits, every surviving peer is unroutable for ~1.6s. One failure would act on
/// that window ~80% of the time and two need it to reach only 2s, so three in a
/// row is the evidence — a 4s outage, 2.5x the worst measured.
pub const FAILURES_BEFORE_GONE: u32 = 3;

/// A DDS line is `kind,receiver,sender,body`. The receiver segment is skipped
/// rather than kept: nothing downstream reads it, and hover fires on every cursor
/// keystroke, so parsing it would allocate a `String` per keypress for no reader.
#[derive(Debug, Clone, PartialEq)]
pub struct DdsEvent {
    pub kind: String,
    pub sender: String,
    pub body: Map<String, Value>,
}

pub type UrlFn = Box<dyn Fn(&str) + Send>;
pub type MarkedFn = Box<dyn Fn(Vec<String>) + Send>;
/// `(url, line_start, line_end)`, still 1-based as the editor published it (I4).
pub type RangeFn = Box<dyn Fn(&str, u32, u32) + Send>;

pub struct StreamHandlers {
    pub on_hover: UrlFn,
    pub on_cd: UrlFn,
    pub on_marked: MarkedFn,
    pub on_range: RangeFn,
}

pub struct Subscription {
    task: Option<tokio::task::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}

impl Subscription {
    fn inert() -> Self {
        Self {
            task: None,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Idempotent — stopping twice is not an error.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub fn parse_event(line: &str) -> Option<DdsEvent> {
    let first = line.find(',')?;
    if first == 0 {
        return None;
    }
    let second = line[first + 1..].find(',')? + first + 1;
    let third = line[second + 1..].find(',')? + second + 1;
    let value: Value = serde_json::from_str(&line[third + 1..]).ok()?;
    let body = value.as_object()?.clone();

    Some(DdsEvent {
        kind: line[..first].to_string(),
        sender: line[second + 1..third].to_string(),
        body,
    })
}

/// The line the editor published, or `None` for anything I6 says to drop whole.
fn range_of(body: &Map<String, Value>) -> Option<(&str, u32, u32)> {
    let line = |key| {
        u32::try_from(body.get(key).and_then(Value::as_u64)?)
            .ok()
            .filter(|line| *line >= 1)
    };
    let url = body
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())?;
    let (start, end) = (line("lineStart")?, line("lineEnd")?);
    (start <= end).then_some((url, start, end))
}

pub fn dispatch(line: &str, yazi_id: &str, handlers: &StreamHandlers) {
    let Some(event) = parse_event(line) else {
        return;
    };

    // Before the sender check, not after: `ya pub-to` publishes under an id of its
    // own, so a range never carries the sender G2 filters on. `yaziId` in the body
    // is what ties it to this instance, and I2's broadcast means every other
    // sidecar on the machine is reading this same line (I3).
    if event.kind == SELECTION_KIND {
        let claimed = match event.body.get("yaziId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            _ => return,
        };
        if claimed != yazi_id {
            return;
        }
        if let Some((url, start, end)) = range_of(&event.body) {
            (handlers.on_range)(url, start, end);
        }
        return;
    }

    if event.sender != yazi_id {
        return;
    }

    if event.kind == MARKED_KIND {
        let Some(urls) = event.body.get("urls").and_then(Value::as_array) else {
            return;
        };
        let urls = urls
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        (handlers.on_marked)(urls);
        return;
    }

    let Some(url) = event.body.get("url").and_then(Value::as_str) else {
        return;
    };
    if url.is_empty() {
        return;
    }

    match event.kind.as_str() {
        "hover" => (handlers.on_hover)(url),
        "cd" => (handlers.on_cd)(url),
        _ => {}
    }
}

/// The spawner `subscribe` uses. Overridable so a test can force a failure.
pub type Spawner = Box<dyn Fn() -> std::io::Result<tokio::process::Child> + Send>;

pub fn subscribe(yazi_id: &str, handlers: StreamHandlers) -> Subscription {
    subscribe_with(
        yazi_id,
        handlers,
        Box::new(|| {
            tokio::process::Command::new("ya")
                .args(["sub", KINDS])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
        }),
    )
}

pub fn subscribe_with(yazi_id: &str, handlers: StreamHandlers, spawn: Spawner) -> Subscription {
    let mut child = match spawn() {
        Ok(child) => child,
        Err(error) => {
            // Missing `ya` is normal on machines without yazi; the sidecar must
            // keep serving its other lifecycle responsibilities.
            eprintln!("failed to subscribe to yazi DDS events: {error}");
            return Subscription::inert();
        }
    };
    let Some(stdout) = child.stdout.take() else {
        // An injected spawner may omit the pipe; without stdout there is no
        // stream to read, but the sidecar still must not panic.
        eprintln!("failed to subscribe to yazi DDS events: stdout was not piped");
        return Subscription::inert();
    };

    let stopped = Arc::new(AtomicBool::new(false));
    let task_stopped = Arc::clone(&stopped);
    let yazi_id = yazi_id.to_string();
    let task = tokio::spawn(async move {
        // Language-forced difference: TypeScript hand-rolls a pending buffer for
        // split chunks. Rust's `BufReader::lines()` owns that concern, with no
        // observable change to the pure `parse_event` and `dispatch` functions.
        let mut lines = BufReader::new(stdout).lines();
        while !task_stopped.load(Ordering::Acquire) {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if task_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    dispatch(&line, &yazi_id, &handlers);
                }
                Ok(None) => break,
                Err(error) => {
                    // A broken DDS stream is terminal for this subscription, but
                    // must not bring down the sidecar.
                    eprintln!("failed to read yazi DDS event: {error}");
                    break;
                }
            }
        }
        drop(child);
    });

    Subscription {
        task: Some(task),
        stopped,
    }
}

/// The argv `reveal` spawns, split out so a test can assert on it.
pub fn reveal_args(yazi_id: &str, file_path: &str) -> Vec<String> {
    vec![
        "emit-to".to_string(),
        yazi_id.to_string(),
        "reveal".to_string(),
        file_path.to_string(),
    ]
}

pub fn reveal(yazi_id: &str, file_path: &str) {
    // Gotcha: `tokio::process::Command::spawn` needs a reactor, so this must be
    // called inside the tokio runtime context. The handler always runs in a task,
    // but a future caller from a plain thread would panic.
    match tokio::process::Command::new("ya")
        .args(reveal_args(yazi_id, file_path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            // Tokio reaps the orphan in the background.
            drop(child);
        }
        Err(error) => {
            // Log it and move on—don't panic. The F3 answer is sent before the
            // spawn is known to have succeeded.
            eprintln!("failed to reveal path in yazi: {error}");
        }
    }
}

pub async fn probe_alive(yazi_id: &str) -> bool {
    match tokio::process::Command::new("ya")
        .args(["emit-to", yazi_id, "noop"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) => status.success(),
        // Missing `ya` is indistinguishable from a receiver DDS cannot route to;
        // liveness owns the consecutive-failure tolerance.
        Err(error) => {
            eprintln!("failed to probe yazi liveness: {error}");
            false
        }
    }
}

pub struct LivenessOptions {
    pub interval_ms: u64,
    pub failures_before_gone: u32,
}

pub fn watch_liveness<P, Fut>(
    yazi_id: &str,
    opts: LivenessOptions,
    probe: P,
    on_gone: impl FnOnce() + Send + 'static,
) -> Subscription
where
    P: Fn(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = bool> + Send + 'static,
{
    let stopped = Arc::new(AtomicBool::new(false));
    let task_stopped = Arc::clone(&stopped);
    let yazi_id = yazi_id.to_string();
    let task = tokio::spawn(async move {
        let mut failures = 0;
        let mut on_gone = Some(on_gone);

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(opts.interval_ms)).await;
            if task_stopped.load(Ordering::Acquire) {
                break;
            }

            let alive = probe(yazi_id.clone()).await;
            if task_stopped.load(Ordering::Acquire) {
                break;
            }

            if alive {
                failures = 0;
                continue;
            }

            failures += 1;
            if failures >= opts.failures_before_gone {
                // `on_gone` is a local `Option` in the one task that can reach it,
                // so `take` is the whole at-most-once guarantee.
                if let Some(on_gone) = on_gone.take() {
                    on_gone();
                }
                break;
            }
        }
    });

    Subscription {
        task: Some(task),
        stopped,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;

    fn calls() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn handlers(
        hover: Arc<Mutex<Vec<String>>>,
        cd: Arc<Mutex<Vec<String>>>,
        marked: Arc<Mutex<Vec<Vec<String>>>>,
    ) -> StreamHandlers {
        StreamHandlers {
            on_hover: Box::new(move |url| hover.lock().unwrap().push(url.to_string())),
            on_cd: Box::new(move |url| cd.lock().unwrap().push(url.to_string())),
            on_marked: Box::new(move |urls| marked.lock().unwrap().push(urls)),
            on_range: Box::new(|_, _, _| {}),
        }
    }

    fn marks() -> Arc<Mutex<Vec<Vec<String>>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn empty_handlers() -> StreamHandlers {
        handlers(calls(), calls(), marks())
    }

    /// Handlers wired so only the hover sink is observable.
    fn hover_probe() -> (Arc<Mutex<Vec<String>>>, StreamHandlers) {
        let hover = calls();
        let handlers = handlers(Arc::clone(&hover), calls(), marks());
        (hover, handlers)
    }

    /// Handlers wired so only the marked sink is observable.
    fn marked_probe() -> (Arc<Mutex<Vec<Vec<String>>>>, StreamHandlers) {
        let marked = marks();
        let handlers = handlers(calls(), calls(), Arc::clone(&marked));
        (marked, handlers)
    }

    /// Handlers wired so only the range sink is observable.
    fn range_probe() -> (Ranges, StreamHandlers) {
        let ranges: Ranges = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&ranges);
        let handlers = StreamHandlers {
            on_range: Box::new(move |url, start, end| {
                sink.lock().unwrap().push((url.to_string(), start, end))
            }),
            ..handlers(calls(), calls(), marks())
        };
        (ranges, handlers)
    }

    type Ranges = Arc<Mutex<Vec<(String, u32, u32)>>>;

    /// A `claude-selection` line as `ya pub-to 0` really writes it: `sender` is
    /// the publishing `ya`'s own id, never the yazi the editor belongs to (I3).
    fn range_line(yazi_id: &str, body: &str) -> String {
        format!("claude-selection,0,some-other-ya,{{\"yaziId\":{yazi_id},{body}}}")
    }

    #[test]
    fn h3_kinds_contains_marked_kind() {
        assert!(KINDS.split(',').any(|kind| kind == MARKED_KIND));
    }

    #[test]
    fn i2_kinds_contains_selection_kind() {
        assert!(KINDS.split(',').any(|kind| kind == SELECTION_KIND));
    }

    #[test]
    fn i3_a_range_is_matched_on_yazi_id_not_sender() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"ours\"",
                r#""url":"/tmp/one.txt","lineStart":10,"lineEnd":20"#,
            ),
            "ours",
            &handlers,
        );
        assert_eq!(
            *ranges.lock().unwrap(),
            [("/tmp/one.txt".to_owned(), 10, 20)]
        );
    }

    #[test]
    fn i3_a_range_for_another_yazi_is_ignored() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"theirs\"",
                r#""url":"/tmp/one.txt","lineStart":10,"lineEnd":20"#,
            ),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i3_a_numeric_yazi_id_matches_too() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line("175", r#""url":"/tmp/one.txt","lineStart":1,"lineEnd":1"#),
            "175",
            &handlers,
        );
        assert_eq!(ranges.lock().unwrap().len(), 1);
    }

    #[test]
    fn i3_a_range_without_a_yazi_id_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            r#"claude-selection,0,ours,{"url":"/tmp/one.txt","lineStart":10,"lineEnd":20}"#,
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_an_empty_url_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line("\"ours\"", r#""url":"","lineStart":10,"lineEnd":20"#),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_a_missing_line_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line("\"ours\"", r#""url":"/tmp/one.txt","lineStart":10"#),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_a_non_numeric_line_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"ours\"",
                r#""url":"/tmp/one.txt","lineStart":"10","lineEnd":20"#,
            ),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_line_zero_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"ours\"",
                r#""url":"/tmp/one.txt","lineStart":0,"lineEnd":20"#,
            ),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_a_reversed_range_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"ours\"",
                r#""url":"/tmp/one.txt","lineStart":20,"lineEnd":10"#,
            ),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn i6_a_line_beyond_u32_is_dropped() {
        let (ranges, handlers) = range_probe();
        dispatch(
            &range_line(
                "\"ours\"",
                r#""url":"/tmp/one.txt","lineStart":1,"lineEnd":4294967296"#,
            ),
            "ours",
            &handlers,
        );
        assert!(ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_parse_event_hover() {
        let event = parse_event(r#"hover,0,175,{"tab":0,"url":"/tmp/one.txt"}"#).unwrap();
        assert_eq!(event.kind, "hover");
        assert_eq!(event.sender, "175");
        assert_eq!(event.body["url"], "/tmp/one.txt");
    }

    #[test]
    fn g2_parse_event_cd_with_commas_in_path() {
        let event = parse_event(r#"cd,0,175,{"tab":0,"url":"/tmp/a,b/c,d"}"#).unwrap();
        assert_eq!(event.body["url"], "/tmp/a,b/c,d");
    }

    #[test]
    fn g2_parse_event_too_few_commas() {
        assert_eq!(parse_event("hover,0,175"), None);
    }

    #[test]
    fn g2_parse_event_invalid_json() {
        assert_eq!(parse_event("hover,0,175,{ not json"), None);
    }

    #[test]
    fn g2_parse_event_non_event_line() {
        assert_eq!(
            parse_event("Connected to existing DDS server on instance 175"),
            None
        );
    }

    #[test]
    fn g2_parse_event_empty_line() {
        assert_eq!(parse_event(""), None);
    }

    #[test]
    fn g2_parse_event_non_object_body() {
        assert_eq!(parse_event("hover,0,175,[]"), None);
    }

    #[test]
    fn g2_hover_from_our_instance() {
        let (hover, handlers) = hover_probe();
        dispatch(r#"hover,0,ours,{"url":"/tmp/one.txt"}"#, "ours", &handlers);
        assert_eq!(*hover.lock().unwrap(), ["/tmp/one.txt"]);
    }

    #[test]
    fn g2_hover_from_other_instance() {
        let (hover, handlers) = hover_probe();
        dispatch(
            r#"hover,0,theirs,{"url":"/tmp/one.txt"}"#,
            "ours",
            &handlers,
        );
        assert!(hover.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_cd_reaches_on_cd() {
        let cd = calls();
        let handlers = handlers(calls(), Arc::clone(&cd), Arc::new(Mutex::new(Vec::new())));
        dispatch(r#"cd,0,ours,{"url":"/tmp/project"}"#, "ours", &handlers);
        assert_eq!(*cd.lock().unwrap(), ["/tmp/project"]);
    }

    #[test]
    fn g2_absent_url_dropped() {
        let (hover, handlers) = hover_probe();
        dispatch(r#"hover,0,ours,{"tab":0}"#, "ours", &handlers);
        assert!(hover.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_empty_url_dropped() {
        let (hover, handlers) = hover_probe();
        dispatch(r#"hover,0,ours,{"url":""}"#, "ours", &handlers);
        assert!(hover.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_null_url_dropped() {
        let (hover, handlers) = hover_probe();
        dispatch(r#"hover,0,ours,{"url":null}"#, "ours", &handlers);
        assert!(hover.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_dispatch_unknown_kind_ignored() {
        let (hover, handlers) = hover_probe();
        dispatch(r#"rename,0,ours,{"url":"/tmp/one"}"#, "ours", &handlers);
        assert!(hover.lock().unwrap().is_empty());
    }

    #[test]
    fn g2_dispatch_malformed_line_dropped() {
        dispatch("not an event", "ours", &empty_handlers());
    }

    #[test]
    fn h3_marked_event_reaches_on_marked() {
        let (marked, handlers) = marked_probe();
        dispatch(
            r#"claude-marked,0,ours,{"urls":["/tmp/one","/tmp/two"]}"#,
            "ours",
            &handlers,
        );
        assert_eq!(*marked.lock().unwrap(), [["/tmp/one", "/tmp/two"]]);
    }

    #[test]
    fn h3_marked_from_other_instance() {
        let (marked, handlers) = marked_probe();
        dispatch(
            r#"claude-marked,0,theirs,{"urls":["/tmp/one"]}"#,
            "ours",
            &handlers,
        );
        assert!(marked.lock().unwrap().is_empty());
    }

    #[test]
    fn h7_empty_marked_set_delivered() {
        let (marked, handlers) = marked_probe();
        dispatch(r#"claude-marked,0,ours,{"urls":[]}"#, "ours", &handlers);
        assert_eq!(*marked.lock().unwrap(), [Vec::<String>::new()]);
    }

    #[test]
    fn h3_marked_without_urls_dropped() {
        let (marked, handlers) = marked_probe();
        dispatch(
            r#"claude-marked,0,ours,{"url":"/tmp/one"}"#,
            "ours",
            &handlers,
        );
        assert!(marked.lock().unwrap().is_empty());
    }

    #[test]
    fn h3_marked_with_non_string_entry_filtered() {
        let (marked, handlers) = marked_probe();
        dispatch(
            r#"claude-marked,0,ours,{"urls":["/tmp/one.txt",null,42]}"#,
            "ours",
            &handlers,
        );
        assert_eq!(*marked.lock().unwrap(), [["/tmp/one.txt"]]);
    }

    #[test]
    fn h3_marked_branch_before_url_check() {
        let (marked, handlers) = marked_probe();
        dispatch(
            r#"claude-marked,0,ours,{"urls":["/tmp/one"]}"#,
            "ours",
            &handlers,
        );
        assert_eq!(marked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn g2_subscribe_spawn_failure_yields_inert_subscription() {
        let subscription = subscribe_with(
            "ours",
            empty_handlers(),
            Box::new(|| Err(io::Error::new(io::ErrorKind::NotFound, "no ya"))),
        );
        subscription.stop();
        subscription.stop();
        tokio::task::yield_now().await;
    }

    #[test]
    fn f3_reveal_args_builds_correct_argv() {
        assert_eq!(
            reveal_args("175", "/tmp/path with spaces/file.txt"),
            ["emit-to", "175", "reveal", "/tmp/path with spaces/file.txt"]
        );
    }

    fn scripted_probe(
        script: Arc<Mutex<VecDeque<bool>>>,
        ids: Arc<Mutex<Vec<String>>>,
    ) -> impl Fn(String) -> std::future::Ready<bool> + Send + 'static {
        move |id| {
            ids.lock().unwrap().push(id);
            let mut script = script.lock().unwrap();
            let answer = if script.len() > 1 {
                script.pop_front().unwrap()
            } else {
                *script.front().unwrap()
            };
            std::future::ready(answer)
        }
    }

    fn liveness(
        answers: impl IntoIterator<Item = bool>,
    ) -> (Subscription, Arc<Mutex<Vec<String>>>, oneshot::Receiver<()>) {
        let script = Arc::new(Mutex::new(answers.into_iter().collect()));
        let ids = calls();
        let (gone_tx, gone_rx) = oneshot::channel();
        let subscription = watch_liveness(
            "ours",
            LivenessOptions {
                interval_ms: 3,
                failures_before_gone: 3,
            },
            scripted_probe(script, Arc::clone(&ids)),
            move || {
                // The receiver may intentionally be dropped by a negative test.
                let _ = gone_tx.send(());
            },
        );
        (subscription, ids, gone_rx)
    }

    #[tokio::test]
    async fn g3_consecutive_failures_end_the_sidecar() {
        let (_subscription, ids, gone) = liveness([false]);
        tokio::time::timeout(Duration::from_millis(120), gone)
            .await
            .expect("on_gone timed out")
            .expect("on_gone sender dropped");
        assert_eq!(*ids.lock().unwrap(), ["ours", "ours", "ours"]);
    }

    #[tokio::test]
    async fn g3_lone_failure_does_not_end() {
        let (subscription, ids, gone) = liveness([false, true]);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), gone)
                .await
                .is_err()
        );
        subscription.stop();
        assert!(ids.lock().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn g3_success_resets_count() {
        let (_subscription, ids, gone) = liveness([false, true, false, false, false]);
        tokio::time::timeout(Duration::from_millis(120), gone)
            .await
            .expect("on_gone timed out")
            .expect("on_gone sender dropped");
        assert_eq!(ids.lock().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn g3_live_yazi_never_declared_gone() {
        let (subscription, ids, gone) = liveness([true]);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), gone)
                .await
                .is_err()
        );
        subscription.stop();
        assert!(!ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn g3_stop_ends_poll() {
        let (subscription, ids, _gone) = liveness([false]);
        subscription.stop();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(ids.lock().unwrap().is_empty());
    }
}
