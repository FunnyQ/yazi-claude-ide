use std::sync::{Arc, Mutex};

use yazi_claude_ide::lock::{self, LockFile};
use yazi_claude_ide::server::{StartOptions, start_sidecar};
use yazi_claude_ide::yazi::{self, LivenessOptions, StreamHandlers};

struct Folders {
    anchor: String,
    anchored: bool,
    folders: Vec<String>,
}

fn folders_of(state: &Mutex<Folders>) -> Vec<String> {
    state
        .lock()
        .map(|state| state.folders.clone())
        .unwrap_or_default()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let yazi_id = std::env::var("YAZI_ID").unwrap_or_default();
    if yazi_id.is_empty() {
        eprintln!("YAZI_ID is unset — the sidecar must be launched by the plugin");
        std::process::exit(1);
    }
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let cwd = std::env::current_dir()?;
    // Provisional. yazi's own cwd is where the user ran it, not necessarily where
    // it opened. The first `cd` carries the real directory and arrives at startup
    // (measured), so this un-latched pair lives for milliseconds (B1).
    let anchor = lock::anchor_for(&cwd);
    let state = Arc::new(Mutex::new(Folders {
        anchor: anchor.display().to_string(),
        anchored: false,
        folders: lock::workspace_folders(&anchor, &cwd),
    }));

    let dir = lock::lock_dir();
    lock::reclaim_stale(&dir);

    // Take the guard, read or mutate, clone out what the caller needs, and drop
    // it before an await or file write.
    let workspace_state = Arc::clone(&state);
    let reveal_yazi_id = yazi_id.clone();
    let auth_token = lock::new_auth_token();
    let sidecar = Arc::new(
        start_sidecar(StartOptions {
            workspace_folders: Box::new(move || folders_of(&workspace_state)),
            reveal: Box::new(move |path| yazi::reveal(&reveal_yazi_id, path)),
            auth_token,
        })
        .await?,
    );

    lock::write_lock(
        &dir,
        sidecar.port(),
        &LockFile {
            pid: std::process::id(),
            workspace_folders: folders_of(&state),
            ide_name: "yazi".to_owned(),
            transport: "ws".to_owned(),
            auth_token: sidecar.auth_token().to_owned(),
        },
    )?;

    let hover_sidecar = Arc::clone(&sidecar);
    let marked_sidecar = Arc::clone(&sidecar);
    let cd_sidecar = Arc::clone(&sidecar);
    let cd_state = Arc::clone(&state);
    let cd_dir = dir.clone();
    let stream = yazi::subscribe(
        &yazi_id,
        StreamHandlers {
            on_hover: Box::new(move |url| hover_sidecar.set_focus(Some(url))),
            on_marked: Box::new(move |urls| {
                // This is the only observable place without Claude connected: no
                // open connection means the H8 notification itself sends nothing.
                eprintln!("yazi-claude-ide: marked {} file(s)", urls.len());
                marked_sidecar.mention(&urls);
            }),
            on_cd: Box::new(move |url| {
                let folders = {
                    let Ok(mut state) = cd_state.lock() else {
                        return;
                    };
                    if !state.anchored {
                        state.anchor = lock::anchor_for(std::path::Path::new(url))
                            .display()
                            .to_string();
                        state.anchored = true;
                    }
                    state.folders = lock::workspace_folders(
                        std::path::Path::new(&state.anchor),
                        std::path::Path::new(url),
                    );
                    state.folders.clone()
                };
                lock::update_folders(&cd_dir, cd_sidecar.port(), folders);
            }),
        },
    );

    let (gone_sender, mut gone_receiver) = tokio::sync::mpsc::channel(1);
    // These overrides exist only to make integration liveness finish in about a
    // second instead of production's measured six; keep them in this wiring.
    let liveness_options = LivenessOptions {
        interval_ms: std::env::var("YCI_POLL_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(yazi::POLL_MS),
        failures_before_gone: std::env::var("YCI_FAILURES_BEFORE_GONE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(yazi::FAILURES_BEFORE_GONE),
    };
    let liveness = yazi::watch_liveness(
        &yazi_id,
        liveness_options,
        |id| async move { yazi::probe_alive(&id).await },
        move || {
            eprintln!("yazi-claude-ide: yazi is gone, exiting");
            let _ = gone_sender.try_send(());
        },
    );

    let startup_anchor = state
        .lock()
        .map(|state| state.anchor.clone())
        .unwrap_or_default();
    eprintln!(
        "yazi-claude-ide: ws://{}:{} yazi={} anchor={}",
        sidecar.hostname(),
        sidecar.port(),
        yazi_id,
        startup_anchor
    );

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
        _ = gone_receiver.recv() => {}
    }

    liveness.stop();
    stream.stop();
    sidecar.stop();
    // A6 applies to all three paths; removal completes before main returns.
    lock::remove_lock(&dir, sidecar.port());
    Ok(())
}
