import type { AssetListItem } from "../lib/types";

export type GridPage = {
  startCursor: number;
  count: number;
  nextCursor: number | null;
  items: AssetListItem[] | null;
};

export function commitPage(pages: GridPage[], page: GridPage): GridPage[] {
  const existing = pages.filter((entry) => entry.startCursor !== page.startCursor);
  const neighbor = existing.reduce<GridPage | undefined>((nearest, entry) => {
    if (!entry.items) return nearest;
    if (!nearest || Math.abs(entry.startCursor - page.startCursor) < Math.abs(nearest.startCursor - page.startCursor)) {
      return entry;
    }
    return nearest;
  }, undefined);
  return [...existing, page]
    .sort((left, right) => left.startCursor - right.startCursor)
    .map((entry) => entry.items && entry !== page && entry !== neighbor ? { ...entry, items: null } : entry);
}

export function gridSlots(pages: GridPage[], start: number, end: number) {
  // Page positions stay stable when their media records are evicted.
  const slots: { index: number; pageStart: number; asset: AssetListItem | undefined }[] = [];
  let low = 0;
  let high = pages.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    const page = pages[middle];
    if (page.startCursor + page.count <= start) low = middle + 1;
    else high = middle;
  }
  for (let i = low; i < pages.length; i += 1) {
    const page = pages[i];
    if (page.startCursor >= end) break;
    const pageEnd = Math.min(end, page.startCursor + page.count);
    for (let index = Math.max(start, page.startCursor); index < pageEnd; index += 1) {
      slots.push({ index, pageStart: page.startCursor, asset: page.items?.[index - page.startCursor] });
    }
  }
  return slots;
}

export function getGridWindow(
  slotCount: number,
  scrollTop: number,
  { columns, height, rowPitch, padding, overscan }: {
    columns: number;
    height: number;
    rowPitch: number;
    padding: number;
    overscan: number;
  },
) {
  const rowCount = Math.ceil(slotCount / columns);
  const firstVisibleRow = Math.min(rowCount, Math.max(0, Math.floor((scrollTop - padding) / rowPitch)));
  const endVisibleRow = Math.min(rowCount, Math.max(firstVisibleRow + 1, Math.ceil((scrollTop + height - padding) / rowPitch)));
  const startRow = Math.max(0, firstVisibleRow - overscan);
  const endRow = Math.min(rowCount, endVisibleRow + overscan);
  return {
    firstVisibleRow,
    endVisibleRow,
    startRow,
    endRow,
    paddingTop: padding + startRow * rowPitch,
    paddingBottom: padding + (rowCount - endRow) * rowPitch,
  };
}


// Normal browsing prepares only rendered media, with the visible rows first.
// An explicit preload can also process the rest of the hydrated pages.
export function gridMediaWork(
  assets: AssetListItem[],
  rendered: ReturnType<typeof gridSlots>,
  visible: ReturnType<typeof gridSlots>,
  preload: boolean,
): AssetListItem[] {
  const candidates = [
    ...visible.map(({ asset }) => asset),
    ...rendered.map(({ asset }) => asset),
    ...(preload ? assets : []),
  ];
  const unique = new Map<number, AssetListItem>();
  for (const asset of candidates) {
    if (asset) unique.set(asset.id, asset);
  }
  return Array.from(unique.values());
}
