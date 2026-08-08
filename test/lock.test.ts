// Contract clauses A1-A7 and B1-B5. Each test names the clause it covers.
import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

import {
  anchorFor,
  lockDir,
  newAuthToken,
  readLock,
  reclaimStale,
  removeLock,
  updateFolders,
  workspaceFolders,
  writeLock,
} from "../src/lock.ts";

const REPO = path.resolve(import.meta.dir, "..");

function tmpDir(): string {
  return fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-test-"));
}

function lock(dir: string, folders: string[]) {
  return writeLock(dir, 41234, {
    pid: process.pid,
    workspaceFolders: folders,
    ideName: "yazi",
    transport: "ws",
    authToken: newAuthToken(),
  });
}

describe("A. discovery and the lock file", () => {
  test("A1 the lock file lives at <config>/ide/<port>.lock", () => {
    const home = tmpDir();
    expect(lockDir({ HOME: home })).toBe(path.join(home, ".claude", "ide"));
    expect(lockDir({ CLAUDE_CONFIG_DIR: "/somewhere/cfg" })).toBe(
      "/somewhere/cfg/ide",
    );
    // The real environment must resolve too, whichever of the two is set.
    expect(path.isAbsolute(lockDir())).toBe(true);
    expect(lockDir()).toEndWith(path.join("ide"));
  });

  test("A1 the file name is the bound port", () => {
    const dir = tmpDir();
    expect(path.basename(lock(dir, [REPO]))).toBe("41234.lock");
  });

  test("A2 directory mode is 0700 and file mode is 0600", () => {
    const dir = path.join(tmpDir(), "ide");
    const file = lock(dir, [REPO]);
    expect(fs.statSync(dir).mode & 0o777).toBe(0o700);
    expect(fs.statSync(file).mode & 0o777).toBe(0o600);
  });

  test("A3 the lock file carries exactly the five fields", () => {
    const dir = tmpDir();
    const written = JSON.parse(fs.readFileSync(lock(dir, [REPO]), "utf8"));
    expect(Object.keys(written).sort()).toEqual([
      "authToken",
      "ideName",
      "pid",
      "transport",
      "workspaceFolders",
    ]);
    expect(written.ideName).toBe("yazi");
    expect(written.transport).toBe("ws");
  });

  test("A4 authToken is 32 lowercase hex characters and differs per call", () => {
    const a = newAuthToken();
    expect(a).toMatch(/^[0-9a-f]{32}$/);
    expect(a).not.toBe(newAuthToken());
  });

  test("A6 removeLock deletes the file and tolerates a missing one", () => {
    const dir = tmpDir();
    const file = lock(dir, [REPO]);
    removeLock(dir, 41234);
    expect(fs.existsSync(file)).toBe(false);
    expect(() => removeLock(dir, 41234)).not.toThrow();
  });

  test("A7 startup reclaims dead-pid locks and spares live ones", () => {
    const dir = tmpDir();
    writeLock(dir, 1111, {
      pid: 999999,
      workspaceFolders: [REPO],
      ideName: "yazi",
      transport: "ws",
      authToken: newAuthToken(),
    });
    lock(dir, [REPO]); // port 41234, this process's own live pid
    fs.writeFileSync(path.join(dir, "not-a-lock.txt"), "ignore me");

    const removed = reclaimStale(dir, (pid) => pid === process.pid);

    expect(removed).toEqual([path.join(dir, "1111.lock")]);
    expect(fs.existsSync(path.join(dir, "41234.lock"))).toBe(true);
    expect(fs.existsSync(path.join(dir, "not-a-lock.txt"))).toBe(true);
  });

  test("A7 an unparseable lock file is reclaimed rather than crashing startup", () => {
    const dir = tmpDir();
    fs.mkdirSync(dir, { recursive: true });
    const junk = path.join(dir, "2222.lock");
    fs.writeFileSync(junk, "{ not json");
    expect(reclaimStale(dir, () => true)).toEqual([junk]);
  });
});

describe("B. workspace folders", () => {
  test("B1 the pair is anchor then cursor", () => {
    expect(workspaceFolders(REPO, path.join(REPO, "spike"))).toEqual([
      REPO,
      path.join(REPO, "spike"),
    ]);
  });

  test("B2 an anchor equal to the cursor collapses to one entry", () => {
    expect(workspaceFolders(REPO, REPO)).toEqual([REPO]);
  });

  test("B3 updateFolders republishes the pair the caller computed", () => {
    const dir = tmpDir();
    lock(dir, workspaceFolders(REPO, REPO));

    updateFolders(dir, 41234, workspaceFolders(REPO, path.join(REPO, "spike")));
    expect(readLock(dir, 41234)?.workspaceFolders).toEqual([
      REPO,
      path.join(REPO, "spike"),
    ]);

    updateFolders(dir, 41234, workspaceFolders(REPO, "/tmp"));
    expect(readLock(dir, 41234)?.workspaceFolders).toEqual([REPO, "/tmp"]);

    // Back to the anchor: the pair collapses again rather than duplicating.
    updateFolders(dir, 41234, workspaceFolders(REPO, REPO));
    expect(readLock(dir, 41234)?.workspaceFolders).toEqual([REPO]);
  });

  test("B3 updateFolders on a lock file that is gone is a no-op", () => {
    const dir = tmpDir();
    expect(() => updateFolders(dir, 41234, [REPO])).not.toThrow();
    expect(readLock(dir, 41234)).toBeNull();
  });

  test("B4 the rewrite preserves pid and authToken, and the file mode", () => {
    const dir = tmpDir();
    const file = lock(dir, workspaceFolders(REPO, REPO));
    const before = readLock(dir, 41234)!;

    updateFolders(dir, 41234, workspaceFolders(REPO, "/tmp"));

    const after = readLock(dir, 41234)!;
    expect(after.pid).toBe(before.pid);
    expect(after.authToken).toBe(before.authToken);
    expect(after.ideName).toBe("yazi");
    expect(after.transport).toBe("ws");
    expect(fs.statSync(file).mode & 0o777).toBe(0o600);
  });

  test("B5 paths are absolutised and stripped of trailing slashes", () => {
    expect(workspaceFolders(`${REPO}/`, `${REPO}/spike/`)).toEqual([
      REPO,
      path.join(REPO, "spike"),
    ]);
    expect(workspaceFolders(".", "spike")).toEqual([
      process.cwd(),
      path.join(process.cwd(), "spike"),
    ]);
    expect(workspaceFolders("/", "/")).toEqual(["/"]);
  });

  test("B5 symlinks are advertised as given, not resolved", () => {
    const dir = tmpDir();
    const real = path.join(dir, "real");
    const link = path.join(dir, "link");
    fs.mkdirSync(real);
    fs.symlinkSync(real, link);
    expect(workspaceFolders(link, link)).toEqual([link]);
  });

  test("B1 the anchor is the git root, or the directory itself outside a repo", () => {
    expect(anchorFor(path.join(REPO, "plugin", "claude-ide.yazi"))).toBe(REPO);
    const outside = tmpDir();
    expect(anchorFor(outside)).toBe(outside);
  });

  test("B1 a homedir-relative anchor stays absolute", () => {
    expect(path.isAbsolute(anchorFor(homedir()))).toBe(true);
  });
});
