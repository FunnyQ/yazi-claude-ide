use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yazi_claude_ide::lock::{self, LockFile};
use yazi_claude_ide::server::{DiffLaunch, StartOptions, start_sidecar};
use yazi_claude_ide::yazi::{self, LivenessOptions, StreamHandlers};

struct Folders {
    anchor: String,
    anchored: bool,
    folders: Vec<String>,
}

/// J2. The copy and the script are the user's file in all but name.
fn write_private(path: &Path, contents: &str) -> Option<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    file.write_all(contents.as_bytes()).ok()
}

/// J1-J3. Put the proposed contents where the viewer can open them, write the
/// script yazi will run, and ask yazi for the terminal. `None` at every failure,
/// which J7 turns back into `-32601`.
fn launch_diff(template: &str, yazi_id: &str, launch: DiffLaunch<'_>) -> Option<PathBuf> {
    // The token directory itself is still created non-recursively, so a token that
    // is already on disk fails the launch instead of reusing someone else's scratch.
    // The uid is in the root because `temp_dir()` is the shared /tmp on Linux, where
    // the first user to open a diff would otherwise own the directory and leave every
    // other user's `openDiff` failing into J7. `main.lua` names the log directory the
    // same way. macOS needs neither — `TMPDIR` is already per-user there.
    let root = std::env::temp_dir()
        .join(format!("yazi-claude-ide+{}", unsafe { libc::getuid() }))
        .join("diff");
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&root)
        .ok()?;
    let dir = root.join(launch.token);
    fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;

    // Keep the user's file name: the viewer reads its syntax highlighting off it,
    // and a diff of `proposed` against `main.rs` is a worse thing to look at.
    let name = Path::new(launch.old_path)
        .file_name()
        .unwrap_or_else(|| OsStr::new("proposed"));
    let new_path = dir.join(name);
    let script_path = dir.join("view.sh");
    let launched = write_private(&new_path, launch.new_contents)
        .and_then(|()| {
            write_private(
                &script_path,
                &yazi::diff_script(
                    template,
                    yazi_id,
                    launch.token,
                    launch.old_path,
                    new_path.to_str()?,
                ),
            )
        })
        .and_then(|()| yazi::open_diff(yazi_id, script_path.to_str()?).then_some(()));

    match launched {
        Some(()) => Some(new_path),
        None => {
            let _ = fs::remove_dir_all(&dir);
            None
        }
    }
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
    // J1. Read once: an opt-in nobody set means section J never runs, and F5's
    // -32601 is what every openDiff gets.
    let diff_template = std::env::var("YCI_DIFF_CMD")
        .ok()
        .filter(|template| !template.trim().is_empty());
    let diff_yazi_id = yazi_id.clone();
    let sidecar = Arc::new(
        start_sidecar(StartOptions {
            workspace_folders: Box::new(move || folders_of(&workspace_state)),
            reveal: Box::new(move |path| yazi::reveal(&reveal_yazi_id, path)),
            open_diff: Box::new(move |launch| {
                launch_diff(diff_template.as_deref()?, &diff_yazi_id, launch)
            }),
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
            ide_name: lock::ide_name(),
            transport: "ws".to_owned(),
            auth_token: sidecar.auth_token().to_owned(),
        },
    )?;

    let hover_sidecar = Arc::clone(&sidecar);
    let marked_sidecar = Arc::clone(&sidecar);
    let editor_selection_sidecar = Arc::clone(&sidecar);
    let diff_done_sidecar = Arc::clone(&sidecar);
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
            on_editor_selection: Box::new(move |selection| {
                // I10. The range, never the text — this log lands in /tmp, and the
                // text is the contents of the user's file. It is also the only
                // observable this channel has: no yazi UI, behind a block opener,
                // and its whole failure mode is silence.
                eprintln!(
                    "yazi-claude-ide: selection {} L{}-{}",
                    selection.url, selection.line_start, selection.line_end
                );
                editor_selection_sidecar.set_editor_selection(
                    &selection.url,
                    (selection.line_start, selection.line_end),
                    (selection.char_start, selection.char_end),
                    &selection.text,
                );
            }),
            on_diff_done: Box::new(move |token| {
                // J8 again: the token names which request, and nothing about the
                // contents the user just read.
                eprintln!("yazi-claude-ide: diff closed");
                diff_done_sidecar.finish_diff(&token);
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
