import { startTransition, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import dayjs from "dayjs";

import { api } from "../lib/tauri";
import { logClient } from "../lib/logger";
import { materializeImageSrc } from "../lib/mediaSrc";
import type { AssetListItem, ViewerPlaybackHint, ViewerPlaybackSupport } from "../lib/types";

const GRID_TILE_WIDTH = 210;
const GRID_GAP = 6;
const GRID_PADDING = 6;
const GRID_THUMBNAIL_SIZE = 210;
const THUMB_PROBE_BATCH_SIZE = 96;
const THUMB_ENQUEUE_BATCH_SIZE = 24;
const PREVIEW_PROBE_BATCH_SIZE = 96;
const PREVIEW_ENQUEUE_BATCH_SIZE = 24;

type MediaGridProps = {
  assets: AssetListItem[];
  viewMode: "timeline" | "album";
  entries: Array<
    | {
        kind: "asset";
        asset: AssetListItem;
      }
    | {
        kind: "placeholder";
        key: string;
        pageStartCursor: number;
      hydrationObserverKey: string | null;
      }
  >;
  onSelect: (assetId: number) => void;
  onHydratePlaceholderPage?: (pageStartCursor: number) => void;
  viewerPreviewReadyAssetIds?: number[];
  onLeadingDateChange?: (value?: string) => void;
  thumbnailResetKey?: number;
  hasMoreBefore?: boolean;
  hasMoreAfter?: boolean;
  isLoadingMoreBefore?: boolean;
  isLoadingMore?: boolean;
  onLoadMoreBefore?: () => void;
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
  entries,
  onSelect,
  onHydratePlaceholderPage,
  viewerPreviewReadyAssetIds = [],
  onLeadingDateChange,
  thumbnailResetKey,
  hasMoreBefore = false,
  hasMoreAfter = false,
  isLoadingMoreBefore = false,
  isLoadingMore = false,
  onLoadMoreBefore,
  onLoadMore,
  thumbnailPreload,
  viewerPreviewSize,
  onThumbnailPreloadProgress,
  viewerPlaybackSupport,
}: MediaGridProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const loadPreviousRef = useRef<HTMLDivElement | null>(null);
  const loadMoreRef = useRef<HTMLDivElement | null>(null);
  const tileRefs = useRef(new Map<number, HTMLButtonElement>());
  const placeholderRefs = useRef(new Map<string, HTMLDivElement>());
  const prependAnchorRef = useRef<{ assetId: number; top: number } | null>(null);
  const [width, setWidth] = useState(1200);
  const [thumbs, setThumbs] = useState<Record<number, ThumbnailState>>({});
  const [videoPlaybackHints, setVideoPlaybackHints] = useState<Record<number, ViewerPlaybackHint["status"]>>({});
  const [visibleIds, setVisibleIds] = useState<number[]>([]);
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
  const columns = columnCount(width);
  const visibleIdSet = useMemo(() => new Set(visibleIds), [visibleIds]);
  const viewerPreviewReadySet = useMemo(
    () => new Set(viewerPreviewReadyAssetIds),
    [viewerPreviewReadyAssetIds],
  );
  const bootstrapVisibleIds = useMemo(
    () => assets.slice(0, Math.max(columns * 3, 12)).map((asset) => asset.id),
    [assets, columns],
  );
  const effectiveVisibleIdSet = useMemo(() => {
    if (visibleIdSet.size > 0) {
      return visibleIdSet;
    }
    return new Set(bootstrapVisibleIds);
  }, [bootstrapVisibleIds, visibleIdSet]);
  const thumbnailSize = useMemo(() => GRID_THUMBNAIL_SIZE, []);

  useEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const root = parentRef.current;
    if (root) {
      root.scrollTop = 0;
    }
    prependAnchorRef.current = null;
    setThumbs({});
    setVisibleIds([]);
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
      setVisibleIds((current) => current.filter((assetId) => assetIds.has(assetId)));
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

  function logIdleSnapshot(kind: "thumb" | "preview", reason: string) {
    if (!thumbnailPreload?.active || assets.length === 0) {
      return;
    }

    const thumbSummary = summarizeThumbStates();
    const previewSummary = summarizePreviewStates();
    const signature = `${reason}|t:${thumbSummary.ready}/${thumbSummary.unavailable}/${thumbSummary.pending}/${thumbSummary.missing}/${thumbSummary.unknown}|p:${previewSummary.ready}/${previewSummary.unavailable}/${previewSummary.pending}/${previewSummary.waitingForThumb}/${previewSummary.missing}/${previewSummary.unknown}|v:${visibleIds.length}|a:${assets.length}`;
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
      `${kind} preload idle reason=${reason} visible=${visibleIds.length}/${assets.length} thumbs ready=${thumbSummary.ready} unavailable=${thumbSummary.unavailable} pending=${thumbSummary.pending} missing=${thumbSummary.missing} unknown=${thumbSummary.unknown} previews ready=${previewSummary.ready} unavailable=${previewSummary.unavailable} pending=${previewSummary.pending} waiting_for_thumb=${previewSummary.waitingForThumb} missing=${previewSummary.missing} unknown=${previewSummary.unknown}`,
    );
  }

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
    if (!isLoadingMoreBefore) {
      return;
    }

    const root = parentRef.current;
    if (!root) {
      return;
    }

    const anchorAsset = assets.find((asset) => effectiveVisibleIdSet.has(asset.id)) ?? assets[0];
    if (!anchorAsset) {
      prependAnchorRef.current = null;
      return;
    }

    const element = tileRefs.current.get(anchorAsset.id);
    if (!element) {
      prependAnchorRef.current = null;
      return;
    }

    prependAnchorRef.current = {
      assetId: anchorAsset.id,
      top: element.getBoundingClientRect().top - root.getBoundingClientRect().top,
    };
  }, [assets, effectiveVisibleIdSet, isLoadingMoreBefore]);

  useLayoutEffect(() => {
    if (isLoadingMoreBefore) {
      return;
    }

    const anchor = prependAnchorRef.current;
    const root = parentRef.current;
    if (!anchor || !root) {
      return;
    }

    const element = tileRefs.current.get(anchor.assetId);
    if (!element) {
      prependAnchorRef.current = null;
      return;
    }

    const nextTop = element.getBoundingClientRect().top - root.getBoundingClientRect().top;
    root.scrollTop += nextTop - anchor.top;
    prependAnchorRef.current = null;
  }, [assets, isLoadingMoreBefore]);

  useEffect(() => {
    const root = parentRef.current;
    const sentinel = loadPreviousRef.current;
    if (!root || !sentinel || !hasMoreBefore) {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting) && !isLoadingMoreBefore) {
          onLoadMoreBefore?.();
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
  }, [hasMoreBefore, isLoadingMoreBefore, onLoadMoreBefore, assets.length]);

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
    const root = parentRef.current;
    if (!root || !onHydratePlaceholderPage) {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        const pendingPageStarts = new Set<number>();
        for (const entry of entries) {
          if (!entry.isIntersecting) {
            continue;
          }
          const pageStartCursor = Number((entry.target as HTMLElement).dataset.pageStartCursor);
          if (Number.isFinite(pageStartCursor)) {
            pendingPageStarts.add(pageStartCursor);
          }
        }

        for (const pageStartCursor of pendingPageStarts) {
          onHydratePlaceholderPage(pageStartCursor);
        }
      },
      {
        root,
        rootMargin: "500px 0px",
        threshold: 0.01,
      },
    );

    for (const element of placeholderRefs.current.values()) {
      observer.observe(element);
    }

    return () => observer.disconnect();
  }, [entries, onHydratePlaceholderPage]);

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

      const preloadPendingIds: number[] = [];
      const requestIds = [...new Set([...probedMissingIds, ...visiblePendingIds])].slice(0, THUMB_ENQUEUE_BATCH_SIZE);
      if (requestIds.length === 0) {
        logIdleSnapshot("thumb", "no_thumb_targets");
        return;
      }

      if (thumbGenerateInFlightRef.current) {
        logIdleSnapshot("thumb", "generation_in_flight");
        return;
      }

      void logClient(
        "grid",
        `thumb request visible=${visiblePendingIds.join(",") || "none"} preload=none requested=${requestIds.join(",")}`,
      );

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
    const firstVisibleAsset =
      assets.find((asset) => effectiveVisibleIdSet.has(asset.id)) ?? assets[0];
    onLeadingDateChange?.(firstVisibleAsset?.taken_at_utc ?? undefined);
  }, [assets, effectiveVisibleIdSet, onLeadingDateChange]);

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

  if (entries.length === 0) {
    return <div className="empty-state">No indexed assets match the current view.</div>;
  }

  return (
    <div className="grid-scroller" ref={parentRef}>
      {hasMoreBefore || isLoadingMoreBefore ? (
        <div className="grid-load-more grid-load-more-top" ref={loadPreviousRef} aria-live="polite">
          {isLoadingMoreBefore ? "Loading earlier media..." : "Scroll up to load earlier media"}
        </div>
      ) : null}
      <div
        className="media-grid"
        style={{
          gridTemplateColumns: `repeat(auto-fit, ${GRID_TILE_WIDTH}px)`,
        }}
      >
        {entries.map((entry) => {
          if (entry.kind === "placeholder") {
            return (
              <div
                key={entry.key}
                className="tile tile-placeholder"
                aria-hidden="true"
                data-page-start-cursor={entry.hydrationObserverKey ? entry.pageStartCursor : undefined}
                ref={(element) => {
                  if (!entry.hydrationObserverKey) {
                    return;
                  }
                  if (element) {
                    placeholderRefs.current.set(entry.hydrationObserverKey, element);
                  } else {
                    placeholderRefs.current.delete(entry.hydrationObserverKey);
                  }
                }}
              >
                <div className="thumb thumb-placeholder" />
                <div className="tile-body tile-body-placeholder">
                  <div className="placeholder-line placeholder-line-title" />
                  <div className="placeholder-line placeholder-line-meta" />
                </div>
              </div>
            );
          }

          const asset = entry.asset;
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
