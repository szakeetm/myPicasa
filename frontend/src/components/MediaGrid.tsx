import { startTransition, useEffect, useMemo, useRef, useState } from "react";
import dayjs from "dayjs";

import { api } from "../lib/tauri";
import { logClient } from "../lib/logger";
import type { AssetListItem } from "../lib/types";

type MediaGridProps = {
  assets: AssetListItem[];
  onSelect: (assetId: number) => void;
  onLeadingDateChange?: (value?: string) => void;
  thumbnailResetKey?: number;
  thumbnailPreload?: {
    active: boolean;
    runId: number;
  };
  onThumbnailPreloadProgress?: (value?: {
    completed: number;
    total: number;
  }) => void;
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

export function MediaGrid({
  assets,
  onSelect,
  onLeadingDateChange,
  thumbnailResetKey,
  thumbnailPreload,
  onThumbnailPreloadProgress,
}: MediaGridProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const tileRefs = useRef(new Map<number, HTMLButtonElement>());
  const [width, setWidth] = useState(1200);
  const [thumbs, setThumbs] = useState<Record<number, ThumbnailState>>({});
  const [visibleIds, setVisibleIds] = useState<number[]>([]);
  const thumbsRef = useRef<Record<number, ThumbnailState>>({});
  const requestInFlightRef = useRef(false);
  const lastBatchLogRef = useRef<{
    signature: string;
    at: number;
  }>({
    signature: "",
    at: 0,
  });
  const lastProgressLogRef = useRef<{
    completed: number;
    total: number;
  }>({
    completed: -1,
    total: -1,
  });
  const columns = columnCount(width);
  const visibleIdSet = useMemo(() => new Set(visibleIds), [visibleIds]);
  const thumbnailSize = useMemo(() => {
    const devicePixelRatio =
      typeof window === "undefined" ? 1 : Math.max(window.devicePixelRatio || 1, 1);
    const estimatedTileWidth = Math.max(width / columns, 160);
    return Math.min(512, Math.max(256, Math.ceil(estimatedTileWidth * devicePixelRatio * 0.75)));
  }, [columns, width]);

  useEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setThumbs({});
    setVisibleIds([]);
    requestInFlightRef.current = false;
    lastBatchLogRef.current = { signature: "", at: 0 };
    lastProgressLogRef.current = { completed: -1, total: -1 };
  }, [assets, thumbnailResetKey, thumbnailSize]);

  useEffect(() => {
    thumbsRef.current = thumbs;
  }, [thumbs]);

  useEffect(() => {
    const root = parentRef.current;
    if (!root) return;

    const nextVisible = new Set<number>();
    const observer = new IntersectionObserver(
      (entries) => {
        let changed = false;
        for (const entry of entries) {
          const target = entry.target as HTMLElement;
          const assetId = Number(target.dataset.assetId);
          if (!Number.isFinite(assetId)) continue;

          if (entry.isIntersecting) {
            if (!nextVisible.has(assetId)) {
              nextVisible.add(assetId);
              changed = true;
            }
          } else if (nextVisible.delete(assetId)) {
            changed = true;
          }
        }

        if (changed) {
          const nextIds = assets.filter((asset) => nextVisible.has(asset.id)).map((asset) => asset.id);
          startTransition(() => {
            setVisibleIds(nextIds);
          });
        }
      },
      {
        root,
        rootMargin: "120px 0px",
        threshold: 0.01,
      },
    );
    for (const asset of assets) {
      const element = tileRefs.current.get(asset.id);
      if (element) {
        observer.observe(element);
      }
    }

    return () => observer.disconnect();
  }, [assets, columns]);

  useEffect(() => {
    let disposed = false;

    async function processBatch() {
      if (requestInFlightRef.current) {
        return;
      }

      const visiblePendingIds = assets
        .filter((asset) => {
          const state = thumbsRef.current[asset.id];
          return visibleIdSet.has(asset.id) && (!state || state.status === "pending");
        })
        .slice(0, 12)
        .map((asset) => asset.id);

      const preloadPendingIds = thumbnailPreload?.active
        ? assets
            .filter((asset) => {
              const state = thumbsRef.current[asset.id];
              return !visibleIdSet.has(asset.id) && (!state || state.status === "pending");
            })
            .slice(0, Math.max(0, 12 - visiblePendingIds.length))
            .map((asset) => asset.id)
        : [];

      const requestIds = [...visiblePendingIds, ...preloadPendingIds];
      if (requestIds.length === 0) {
        return;
      }

      startTransition(() => {
        setThumbs((current) => {
          const next = { ...current };
          for (const id of requestIds) {
            if (!next[id]) {
              next[id] = { status: "pending" };
            }
          }
          return next;
        });
      });

      requestInFlightRef.current = true;
      try {
        const batch = await api.requestThumbnailsBatch(requestIds, thumbnailSize);
        if (disposed) {
          return;
        }

        const readyCount = batch.filter((item) => item.status === "ready").length;
        const pendingCount = batch.filter((item) => item.status === "pending").length;
        const unavailableCount = batch.filter((item) => item.status === "unavailable").length;
        const batchMode =
          visiblePendingIds.length > 0 && preloadPendingIds.length > 0
            ? "mixed"
            : visiblePendingIds.length > 0
              ? "visible"
              : "preload";
        const signature = `${batchMode}:${requestIds.length}:${readyCount}:${pendingCount}:${unavailableCount}:${thumbnailSize}`;
        const now = Date.now();
        if (
          lastBatchLogRef.current.signature !== signature ||
          now - lastBatchLogRef.current.at > 2000
        ) {
          lastBatchLogRef.current = { signature, at: now };
          void logClient(
            "grid",
            `thumb batch mode=${batchMode} size=${thumbnailSize} requested=${requestIds.length} ready=${readyCount} pending=${pendingCount} unavailable=${unavailableCount}`,
          );
        }

        startTransition(() => {
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
        });
      } catch (error) {
        await logClient("grid", `thumbnail batch failed: ${String(error)}`, "error");
        if (disposed) {
          return;
        }
      } finally {
        requestInFlightRef.current = false;
      }
    }

    void processBatch();
    const handle = window.setInterval(() => {
      void processBatch();
    }, thumbnailPreload?.active ? 120 : 220);

    return () => {
      disposed = true;
      window.clearInterval(handle);
    };
  }, [assets, thumbnailPreload?.active, thumbnailSize, visibleIdSet]);

  useEffect(() => {
    const firstVisibleAsset = assets.find((asset) => visibleIdSet.has(asset.id)) ?? assets[0];
    onLeadingDateChange?.(firstVisibleAsset?.taken_at_utc ?? undefined);
  }, [assets, onLeadingDateChange, visibleIdSet]);

  useEffect(() => {
    if (!thumbnailPreload?.active) {
      lastProgressLogRef.current = { completed: -1, total: -1 };
      onThumbnailPreloadProgress?.(undefined);
      return;
    }

    const total = assets.length;
    const completed = assets.filter((asset) => {
      const state = thumbs[asset.id];
      return state?.status === "ready" || state?.status === "unavailable";
    }).length;

    const previous = lastProgressLogRef.current;
    const changed = completed !== previous.completed || total !== previous.total;
    if (!changed) {
      return;
    }

    lastProgressLogRef.current = { completed, total };

    if (
      completed === total ||
      previous.completed < 0 ||
      completed === 0 ||
      completed - previous.completed >= 24 ||
      total !== previous.total
    ) {
      void logClient("grid", `thumb preload progress completed=${completed} total=${total}`);
    }

    onThumbnailPreloadProgress?.({ completed, total });
  }, [assets, onThumbnailPreloadProgress, thumbnailPreload?.active, thumbs]);

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
          <button
            key={asset.id}
            className="tile"
            data-asset-id={asset.id}
            ref={(element) => {
              if (element) {
                tileRefs.current.set(asset.id, element);
              } else {
                tileRefs.current.delete(asset.id);
              }
            }}
            onClick={() => onSelect(asset.id)}
          >
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
              {asset.media_kind === "video" ? (
                <button
                  className="thumb-player-button"
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    onSelect(asset.id);
                  }}
                  aria-label="Play video"
                  title="Play video"
                >
                  <span className="thumb-player-icon" aria-hidden="true">
                    ▶
                  </span>
                </button>
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
