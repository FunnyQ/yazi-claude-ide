// Polls several yazi ids with the sidecar's own liveness probe and prints every
// transition, so the succession spike measures the exact call src/yazi.ts makes
// rather than an approximation of it. TMPDIR is inherited, which is what aims
// `ya` at the private DDS.
//
//   bun probe-watch.ts <id> [<id> …] <seconds>
import { probeAlive } from "../../src/yazi.ts";

const ids = process.argv.slice(2, -1);
const seconds = Number(process.argv.at(-1));
const start = performance.now();
const elapsed = () => (performance.now() - start) / 1000;

const up = new Map<string, boolean | null>(ids.map((id) => [id, null]));
const failures = new Map<string, number>(ids.map((id) => [id, 0]));
let ticks = 0;

while (elapsed() < seconds) {
  ticks++;
  // All ids in the same tick, so their transitions share a clock.
  const results = await Promise.all(ids.map((id) => probeAlive(id)));
  ids.forEach((id, i) => {
    const alive = results[i]!;
    if (!alive) failures.set(id, failures.get(id)! + 1);
    if (up.get(id) !== alive) {
      const at = elapsed().toFixed(2).padStart(6);
      console.log(`  t+${at}s  ${id}  ${alive ? "up" : "DOWN"}`);
      up.set(id, alive);
    }
  });
  await Bun.sleep(100);
}

console.log(`  ${ticks} ticks over ${seconds}s`);
for (const id of ids) console.log(`  ${id}: ${failures.get(id)} failures`);
