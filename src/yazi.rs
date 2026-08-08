use serde_json::Map;
use serde_json::Value;

pub const MARKED_KIND: &str = "claude-marked";
pub const KINDS: &str = "hover,cd,claude-marked";
pub const POLL_MS: u64 = 2_000;
pub const FAILURES_BEFORE_GONE: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct DdsEvent {
    pub kind: String,
    pub receiver: String,
    pub sender: String,
    pub body: Map<String, Value>,
}

pub type UrlFn = Box<dyn Fn(&str) + Send>;
pub type MarkedFn = Box<dyn Fn(Vec<String>) + Send>;

pub struct StreamHandlers {
    pub on_hover: UrlFn,
    pub on_cd: UrlFn,
    pub on_marked: MarkedFn,
}

pub struct Subscription {
    _private: (),
}

impl Subscription {
    /// Idempotent — stopping twice is not an error.
    pub fn stop(&self) {
        todo!()
    }
}

pub fn parse_event(_line: &str) -> Option<DdsEvent> {
    todo!()
}

pub fn dispatch(_line: &str, _yazi_id: &str, _handlers: &StreamHandlers) {
    todo!()
}

/// The spawner `subscribe` uses. Overridable so a test can force a failure.
pub type Spawner = Box<dyn Fn() -> std::io::Result<tokio::process::Child> + Send>;

pub fn subscribe(_yazi_id: &str, _handlers: StreamHandlers) -> Subscription {
    todo!()
}

pub fn subscribe_with(_yazi_id: &str, _handlers: StreamHandlers, _spawn: Spawner) -> Subscription {
    todo!()
}

/// The argv `reveal` spawns, split out so a test can assert on it.
pub fn reveal_args(_yazi_id: &str, _file_path: &str) -> Vec<String> {
    todo!()
}

pub fn reveal(_yazi_id: &str, _file_path: &str) {
    todo!()
}

pub async fn probe_alive(_yazi_id: &str) -> bool {
    todo!()
}

pub struct LivenessOptions {
    pub interval_ms: u64,
    pub failures_before_gone: u32,
}

impl Default for LivenessOptions {
    fn default() -> Self {
        Self {
            interval_ms: POLL_MS,
            failures_before_gone: FAILURES_BEFORE_GONE,
        }
    }
}

pub fn watch_liveness<P, Fut>(
    _yazi_id: &str,
    _opts: LivenessOptions,
    _probe: P,
    _on_gone: impl FnOnce() + Send + 'static,
) -> Subscription
where
    P: Fn(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = bool> + Send + 'static,
{
    todo!()
}

/// The production wiring: the real probe, the measured interval and threshold.
pub fn watch_liveness_default(
    _yazi_id: &str,
    _on_gone: impl FnOnce() + Send + 'static,
) -> Subscription {
    todo!()
}
