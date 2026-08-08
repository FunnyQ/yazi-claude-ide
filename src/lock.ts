// The lock file Claude Code discovers us by. Contract clauses A and B.
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

export type LockFile = {
  pid: number;
  workspaceFolders: string[];
  ideName: "yazi";
  transport: "ws";
  authToken: string;
};

const LOCK_NAME = /^(\d+)\.lock$/;

export function newAuthToken(): string {
  return randomBytes(16).toString("hex");
}

export function lockDir(
  env: Record<string, string | undefined> = process.env,
): string {
  const config =
    env.CLAUDE_CONFIG_DIR ?? path.join(env.HOME ?? homedir(), ".claude");
  return path.join(config, "ide");
}

/** Absolute, no trailing slash, symlinks left alone (B5). */
function normalise(dir: string): string {
  return path.resolve(dir);
}

/** The anchor entry: the git root of `dir`, or `dir` itself outside a repo (B1). */
export function anchorFor(dir: string): string {
  try {
    const root = execFileSync(
      "git",
      ["-C", dir, "rev-parse", "--show-toplevel"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    if (root) return normalise(root);
  } catch {
    // Not a repository, or no git on PATH. Either way the directory stands alone.
  }
  return normalise(dir);
}

/** Anchor first, cursor second, collapsed to one entry when they coincide (B1, B2). */
export function workspaceFolders(anchor: string, cursor: string): string[] {
  const a = normalise(anchor);
  const c = normalise(cursor);
  return a === c ? [a] : [a, c];
}

function lockPath(dir: string, port: number): string {
  return path.join(dir, `${port}.lock`);
}

export function writeLock(dir: string, port: number, lock: LockFile): string {
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.chmodSync(dir, 0o700); // mkdir's mode is masked by umask, and may pre-exist
  const file = lockPath(dir, port);
  // Written via rename so the CLI can never read a half-written lock file. When
  // the CLI re-reads is unmeasured, so it has to be safe at every instant.
  const temp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temp, JSON.stringify(lock), { mode: 0o600 });
  fs.chmodSync(temp, 0o600);
  fs.renameSync(temp, file);
  return file;
}

export function readLock(dir: string, port: number): LockFile | null {
  try {
    return JSON.parse(fs.readFileSync(lockPath(dir, port), "utf8")) as LockFile;
  } catch {
    return null;
  }
}

export function removeLock(dir: string, port: number): void {
  try {
    fs.unlinkSync(lockPath(dir, port));
  } catch {
    // Already gone. Removing a lock twice is not an error.
  }
}

/** Replace the cursor entry, keep the anchor and everything else (B3, B4). */
export function rewriteCursor(dir: string, port: number, cursor: string): void {
  const lock = readLock(dir, port);
  if (!lock) return;
  writeLock(dir, port, {
    ...lock,
    workspaceFolders: workspaceFolders(lock.workspaceFolders[0]!, cursor),
  });
}

function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === "EPERM";
  }
}

/**
 * Remove lock files left behind by dead sidecars, and only those (A7).
 * An unparseable lock file is stale by definition — nothing can connect through it.
 */
export function reclaimStale(
  dir: string,
  isAlive: (pid: number) => boolean = pidAlive,
): string[] {
  let entries: string[];
  try {
    entries = fs.readdirSync(dir);
  } catch {
    return [];
  }

  const removed: string[] = [];
  for (const entry of entries) {
    const match = LOCK_NAME.exec(entry);
    if (!match) continue;
    const lock = readLock(dir, Number(match[1]));
    if (lock && typeof lock.pid === "number" && isAlive(lock.pid)) continue;
    const file = path.join(dir, entry);
    try {
      fs.unlinkSync(file);
      removed.push(file);
    } catch {
      // Someone else cleaned it up first, which is the outcome we wanted.
    }
  }
  return removed;
}
