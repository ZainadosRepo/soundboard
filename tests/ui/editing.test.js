import { test } from "node:test";
import assert from "node:assert/strict";
import { TrimEditor, Viewport, columns, range, playLength, seconds, triggerModifier, accelerator } from "../../ui/editing.js";

const player = { trimStart: 0, trimEnd: 0, fadeInTime: 0, fadeOutTime: 0 };
const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ≠ ${b}`);

test("trim keeps a minimum playable length", () => {
  const e = new TrimEditor(10);
  const s = e.settingEnd(4, player);
  close(s.trimEnd, 6);
  close(e.settingStart(9, s).trimStart, 3.9);
  close(e.settingEnd(0, { ...player, trimStart: 2 }).trimEnd, 7.9);
});

test("fades fit in half the playable part and follow trims", () => {
  const e = new TrimEditor(10);
  close(e.settingFadeInEnd(8, player).fadeInTime, 5);
  const faded = { ...player, fadeInTime: 4, fadeOutTime: 4 };
  const trimmed = e.settingStart(4, faded);
  close(trimmed.fadeInTime, 3);
  close(trimmed.fadeOutTime, 3);
  close(e.settingFadeOutStart(9, player).fadeOutTime, 1);
});

test("selection edits", () => {
  const e = new TrimEditor(10);
  const sel = range(6, 2);
  const cropped = e.cropping(sel, player);
  close(cropped.trimStart, 2);
  close(cropped.trimEnd, 4);
  const fadedIn = e.fadingIn(range(1, 3), player);
  close(fadedIn.trimStart, 1);
  close(fadedIn.fadeInTime, 2);
  const fadedOut = e.fadingOut(range(7, 9), player);
  close(fadedOut.trimEnd, 1);
  close(fadedOut.fadeOutTime, 2);
  assert.deepEqual(e.reverting({ ...cropped, fadeInTime: 1 }), player);
});

test("viewport zooms, scrolls and stays inside the sound", () => {
  const v = new Viewport(8);
  v.zoomIn();
  assert.deepEqual(v.visible, { start: 2, end: 6 });
  v.scroll(10);
  assert.deepEqual(v.visible, { start: 4, end: 8 });
  v.zoomBy(2, 1);
  assert.deepEqual(v.visible, { start: 6, end: 8 });
  for (let i = 0; i < 20; i++) v.zoomIn();
  close(v.length, 0.05);
  v.zoomOut();
  close(v.length, 0.1);
  v.zoomToFit();
  assert.deepEqual(v.visible, { start: 0, end: 8 });
  close(v.timeAt(50, 100), 4);
  close(v.x(2, 100), 25);
});

test("columns keep extremes", () => {
  const peaks = [
    { low: -0.1, high: 0.1 },
    { low: -0.9, high: 0.2 },
    { low: -0.2, high: 0.8 },
    { low: 0, high: 0 },
  ];
  assert.deepEqual(columns(peaks, 0, 1, 2), [
    { low: -0.9, high: 0.2 },
    { low: -0.2, high: 0.8 },
  ]);
  assert.equal(columns(peaks, 0, 0.5, 8).length, 8);
  assert.deepEqual(columns([], 0, 1, 4), []);
});

test("formatting", () => {
  assert.equal(seconds(1.256), "1,26 s");
  assert.equal(playLength(75.4, player), "1:15");
  assert.equal(playLength(null, player), "–:––");
});

test("modifiers and global shortcuts", () => {
  assert.equal(triggerModifier({ shiftKey: true }, false), "pause");
  assert.equal(triggerModifier({ ctrlKey: true }, false), "fade");
  assert.equal(triggerModifier({ ctrlKey: true }, true), "none");
  assert.deepEqual(accelerator({ key: "s", code: "KeyS", ctrlKey: true, altKey: true }), { accelerator: "Ctrl+Alt+S" });
  assert.ok(accelerator({ key: "s", code: "KeyS", shiftKey: true }).error);
  assert.equal(accelerator({ key: "Control", code: "ControlLeft", ctrlKey: true }), null);
});
