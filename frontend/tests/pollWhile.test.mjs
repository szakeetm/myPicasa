import assert from "node:assert/strict";
import test from "node:test";
import { setTimeout as wait } from "node:timers/promises";

test("polling waits for the previous read and stops at a terminal result", { timeout: 1000 }, async () => {
  const { pollWhile } = await import("../src/lib/pollWhile.ts");
  let releaseFirst;
  const first = new Promise((resolve) => { releaseFirst = resolve; });
  let finish;
  const finished = new Promise((resolve) => { finish = resolve; });
  let calls = 0;
  const stop = pollWhile(async () => {
    calls += 1;
    if (calls === 1) {
      await first;
      return true;
    }
    finish();
    return false;
  }, 1);
  try {
    await wait(10);
    assert.equal(calls, 1);
    releaseFirst();
    await finished;
    await wait(10);
    assert.equal(calls, 2);
  } finally {
    stop();
  }
});

test("stopping polling aborts the in-flight reader and prevents a follow-up read", async () => {
  const { pollWhile } = await import("../src/lib/pollWhile.ts");
  let release;
  const blocked = new Promise((resolve) => { release = resolve; });
  let calls = 0;
  let signal;
  const stop = pollWhile(async (currentSignal) => {
    calls += 1;
    signal = currentSignal;
    await blocked;
    return true;
  }, 1);
  stop();
  release();
  await wait(10);
  assert.equal(signal.aborted, true);
  assert.equal(calls, 1);
});
