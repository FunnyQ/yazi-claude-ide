#!/usr/bin/env bun
// The sidecar the yazi plugin launches. Owns the lock file, the WebSocket
// server, and this instance's DDS stream — the three halves nothing else joins.
// Usage: sidecar.ts        (YAZI_ID must be inherited from yazi)
import {
  anchorFor,
  lockDir,
  reclaimStale,
  removeLock,
  updateFolders,
  workspaceFolders,
  writeLock,
} from "./lock.ts";
import { startSidecar } from "./server.ts";
import { reveal, subscribe, watchLiveness } from "./yazi.ts";

const yaziId = process.env.YAZI_ID;
if (!yaziId) {
  // Without it, `ya sub` would deliver every yazi on the machine and `ya emit-to`
  // would have nowhere to send. Only a plugin-launched child inherits it (G2).
  console.error(
    "YAZI_ID is unset — the sidecar must be launched by the plugin",
  );
  process.exit(1);
}

// Provisional. yazi's own cwd is where the user ran it, which is not necessarily
// the directory it opened — `yazi ~/work/foo` from home is the common case. The
// first `cd` event carries the real one, and it always arrives at startup
// (measured), so this pair lives for milliseconds.
let anchor = anchorFor(process.cwd());
let anchored = false;
let folders = workspaceFolders(anchor, process.cwd());

const dir = lockDir();
reclaimStale(dir);

const sidecar = startSidecar({
  workspaceFolders: () => folders,
  reveal: (filePath) => reveal(yaziId, filePath),
});

writeLock(dir, sidecar.port, {
  pid: process.pid,
  workspaceFolders: folders,
  ideName: "yazi",
  transport: "ws",
  authToken: sidecar.authToken,
});

const stream = subscribe(yaziId, {
  onHover: (url) => sidecar.setFocus(url),
  onMarked: (urls) => {
    // The only place the gesture is observable without a Claude attached: with
    // no connection open it sends nothing (H8), so the log is what the manual
    // harness asserts against.
    console.error(`yazi-claude-ide: marked ${urls.length} file(s)`);
    sidecar.mention(urls);
  },
  onCd: (url) => {
    // The directory yazi opened is the first one it announces — that is the
    // startup directory the anchor is defined against (B1). After that the
    // anchor never moves and only the cursor entry follows yazi (B3).
    if (!anchored) {
      anchor = anchorFor(url);
      anchored = true;
    }
    folders = workspaceFolders(anchor, url);
    updateFolders(dir, sidecar.port, folders);
  },
});

// G3. The double fork that keeps yazi from killing this process on startup also
// keeps it from killing it on exit, and DDS announces no departure. So ask.
const liveness = watchLiveness(yaziId, () => {
  console.error("yazi-claude-ide: yazi is gone, exiting");
  shutdown();
});

console.error(
  `yazi-claude-ide: ws://${sidecar.hostname}:${sidecar.port} yazi=${yaziId} anchor=${anchor}`,
);

function shutdown(): never {
  liveness.stop();
  stream.stop();
  sidecar.stop();
  removeLock(dir, sidecar.port);
  process.exit(0);
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
