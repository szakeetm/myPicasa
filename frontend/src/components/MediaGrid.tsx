import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import dayjs from "dayjs";

import { api } from "../lib/tauri";
import { logClient } from "../lib/logger";
import type { AssetListItem } from "../lib/types";

type MediaGridProps = {
  assets: AssetListItem[];
  onSelect: (assetId: number) => void;
};

const ROW_HEIGHT = 228;

type ThumbnailState = {
  status: "pending" | "ready" | "unavailable";
  src?: string | null;
};

function columnCount(width: number) {
  if (width < 640) return 1;
  if (width < 980) return 2;
  if (width < 1320) return 3;
  return 4;
}

export function MediaGrid({ assets, onSelect }: MediaGridProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(1200);
  const [requestTick, setRequestTick] = useState(0);
  const [thumbs, setThumbs] = useState<Record<number, ThumbnailState>>({});
  const columns = columnCount(width);
  const rows = useMemo(() => {
    const output: AssetListItem[][] = [];
    for (let index = 0; index < assets.length; index += columns) {
      output.push(assets.slice(index, index + columns));
    }
    return output;
  }, [assets, columns]);

  useEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 4,
  });

  const visibleIds = useMemo(
    () =>
      rowVirtualizer
        .getVirtualItems()
        .flatMap((item) => rows[item.index] ?? [])
        .map((asset) => asset.id),
    [rowVirtualizer, rows],
  );

  useEffect(() => {
    const timer = window.setInterval(() => {
      setRequestTick((value) => value + 1);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (visibleIds.length === 0) return;
    const timer = window.setTimeout(async () => {
      const pending = visibleIds.filter((id) => {
        const state = thumbs[id];
        return !state || state.status === "pending";
      });
      if (pending.length === 0) return;

      setThumbs((current) => {
        const next = { ...current };
        for (const id of pending) {
          if (!next[id]) {
            next[id] = { status: "pending" };
          }
        }
        return next;
      });

      await logClient("grid", `requesting batch of ${pending.length} visible thumbnails`);
      try {
        const batch = await api.requestThumbnailsBatch(pending, 256);
        setThumbs((current) => {
          const next = { ...current };
          for (const item of batch) {
            if (item.status === "ready") {
              next[item.asset_id] = { status: "ready", src: item.data_url ?? null };
            } else if (item.status === "unavailable") {
              next[item.asset_id] = { status: "unavailable", src: null };
            } else if (!next[item.asset_id]) {
              next[item.asset_id] = { status: "pending" };
            }
          }
          return next;
        });
      } catch (error) {
        await logClient("grid", `thumbnail batch failed: ${String(error)}`, "error");
        setThumbs((current) => {
          const next = { ...current };
          for (const id of pending) {
            next[id] = { status: "unavailable", src: null };
          }
          return next;
        });
      }
    }, 150);

    return () => window.clearTimeout(timer);
  }, [requestTick, thumbs, visibleIds]);

  if (assets.length === 0) {
    return <div className="empty-state">No indexed assets match the current view.</div>;
  }

  return (
    <div className="grid-scroller" ref={parentRef}>
      <div className="grid-inner" style={{ height: rowVirtualizer.getTotalSize() }}>
        {rowVirtualizer.getVirtualItems().map((virtualRow) => {
          const row = rows[virtualRow.index] ?? [];
          return (
            <div
              className="grid-row"
              key={virtualRow.key}
              style={{
                transform: `translateY(${virtualRow.start}px)`,
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              }}
            >
              {row.map((asset) => (
                <button key={asset.id} className="tile" onClick={() => onSelect(asset.id)}>
                  <div className="thumb">
                    {thumbs[asset.id]?.status === "ready" ? (
                      <img src={thumbs[asset.id]?.src ?? ""} alt={asset.title ?? "asset"} />
                    ) : thumbs[asset.id]?.status === "unavailable" ? (
                      <div>Preview unavailable</div>
                    ) : (
                      <div>{asset.media_kind === "video" ? "Video preview pending" : "Loading preview"}</div>
                    )}
                  </div>
                  <div className="tile-body">
                    <strong>{asset.title ?? "Untitled asset"}</strong>
                    <div className="muted">
                      {asset.taken_at_utc ? dayjs(asset.taken_at_utc).format("YYYY-MM-DD HH:mm") : "Unknown date"}
                    </div>
                    <div className="chips">
                      <span className="chip">{asset.media_kind}</span>
                      {asset.albums.slice(0, 2).map((album) => (
                        <span className="chip" key={album}>
                          {album}
                        </span>
                      ))}
                    </div>
                  </div>
                </button>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
