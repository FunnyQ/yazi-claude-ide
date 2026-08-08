// Contract clause G2, and the parsing the DDS stream needs before it can obey it.
import { describe, expect, test } from "bun:test";

import { dispatch, parseEvent } from "../src/yazi.ts";
import type { StreamHandlers } from "../src/yazi.ts";

const OURS = "1754500000000000";
const THEIRS = "1754599999999999";

function collect() {
  const hovered: string[] = [];
  const entered: string[] = [];
  const handlers: StreamHandlers = {
    onHover: (url) => void hovered.push(url),
    onCd: (url) => void entered.push(url),
  };
  return { hovered, entered, handlers };
}

describe("parsing a ya sub line", () => {
  test("a hover line yields kind, receiver, sender, and body", () => {
    expect(
      parseEvent(`hover,0,${OURS},{"tab":0,"url":"/tmp/one.txt"}`),
    ).toEqual({
      kind: "hover",
      receiver: "0",
      sender: OURS,
      body: { tab: 0, url: "/tmp/one.txt" },
    });
  });

  test("a comma inside the payload does not split the line", () => {
    // Only the first three commas are separators; the rest belong to the JSON.
    const event = parseEvent(`cd,0,${OURS},{"tab":0,"url":"/tmp/a,b/c,d"}`);
    expect(event?.body.url).toBe("/tmp/a,b/c,d");
  });

  test("a line that is not an event yields null rather than throwing", () => {
    for (const line of [
      "",
      "hover",
      "hover,0",
      `hover,0,${OURS}`,
      `hover,0,${OURS},{ not json`,
      "Connected to existing DDS server on instance 1754500000000000",
    ]) {
      expect(parseEvent(line)).toBeNull();
    }
  });
});

describe("G2. dispatching only our own instance's events", () => {
  test("a hover from our instance reaches onHover", () => {
    const { hovered, handlers } = collect();
    dispatch(`hover,0,${OURS},{"tab":0,"url":"/tmp/one.txt"}`, OURS, handlers);
    expect(hovered).toEqual(["/tmp/one.txt"]);
  });

  test("G2 a hover from another yazi on the machine is ignored", () => {
    const { hovered, handlers } = collect();
    dispatch(
      `hover,0,${THEIRS},{"tab":0,"url":"/tmp/other.txt"}`,
      OURS,
      handlers,
    );
    expect(hovered).toEqual([]);
  });

  test("a cd from our instance reaches onCd", () => {
    const { entered, handlers } = collect();
    dispatch(`cd,0,${OURS},{"tab":0,"url":"/tmp/dir"}`, OURS, handlers);
    expect(entered).toEqual(["/tmp/dir"]);
  });

  test("G2 an absent, empty, or null url is dropped", () => {
    const { hovered, entered, handlers } = collect();
    // Measured at startup: `hover` arrives with a JSON null before state settles.
    dispatch(`hover,0,${OURS},{"tab":1,"url":null}`, OURS, handlers);
    dispatch(`hover,0,${OURS},{"tab":0,"url":""}`, OURS, handlers);
    dispatch(`cd,0,${OURS},{"tab":0}`, OURS, handlers);
    expect(hovered).toEqual([]);
    expect(entered).toEqual([]);
  });

  test("a kind we did not subscribe to is ignored", () => {
    const { hovered, entered, handlers } = collect();
    dispatch(`hey,0,${OURS},{"peers":{}}`, OURS, handlers);
    dispatch(`rename,0,${OURS},{"tab":0,"url":"/tmp/one.txt"}`, OURS, handlers);
    expect(hovered).toEqual([]);
    expect(entered).toEqual([]);
  });

  test("a malformed line is dropped without reaching a handler", () => {
    const { hovered, entered, handlers } = collect();
    dispatch("garbage", OURS, handlers);
    expect(hovered).toEqual([]);
    expect(entered).toEqual([]);
  });
});
