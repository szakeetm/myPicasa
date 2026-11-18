import test from "node:test";
import assert from "node:assert/strict";
import { commitPage, getGridWindow, gridMediaWork, gridSlots } from "../src/app/gridPaging.ts";

const page = (startCursor) => ({
  startCursor,
  count: 200,
  nextCursor: startCursor + 200,
  items: Array.from({ length: 200 }, (_, i) => ({ id: startCursor + i })),
});

test("eviction retains page geometry and supports revisiting an interior slot", () => {
  let pages = [];
  for (let cursor = 0; cursor < 10_000; cursor += 200) {
    pages = commitPage(pages, page(cursor));
  }

  assert.equal(pages.length, 50);
  assert.equal(pages.filter(({ items }) => items).length, 2);
  assert.deepEqual(gridSlots(pages, 531, 534), [531, 532, 533].map((index) => ({
    index,
    pageStart: 400,
    asset: undefined,
  })));
  pages = commitPage(pages, page(400));
  assert.equal(gridSlots(pages, 532, 533)[0].asset.id, 532);
  assert.equal(gridSlots(pages, 9998, 10_000).length, 2);
});

test("an empty terminal page preserves earlier positions", () => {
  const pages = commitPage([page(0)], { startCursor: 200, count: 0, nextCursor: null, items: [] });
  assert.equal(gridSlots(pages, 199, 204).length, 1);
});
test("the rendered window has the same total height anywhere in a large grid", () => {
  const geometry = { columns: 5, height: 800, rowPitch: 272, padding: 6, overscan: 3 };
  const start = getGridWindow(10000, 0, geometry);
  const middle = getGridWindow(10000, 272000, geometry);
  const end = getGridWindow(10000, 543200, geometry);
  for (const window of [start, middle, end]) {
    const renderedRows = window.endRow - window.startRow;
    assert.equal(window.paddingTop + renderedRows * geometry.rowPitch + window.paddingBottom, 2000 * geometry.rowPitch + 12);
    assert.ok(renderedRows <= 10);
  }
  assert.equal(middle.firstVisibleRow, 999);
  assert.equal(end.endRow, 2000);
});

test("normal media work starts with visible slots and stays within the rendered window", () => {
  const slots = gridSlots([page(0), page(200)], 186, 224);
  const visible = gridSlots([page(0), page(200)], 198, 208);
  const assets = [...page(0).items, ...page(200).items];
  const work = gridMediaWork(assets, slots, visible, false);
  assert.deepEqual(work.slice(0, 10).map(({ id }) => id), visible.map(({ index }) => index));
  assert.equal(work.length, 38);
  assert.equal(new Set(work.map(({ id }) => id)).size, 38);
  assert.equal(gridMediaWork(assets, slots, visible, true).length, 400);
});
