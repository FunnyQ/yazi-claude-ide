// yazi's side of the wire: the DDS event stream in, and `reveal` back out.
import { spawn } from "node:child_process";

export type DdsEvent = {
  kind: string;
  receiver: string;
  sender: string;
  body: Record<string, unknown>;
};

export type StreamHandlers = {
  onHover: (url: string) => void;
  onCd: (url: string) => void;
};

/** hover carries the focused file, cd the new directory. Nothing else is needed. */
export const KINDS = "hover,cd";

/**
 * `ya sub` prints `kind,receiver,sender,json`. Only the first three commas
 * separate — a path may contain the rest.
 */
export function parseEvent(line: string): DdsEvent | null {
  const first = line.indexOf(",");
  const second = line.indexOf(",", first + 1);
  const third = line.indexOf(",", second + 1);
  if (first < 1 || second < 0 || third < 0) return null;

  let body: unknown;
  try {
    body = JSON.parse(line.slice(third + 1));
  } catch {
    return null;
  }
  if (typeof body !== "object" || body === null || Array.isArray(body))
    return null;

  return {
    kind: line.slice(0, first),
    receiver: line.slice(first + 1, second),
    sender: line.slice(second + 1, third),
    body: body as Record<string, unknown>,
  };
}

export function dispatch(
  line: string,
  yaziId: string,
  handlers: StreamHandlers,
): void {
  const event = parseEvent(line);
  // `ya sub` is global: every yazi on the machine lands in this stream, and only
  // the sender field tells them apart (G2).
  if (!event || event.sender !== yaziId) return;

  // Measured: hover repeats an unchanged url, and the first events after startup
  // carry an empty one. Both have to be tolerated here rather than downstream.
  const url = event.body.url;
  if (typeof url !== "string" || url === "") return;

  if (event.kind === "hover") handlers.onHover(url);
  else if (event.kind === "cd") handlers.onCd(url);
}

export type Subscription = { stop(): void };

/** Follow this instance's hover and cd events until stopped. */
export function subscribe(
  yaziId: string,
  handlers: StreamHandlers,
): Subscription {
  const child = spawn("ya", ["sub", KINDS], {
    stdio: ["ignore", "pipe", "ignore"],
  });

  let pending = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    pending += chunk;
    const lines = pending.split("\n");
    pending = lines.pop() ?? ""; // a chunk can split a line in half
    for (const line of lines) dispatch(line, yaziId, handlers);
  });

  return {
    stop() {
      child.kill();
    },
  };
}

/** F3. The one out-of-scope tool yazi can honestly perform. */
export function reveal(yaziId: string, filePath: string): void {
  spawn("ya", ["emit-to", yaziId, "reveal", filePath], {
    stdio: "ignore",
  }).unref();
}

/** How often to ask whether yazi is still there. Each probe costs ~7ms. */
export const POLL_MS = 2_000;

/**
 * A probe failure means DDS could not route to the id, which is not quite the
 * same as yazi being gone. Measured: when the instance acting as DDS server
 * exits, every surviving peer is unroutable for ~1.6s. One failure would act on
 * that window ~80% of the time and two need it to reach only 2s, so three in a
 * row is the evidence — a 4s outage, 2.5x the worst measured.
 */
export const FAILURES_BEFORE_GONE = 3;

/**
 * G3. `ya emit-to` exits 0 for a live receiver and 1 for an unknown one, so the
 * poll is one exit code. `noop` is a real yazi command that changes nothing.
 */
export function probeAlive(yaziId: string): Promise<boolean> {
  return new Promise((resolve) => {
    const child = spawn("ya", ["emit-to", yaziId, "noop"], { stdio: "ignore" });
    child.on("close", (code) => resolve(code === 0));
    // `ya` off PATH is unroutable too, and leaving it unhandled would throw.
    child.on("error", () => resolve(false));
  });
}

export type LivenessOptions = {
  intervalMs?: number;
  failuresBeforeGone?: number;
  probe?: (yaziId: string) => Promise<boolean>;
};

/** G3. Call `onGone` once yazi stops answering. DDS announces no departure. */
export function watchLiveness(
  yaziId: string,
  onGone: () => void,
  options: LivenessOptions = {},
): Subscription {
  const intervalMs = options.intervalMs ?? POLL_MS;
  const limit = options.failuresBeforeGone ?? FAILURES_BEFORE_GONE;
  const probe = options.probe ?? probeAlive;

  let failures = 0;
  let stopped = false;
  // A tick chains the next one rather than running on an interval, so a slow
  // probe delays the poll instead of stacking up behind it.
  let timer = setTimeout(tick, intervalMs);

  async function tick(): Promise<void> {
    failures = (await probe(yaziId)) ? 0 : failures + 1;
    if (stopped) return;
    if (failures >= limit) return onGone();
    timer = setTimeout(tick, intervalMs);
  }

  return {
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
