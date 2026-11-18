import { startTransition, useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState } from "react";
import dayjs from "dayjs";

import { getGridWindow, gridSlots, type GridPage } from "../app/gridPaging";
import { api } from "../lib/tauri";
import { logClient } from "../lib/logger";
import { materializeImageSrc } from "../lib/mediaSrc";
import type { AssetListItem, ViewerPlaybackHint, ViewerPlaybackSupport } from "../lib/types";

const GRID_TILE_WIDTH = 210;
const GRID_GAP = 6;
const GRID_PADDING = 6;
const GRID_ROW_HEIGHT = GRID_TILE_WIDTH + 56;
const GRID_ROW_PITCH = GRID_ROW_HEIGHT + GRID_GAP;
const ROW_OVERSCAN = 3;
const GRID_THUMBNAIL_SIZE = 210;
const THUMB_PROBE_BATCH_SIZE = 96;
const THUMB_ENQUEUE_BATCH_SIZE = 24;
const PREVIEW_PROBE_BATCH_SIZE = 96;
const PREVIEW_ENQUEUE_BATCH_SIZE = 24;

type MediaGridProps = {
  assets: AssetListItem[];
  viewMode: "timeline" | "album";
  pages: GridPage[];
  onSelect: (assetId: number, index: number) => void;
  onHydratePlaceholderPage?: (pageStartCursor: number) => void;
  viewerPreviewReadyAssetIds?: number[];
  onLeadingDateChange?: (value?: string) => void;
  thumbnailResetKey?: number;
  hasMoreAfter?: boolean;
  isLoadingMore?: boolean;
  onLoadMore?: () => void;
  thumbnailPreload?: {
    active: boolean;
    runId: number;
  };
  viewerPreviewSize: number;
  onThumbnailPreloadProgress?: (value?: {
    thumbsCompleted: number;
    thumbsTotal: number;
    previewsCompleted: number;
    previewsTotal: number;
  }) => void;
  viewerPlaybackSupport: ViewerPlaybackSupport;
};

type ThumbnailState = {
  status?: "pending" | "ready" | "unavailable";
  src?: string | null;
  previewStatus?: "pending" | "ready" | "unavailable";
  thumbChecked?: boolean;
  previewChecked?: boolean;
};

function columnCount(width: number) {
  const usableWidth = Math.max(0, width - GRID_PADDING * 2);
  return Math.max(1, Math.floor((usableWidth + GRID_GAP) / (GRID_TILE_WIDTH + GRID_GAP)));
}

function thumbStatusLabel(asset: AssetListItem, state?: ThumbnailState) {
  if (!state?.thumbChecked) {
    return asset.media_kind === "video" ? "Video thumb not checked yet" : "Thumb not checked yet";
  }
  if (state?.status === "unavailable") {
    return asset.media_kind === "video" ? "Video thumb unavailable" : "Thumb unavailable";
  }
  if (state?.status === "pending") {
    return asset.media_kind === "video" ? "Requested video thumb" : "Requested thumb";
  }
  if (state?.status === undefined) {
    return asset.media_kind === "video" ? "Loading video thumb" : "Loading thumb";
  }
  return asset.media_kind === "video" ? "Loading video thumb" : "Loading thumb";
}

export function MediaGrid({
  assets,
  viewMode,
  pages,
  onSelect,
  onHydratePlaceholderPage,
  viewerPreviewReadyAssetIds = [],
  onLeadingDateChange,
  thumbnailResetKey,
  hasMoreAfter = false,
  isLoadingMore = false,
  onLoadMore,
  thumbnailPreload,
  viewerPreviewSize,
  onThumbnailPreloadProgress,
  viewerPlaybackSupport,
}: MediaGridProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const loadMoreRef = useRef<HTMLDivElement | null>(null);
  const pendingResizeScroll = useRef<number>(null);
  const [viewport, setViewport] = useState({ columns: 1, height: 0, scrollTop: 0 });
  const [thumbs, setThumbs] = useState<Record<number, ThumbnailState>>({});
  const [videoPlaybackHints, setVideoPlaybackHints] = useState<Record<number, ViewerPlaybackHint["status"]>>({});
  const thumbsRef = useRef<Record<number, ThumbnailState>>({});
  const thumbProbeInFlightRef = useRef(false);
  const thumbGenerateInFlightRef = useRef(false);
  const previewProbeInFlightRef = useRef(false);
  const previewGenerateInFlightRef = useRef(false);
  const lastBatchLogRef = useRef<{
    signature: string;
    at: number;
  }>({
    signature: "",
    at: 0,
  });
  const lastProgressLogRef = useRef<{
    thumbsCompleted: number;
    thumbsTotal: number;
    previewsCompleted: number;
    previewsTotal: number;
  }>({
    thumbsCompleted: -1,
    thumbsTotal: -1,
    previewsCompleted: -1,
    previewsTotal: -1,
  });
  const lastIdleLogRef = useRef<{
    thumbSignature: string;
    previewSignature: string;
    at: number;
  }>({
    thumbSignature: "",
    previewSignature: "",
    at: 0,
  });
  const { columns, height, scrollTop } = viewport;
  const lastPage = pages.at(-1);
  const slotCount = lastPage ? lastPage.startCursor + lastPage.count : 0;
  const { firstVisibleRow, endVisibleRow, startRow, endRow, paddingTop, paddingBottom } = getGridWindow(
    slotCount,
    scrollTop,
    { columns, height, rowPitch: GRID_ROW_PITCH, padding: GRID_PADDING, overscan: ROW_OVERSCAN },
  );
  const entries = useMemo(
    () => gridSlots(pages, startRow * columns, endRow * columns),
    [pages, startRow, endRow, columns],
  );
  const visibleSlots = useMemo(
    () => gridSlots(pages, firstVisibleRow * columns, endVisibleRow * columns),
    [pages, firstVisibleRow, endVisibleRow, columns],
  );
  const effectiveVisibleIdSet = useMemo(
    () => new Set(visibleSlots.flatMap(({ asset }) => asset ? [asset.id] : [])),
    [visibleSlots],
  );
  const viewerPreviewReadySet = useMemo(
    () => new Set(viewerPreviewReadyAssetIds),
    [viewerPreviewReadyAssetIds],
  );
  const thumbnailSize = GRID_THUMBNAIL_SIZE;

  useLayoutEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    let columns = columnCount(element.clientWidth);
    function measure() {
      if (!element) return;
      const nextColumns = columnCount(element.clientWidth);
      let scrollTop = element.scrollTop;
      if (nextColumns !== columns) {
        const row = Math.max(0, Math.floor((element.scrollTop - GRID_PADDING) / GRID_ROW_PITCH));
        const nextRow = Math.floor(row * columns / nextColumns);
        scrollTop += (nextRow - row) * GRID_ROW_PITCH;
        pendingResizeScroll.current = scrollTop;
        columns = nextColumns;
      }
      setViewport({ columns, height: element.clientHeight, scrollTop });
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const missingPages = new Set(visibleSlots.filter(({ asset }) => !asset).map(({ pageStart }) => pageStart));
    for (const cursor of missingPages) onHydratePlaceholderPage?.(cursor);
  }, [visibleSlots, onHydratePlaceholderPage]);

  useLayoutEffect(() => {
    const root = parentRef.current;
    if (!root || pendingResizeScroll.current === null) return;
    root.scrollTop = pendingResizeScroll.current;
    pendingResizeScroll.current = null;
  }, [columns]);

  useEffect(() => {
    setThumbs({});
    thumbProbeInFlightRef.current = false;
    thumbGenerateInFlightRef.current = false;
    previewProbeInFlightRef.current = false;
    previewGenerateInFlightRef.current = false;
    lastBatchLogRef.current = { signature: "", at: 0 };
    lastProgressLogRef.current = {
      thumbsCompleted: -1,
      thumbsTotal: -1,
      previewsCompleted: -1,
      previewsTotal: -1,
    };
    lastIdleLogRef.current = {
      thumbSignature: "",
      previewSignature: "",
      at: 0,
    };
    setVideoPlaybackHints({});
  }, [thumbnailResetKey, thumbnailSize]);

  useEffect(() => {
    let cancelled = false;

    async function refreshPlaybackHints() {
      const videoIds = assets
        .filter((asset) => asset.media_kind === "video" && effectiveVisibleIdSet.has(asset.id))
        .map((asset) => asset.id);

      if (videoIds.length === 0) {
        if (!cancelled) {
          setVideoPlaybackHints({});
        }
        return;
      }

      try {
        const hints = await api.getViewerPlaybackHints(videoIds, viewerPlaybackSupport);
        if (cancelled) {
          return;
        }
        setVideoPlaybackHints(
          hints.reduce<Record<number, ViewerPlaybackHint["status"]>>((acc, hint) => {
            acc[hint.asset_id] = hint.status;
            return acc;
          }, {}),
        );
      } catch (error) {
        if (!cancelled) {
          await logClient("grid", `viewer playback hint refresh failed: ${String(error)}`, "error");
        }
      }
    }

    void refreshPlaybackHints();
    const timer = window.setInterval(() => {
      void refreshPlaybackHints();
    }, 2000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [assets, effectiveVisibleIdSet, viewerPlaybackSupport]);

  useEffect(() => {
    const assetIds = new Set(assets.map((asset) => asset.id));
    startTransition(() => {
      setThumbs((current) => {
        let changed = false;
        const next: Record<number, ThumbnailState> = {};
        for (const [assetId, value] of Object.entries(current)) {
          const numericId = Number(assetId);
          if (assetIds.has(numericId)) {
            next[numericId] = value;
          } else {
            changed = true;
          }
        }
        return changed ? next : current;
      });
    });
  }, [assets]);

  useEffect(() => {
    thumbsRef.current = thumbs;
  }, [thumbs]);

  useEffect(() => {
    if (viewerPreviewReadySet.size === 0 || assets.length === 0) {
      return;
    }

    startTransition(() => {
      setThumbs((current) => {
        let changed = false;
        const next = { ...current };
        for (const asset of assets) {
          if (!viewerPreviewReadySet.has(asset.id)) {
            continue;
          }
          const existing = next[asset.id];
          if (existing?.previewStatus === "ready") {
            continue;
          }
          next[asset.id] = {
            ...existing,
            previewStatus: "ready",
            previewChecked: true,
          };
          changed = true;
        }
        return changed ? next : current;
      });
    });
  }, [assets, viewerPreviewReadySet]);

  function summarizeThumbStates() {
    let ready = 0;
    let unavailable = 0;
    let pending = 0;
    let missing = 0;
    let unknown = 0;

    for (const asset of assets) {
      const state = thumbsRef.current[asset.id];
      if (!state?.thumbChecked) {
        unknown += 1;
        continue;
      }
      const status = state.status;
      if (status === "ready") {
        ready += 1;
      } else if (status === "unavailable") {
        unavailable += 1;
      } else if (status === "pending") {
        pending += 1;
      } else {
        missing += 1;
      }
    }

    return { ready, unavailable, pending, missing, unknown };
  }

  function summarizePreviewStates() {
    let ready = 0;
    let unavailable = 0;
    let pending = 0;
    let waitingForThumb = 0;
    let missing = 0;
    let unknown = 0;

    for (const asset of assets) {
      const state = thumbsRef.current[asset.id];
      if (!state?.previewChecked) {
        unknown += 1;
        continue;
      }
      const previewStatus = state?.previewStatus;
      if (previewStatus === "ready") {
        ready += 1;
      } else if (previewStatus === "unavailable") {
        unavailable += 1;
      } else if (previewStatus === "pending") {
        pending += 1;
      } else if (state?.status === "ready") {
        missing += 1;
      } else {
        waitingForThumb += 1;
      }
    }

    return { ready, unavailable, pending, waitingForThumb, missing, unknown };
  }

  const logIdleSnapshot = useEffectEvent((kind: "thumb" | "preview", reason: string) => {
    if (!thumbnailPreload?.active || assets.length === 0) {
      return;
    }

    const thumbSummary = summarizeThumbStates();
    const previewSummary = summarizePreviewStates();
    const signature = `${reason}|t:${thumbSummary.ready}/${thumbSummary.unavailable}/${thumbSummary.pending}/${thumbSummary.missing}/${thumbSummary.unknown}|p:${previewSummary.ready}/${previewSummary.unavailable}/${previewSummary.pending}/${previewSummary.waitingForThumb}/${previewSummary.missing}/${previewSummary.unknown}|v:${effectiveVisibleIdSet.size}|a:${assets.length}`;
    const now = Date.now();
    const previousSignature =
      kind === "thumb" ? lastIdleLogRef.current.thumbSignature : lastIdleLogRef.current.previewSignature;

    if (previousSignature === signature && now - lastIdleLogRef.current.at < 5000) {
      return;
    }

    lastIdleLogRef.current = {
      ...lastIdleLogRef.current,
      at: now,
      ...(kind === "thumb"
        ? { thumbSignature: signature }
        : { previewSignature: signature }),
    };

    void logClient(
      "grid",
      `${kind} preload idle reason=${reason} visible=${effectiveVisibleIdSet.size}/${assets.length} thumbs ready=${thumbSummary.ready} unavailable=${thumbSummary.unavailable} pending=${thumbSummary.pending} missing=${thumbSummary.missing} unknown=${thumbSummary.unknown} previews ready=${previewSummary.ready} unavailable=${previewSummary.unavailable} pending=${previewSummary.pending} waiting_for_thumb=${previewSummary.waitingForThumb} missing=${previewSummary.missing} unknown=${previewSummary.unknown}`,
    );
  });

  useEffect(() => {
    let disposed = false;

    async function processPreviewPass() {
      const previewEligibleIds = assets
        .filter((asset) => {
          const state = thumbsRef.current[asset.id];
          return (
            asset.media_kind !== "video" &&
            state?.status === "ready"
          );
        })
        .map((asset) => asset.id);

      const probeIds = previewEligibleIds
        .filter((assetId) => {
          const state = thumbsRef.current[assetId];
          return !state?.previewChecked || state.previewStatus === "pending";
        })
        .slice(0, PREVIEW_PROBE_BATCH_SIZE);

      let probedMissingIds: number[] = [];

      if (probeIds.length > 0 && !previewProbeInFlightRef.current) {
        previewProbeInFlightRef.current = true;
        try {
          const probeBatch = await api.requestThumbnailsBatch(probeIds, viewerPreviewSize, true, true);
          if (disposed) {
            return;
          }

          probedMissingIds = probeBatch
            .filter((item) => item.status === "missing" || item.status === "stale")
            .map((item) => item.asset_id);

          startTransition(() => {
            setThumbs((current) => {
              const next = { ...current };
              for (const item of probeBatch) {
                next[item.asset_id] = {
                  ...next[item.asset_id],
                  previewChecked: true,
                  previewStatus:
                    item.status === "ready"
                      ? "ready"
                      : item.status === "unavailable"
                        ? "unavailable"
                        : item.status === "pending" || item.status === "stale"
                          ? "pending"
                          : undefined,
                };
              }
              return next;
            });
          });
        } catch (error) {
          if (!disposed) {
            await logClient("grid", `viewer preview cache probe failed requested=${probeIds.length}: ${String(error)}`, "error");
          }
          return;
        } finally {
          previewProbeInFlightRef.current = false;
        }
      } else if (probeIds.length > 0) {
        logIdleSnapshot("preview", "probe_in_flight");
      }

      const visiblePreviewIds = previewEligibleIds
        .filter((assetId) => {
          const state = thumbsRef.current[assetId];
          return (
            state?.previewChecked &&
            (state.previewStatus === undefined || state.previewStatus === "pending")
          );
        });

      const targetIds = [...new Set([...probedMissingIds, ...visiblePreviewIds])].slice(0, PREVIEW_ENQUEUE_BATCH_SIZE);
      if (targetIds.length === 0) {
        logIdleSnapshot("preview", "no_preview_targets");
        return;
      }

      if (previewGenerateInFlightRef.current) {
        logIdleSnapshot("preview", "generation_in_flight");
        return;
      }

      void logClient(
        "grid",
        `preview request visible=${visiblePreviewIds.join(",") || "none"} preload=none requested=${targetIds.join(",")}`,
      );

      startTransition(() => {
        setThumbs((current) => {
          const next = { ...current };
          for (const targetId of targetIds) {
            next[targetId] = {
              ...next[targetId],
              previewStatus: "pending",
              previewChecked: true,
            };
          }
          return next;
        });
      });

      previewGenerateInFlightRef.current = true;
      try {
        const requestStarted = performance.now();
        const batch = await api.requestThumbnailsBatch(targetIds, viewerPreviewSize, true);
        if (disposed) {
          return;
        }

        startTransition(() => {
          setThumbs((current) => {
            const next = { ...current };
            for (const item of batch) {
              next[item.asset_id] = {
                ...next[item.asset_id],
                previewStatus:
                  item.status === "ready"
                    ? "ready"
                    : item.status === "unavailable"
                      ? "unavailable"
                      : "pending",
                previewChecked: true,
              };
            }
            return next;
          });
        });

        const readyCount = batch.filter((item) => item.status === "ready").length;
        const pendingCount = batch.filter((item) => item.status === "pending").length;
        const unavailableCount = batch.filter((item) => item.status === "unavailable").length;
        void logClient(
          "grid",
          `viewer preview batch ready=${readyCount} pending=${pendingCount} unavailable=${unavailableCount} requested=${targetIds.length} size=${viewerPreviewSize} mode=auto elapsed_ms=${Math.round(performance.now() - requestStarted)}`,
        );
      } catch (error) {
        if (disposed) {
          return;
        }
        startTransition(() => {
          setThumbs((current) => {
            const next = { ...current };
            for (const targetId of targetIds) {
              next[targetId] = {
                ...next[targetId],
                previewStatus: undefined,
                previewChecked: false,
              };
            }
            return next;
          });
        });
        await logClient("grid", `viewer preview batch failed requested=${targetIds.length}: ${String(error)}`, "error");
      } finally {
        previewGenerateInFlightRef.current = false;
      }
    }
    void processPreviewPass();
    const handle = window.setInterval(() => {
      void processPreviewPass();
    }, thumbnailPreload?.active ? 120 : 180);

    return () => {
      disposed = true;
      window.clearInterval(handle);
    };
  }, [assets, thumbnailPreload?.active, viewerPreviewSize, thumbnailResetKey]);

  useEffect(() => {
    const root = parentRef.current;
    const sentinel = loadMoreRef.current;
    if (!root || !sentinel || !hasMoreAfter) {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting) && !isLoadingMore) {
          onLoadMore?.();
        }
      },
      {
        root,
        rootMargin: "400px 0px",
        threshold: 0.01,
      },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMoreAfter, isLoadingMore, onLoadMore, assets.length]);

  useEffect(() => {
    let disposed = false;

    async function processBatch() {
      const thumbEligibleIds = assets.map((asset) => asset.id);
      const probeIds = thumbEligibleIds
        .filter((assetId) => {
          const state = thumbsRef.current[assetId];
          return !state?.thumbChecked || state.status === "pending";
        })
        .slice(0, THUMB_PROBE_BATCH_SIZE);

      let probedMissingIds: number[] = [];

      if (probeIds.length > 0 && !thumbProbeInFlightRef.current) {
        thumbProbeInFlightRef.current = true;
        try {
          const probeBatch = await api.requestThumbnailsBatch(probeIds, thumbnailSize, false, true);
          if (disposed) {
            return;
          }

          probedMissingIds = probeBatch
            .filter((item) => item.status === "missing")
            .map((item) => item.asset_id);

          startTransition(() => {
            setThumbs((current) => {
              const next = { ...current };
              for (const item of probeBatch) {
                next[item.asset_id] = {
                  ...next[item.asset_id],
                  thumbChecked: true,
                  status:
                    item.status === "ready"
                      ? "ready"
                      : item.status === "unavailable"
                        ? "unavailable"
                        : item.status === "pending"
                          ? "pending"
                          : undefined,
                  src:
                    item.status === "ready"
                      ? materializeImageSrc(item.data_url) ?? next[item.asset_id]?.src ?? null
                      : next[item.asset_id]?.src,
                };
              }
              return next;
            });
          });
        } catch (error) {
          await logClient("grid", `thumbnail cache probe failed requested=${probeIds.length}: ${String(error)}`, "error");
          return;
        } finally {
          thumbProbeInFlightRef.current = false;
        }
      } else if (probeIds.length > 0) {
        logIdleSnapshot("thumb", "probe_in_flight");
      }

      const visiblePendingIds = thumbEligibleIds.filter((assetId) => {
        const state = thumbsRef.current[assetId];
        return state?.thumbChecked && (state.status === undefined || state.status === "pending");
      });

      const requestIds = [...new Set([...probedMissingIds, ...visiblePendingIds])].slice(0, THUMB_ENQUEUE_BATCH_SIZE);
      if (requestIds.length === 0) {
        logIdleSnapshot("thumb", "no_thumb_targets");
        return;
      }

      if (thumbGenerateInFlightRef.current) {
        logIdleSnapshot("thumb", "generation_in_flight");
        return;
      }

      startTransition(() => {
        setThumbs((current) => {
          const next = { ...current };
          for (const id of requestIds) {
            next[id] = {
              ...next[id],
              status: "pending",
              thumbChecked: true,
            };
          }
          return next;
        });
      });

      thumbGenerateInFlightRef.current = true;
      try {
        const batch = await api.requestThumbnailsBatch(requestIds, thumbnailSize);
        if (disposed) {
          return;
        }

        startTransition(() => {
          setThumbs((current) => {
            const next = { ...current };
            for (const item of batch) {
              if (item.status === "ready") {
                next[item.asset_id] = {
                  ...next[item.asset_id],
                  status: "ready",
                  src: materializeImageSrc(item.data_url) ?? null,
                  thumbChecked: true,
                };
              } else if (item.status === "unavailable") {
                next[item.asset_id] = {
                  ...next[item.asset_id],
                  status: "unavailable",
                  src: null,
                  thumbChecked: true,
                };
              } else {
                next[item.asset_id] = {
                  ...next[item.asset_id],
                  status: "pending",
                  thumbChecked: true,
                };
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
        thumbGenerateInFlightRef.current = false;
      }
    }

    void processBatch();
    const handle = window.setInterval(() => {
      void processBatch();
    }, thumbnailPreload?.active ? 120 : 180);

    return () => {
      disposed = true;
      window.clearInterval(handle);
    };
  }, [assets, thumbnailPreload?.active, thumbnailSize, thumbnailResetKey]);

  useEffect(() => {
    const firstVisibleAsset = visibleSlots.find(({ asset }) => asset)?.asset;
    onLeadingDateChange?.(firstVisibleAsset?.taken_at_utc ?? undefined);
  }, [visibleSlots, onLeadingDateChange]);

  useEffect(() => {
    if (!thumbnailPreload?.active) {
      lastProgressLogRef.current = {
        thumbsCompleted: -1,
        thumbsTotal: -1,
        previewsCompleted: -1,
        previewsTotal: -1,
      };
      onThumbnailPreloadProgress?.(undefined);
      return;
    }

    const thumbsTotal = assets.length;
    const thumbsCompleted = assets.filter((asset) => {
      const state = thumbs[asset.id];
      return state?.status === "ready" || state?.status === "unavailable";
    }).length;
    const previewsTotal = assets.length;
    const previewsCompleted = assets.filter((asset) => {
      const state = thumbs[asset.id];
      return state?.previewChecked && (state?.previewStatus === "ready" || state?.previewStatus === "unavailable");
    }).length;

    const previous = lastProgressLogRef.current;
    const changed =
      thumbsCompleted !== previous.thumbsCompleted ||
      thumbsTotal !== previous.thumbsTotal ||
      previewsCompleted !== previous.previewsCompleted ||
      previewsTotal !== previous.previewsTotal;
    if (!changed) {
      return;
    }

    lastProgressLogRef.current = {
      thumbsCompleted,
      thumbsTotal,
      previewsCompleted,
      previewsTotal,
    };

    if (
      (thumbsCompleted === thumbsTotal && previewsCompleted === previewsTotal) ||
      previous.thumbsCompleted < 0 ||
      thumbsCompleted === 0 ||
      previewsCompleted === 0 ||
      thumbsCompleted - previous.thumbsCompleted >= 24 ||
      previewsCompleted - previous.previewsCompleted >= 24 ||
      thumbsTotal !== previous.thumbsTotal ||
      previewsTotal !== previous.previewsTotal
    ) {
      void logClient(
        "grid",
        `media preload progress thumbs=${thumbsCompleted}/${thumbsTotal} previews=${previewsCompleted}/${previewsTotal}`,
      );
    }

    onThumbnailPreloadProgress?.({
      thumbsCompleted,
      thumbsTotal,
      previewsCompleted,
      previewsTotal,
    });
  }, [assets, onThumbnailPreloadProgress, thumbnailPreload?.active, thumbs]);

  return (
    <div
      className="grid-scroller"
      ref={parentRef}
      onScroll={(event) => {
        const scrollTop = event.currentTarget.scrollTop;
        setViewport((current) => current.scrollTop === scrollTop ? current : { ...current, scrollTop });
      }}
    >
      {slotCount === 0 ? <div className="empty-state">No indexed assets match the current view.</div> : null}
      <div
        className="media-grid"
        style={{
          gridTemplateColumns: `repeat(${columns}, ${GRID_TILE_WIDTH}px)`,
          gridAutoRows: GRID_ROW_HEIGHT,
          paddingTop,
          paddingBottom,
        }}
      >
        {entries.map(({ index, asset }) => {
          if (!asset) {
            return (
              <div key={index} className="tile tile-placeholder" aria-hidden="true">
                <div className="thumb thumb-placeholder" />
                <div className="tile-body tile-body-placeholder">
                  <div className="placeholder-line placeholder-line-title" />
                  <div className="placeholder-line placeholder-line-meta" />
                </div>
              </div>
            );
          }

          return (
            <button
              key={asset.id}
              className={[
                "tile",
                thumbs[asset.id]?.previewChecked ? "preview-state-known" : "preview-state-unknown",
                thumbs[asset.id]?.previewStatus === "pending" ? "preview-state-pending" : "",
                thumbs[asset.id]?.previewChecked &&
                thumbs[asset.id]?.previewStatus !== "ready" &&
                thumbs[asset.id]?.previewStatus !== "pending"
                  ? "preview-state-known-missing"
                  : "",
                thumbs[asset.id]?.previewStatus === "ready" ? "has-viewer-preview" : "",
                asset.media_kind === "video" && videoPlaybackHints[asset.id] === "native"
                  ? "video-ready-native"
                  : "",
                asset.media_kind === "video" && videoPlaybackHints[asset.id] === "transcoded"
                  ? "video-ready-transcoded"
                  : "",
              ]
                .filter(Boolean)
                .join(" ")}
              type="button"
              data-asset-id={asset.id}
              onClick={() => onSelect(asset.id, index)}
            >
              <div
                className={[
                  "thumb",
                  thumbs[asset.id]?.thumbChecked ? "thumb-state-known" : "thumb-state-unknown",
                  thumbs[asset.id]?.previewChecked ? "preview-state-known" : "preview-state-unknown",
                  thumbs[asset.id]?.previewStatus === "ready" ? "has-viewer-preview" : "",
                  asset.media_kind === "video" && videoPlaybackHints[asset.id] === "native"
                    ? "video-ready-native"
                    : "",
                  asset.media_kind === "video" && videoPlaybackHints[asset.id] === "transcoded"
                    ? "video-ready-transcoded"
                    : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
              >
                {thumbs[asset.id]?.status === "ready" ? (
                  <img src={thumbs[asset.id]?.src ?? ""} alt={asset.title ?? "asset"} />
                ) : (
                  <div>{thumbStatusLabel(asset, thumbs[asset.id])}</div>
                )}
                {asset.media_kind === "video" ? (
                  <>
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
                  <span className="thumb-player-button" aria-hidden="true">
                    <span className="thumb-player-icon">▶</span>
                  </span>
                ) : null}
              </div>
              <div className="tile-body">
                <strong>{asset.title ?? "Untitled asset"}</strong>
                <div className="muted">
                  {asset.taken_at_utc ? dayjs(asset.taken_at_utc).format("YYYY-MM-DD HH:mm") : "Unknown date"}
                </div>
                {viewMode === "timeline" && asset.albums.length > 0 ? (
                  <div className="chips">
                    {asset.albums.slice(0, 2).map((album) => (
                      <span className="chip" key={album}>
                        {album}
                      </span>
                    ))}
                  </div>
                ) : null}
              </div>
            </button>
          );
        })}
      </div>
      {hasMoreAfter || isLoadingMore ? (
        <div className="grid-load-more" ref={loadMoreRef} aria-live="polite">
          {isLoadingMore ? "Loading more media..." : "Scroll to load more"}
        </div>
      ) : null}
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
