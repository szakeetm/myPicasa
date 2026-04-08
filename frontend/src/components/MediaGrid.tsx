import { useEffect, useMemo, useRef, useState } from "react";
import dayjs from "dayjs";

import { api } from "../lib/tauri";
import { logClient } from "../lib/logger";
import type { AssetListItem } from "../lib/types";

type MediaGridProps = {
  assets: AssetListItem[];
  onSelect: (assetId: number) => void;
};

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

  useEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  const visibleIds = useMemo(
    () => assets.map((asset) => asset.id),
    [assets],
  );
  const visibleTitles = useMemo(
    () => assets.map((asset) => asset.title ?? `asset-${asset.id}`),
    [assets],
  );

  useEffect(() => {
    const timer = window.setInterval(() => {
      setRequestTick((value) => value + 1);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    setThumbs({});
  }, [assets]);

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
      console.info("thumbnail_visible_assets", {
        visibleIds,
        visibleTitles,
      });
      try {
        const batch = await api.requestThumbnailsBatch(pending, 256);
        const readyIds = batch.filter((item) => item.status === "ready").map((item) => item.asset_id);
        const pendingIds = batch.filter((item) => item.status === "pending").map((item) => item.asset_id);
        const unavailableIds = batch.filter((item) => item.status === "unavailable").map((item) => item.asset_id);
        console.info("thumbnail_batch_client", {
          requested: pending,
          readyIds,
          pendingIds,
          unavailableIds,
        });
        if (readyIds.length > 0) {
          console.info("thumbnail_batch_client_ready", readyIds);
        }
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
  }, [requestTick, thumbs, visibleIds, visibleTitles]);

  if (assets.length === 0) {
    return <div className="empty-state">No indexed assets match the current view.</div>;
  }

  return (
    <div className="grid-scroller" ref={parentRef}>
      <div
        className="media-grid"
        style={{
          gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
        }}
      >
        {assets.map((asset) => (
          <button key={asset.id} className="tile" onClick={() => onSelect(asset.id)}>
            <div className="thumb">
              {thumbs[asset.id]?.status === "ready" ? (
                <img src={thumbs[asset.id]?.src ?? ""} alt={asset.title ?? "asset"} />
              ) : thumbs[asset.id]?.status === "unavailable" ? (
                <div>Preview unavailable</div>
              ) : (
                <div>{asset.media_kind === "video" ? "Video preview pending" : "Loading preview"}</div>
              )}
              {asset.media_kind === "video" ? (
                <>
                  <div className="thumb-play-badge" aria-hidden="true">
                    <span className="thumb-play-icon">▶</span>
                    <span>Video</span>
                  </div>
                  {asset.duration_ms ? (
                    <div className="thumb-duration-badge">{formatDuration(asset.duration_ms)}</div>
                  ) : null}
                </>
              ) : null}
              {asset.has_live_photo ? (
                <div className="thumb-live-badge" aria-hidden="true" title="Live Photo">
                  ◎
                </div>
              ) : null}
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
    </div>
  );
}

function formatDuration(durationMs: number) {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}
