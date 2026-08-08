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
