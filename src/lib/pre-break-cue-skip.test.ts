import { expect, test } from "bun:test";
import { createCueSkipController, type CueSkipState } from "./pre-break-cue-skip";
import { preBreakCueSkipAvailable } from "./pre-break-cue";

test("skip is available only in fully expanded appearances", () => {
  for (const [remaining, expected] of [[60001,false],[60000,false],[59651,false],[59650,true],[56351,true],[56350,false],[56000,false],[10001,false],[10000,false],[9650,true],[1,true],[0,false]] as const)
    expect(preBreakCueSkipAvailable(remaining,"scheduled")).toBe(expected);
  for (const [remaining, expected] of [[17000,false],[16650,true],[13350,false],[13000,false],[11000,false],[10650,true],[1001,true],[1000,false]] as const)
    expect(preBreakCueSkipAvailable(remaining,"preview")).toBe(expected);
});

test("one authoritative skip, then confirmation and retraction", async () => {
  let resolve!: () => void, finish!: () => void;
  let calls = 0;
  const states: CueSkipState[] = [];
  const controller = createCueSkipController({
    request: () => { calls++; return new Promise<void>(done => resolve = done); },
    changed: state => states.push(state), failed: () => { throw Error("unexpected"); },
    afterConfirmation: callback => { finish=callback; return () => {}; }
  });
  const pending = controller.skip();
  await controller.skip();
  expect(calls).toBe(1); expect(states).toEqual(["pending"]);
  resolve(); await pending;
  expect(states).toEqual(["pending","skipped"]);
  finish(); await controller.skip();
  expect(states).toEqual(["pending","skipped","retracting"]); expect(calls).toBe(1);
});

test("failed skips never claim success and allow a retry", async () => {
  const states: CueSkipState[] = [], errors: unknown[] = [];
  let calls=0;
  const controller=createCueSkipController({request:async()=>{if(calls++===0)throw "expired";},changed:s=>states.push(s),failed:e=>errors.push(e),afterConfirmation:()=>()=>{}});
  await controller.skip(); expect(states).toEqual(["pending","idle"]); expect(errors).toEqual(["expired"]);
  await controller.skip(); expect(states.at(-1)).toBe("skipped"); controller.dispose();
});

test("cancellation while the native request is pending cannot resurrect the cue", async () => {
  let resolve!:()=>void;
  const states:CueSkipState[]=[];
  const controller=createCueSkipController({request:()=>new Promise<void>(done=>resolve=done),changed:s=>states.push(s),failed:()=>{}});
  const pending=controller.skip();controller.dispose();resolve();await pending;
  expect(states).toEqual(["pending"]);await controller.skip();expect(states).toEqual(["pending"]);
});

test("disposal cancels the confirmation timer", async () => {
  let cancelled=false, finish!:()=>void;
  const states:CueSkipState[]=[];
  const controller=createCueSkipController({request:async()=>{},changed:s=>states.push(s),failed:()=>{},afterConfirmation:callback=>{finish=callback;return()=>{cancelled=true;};}});
  await controller.skip();controller.dispose();finish();expect(cancelled).toBe(true);expect(states).toEqual(["pending","skipped"]);
});
