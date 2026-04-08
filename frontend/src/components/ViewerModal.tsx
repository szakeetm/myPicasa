import { useEffect, useMemo, useRef, useState } from "react";

import dayjs from "dayjs";
import { convertFileSrc } from "@tauri-apps/api/core";

import { api } from "../lib/tauri";
import type { AssetDetail } from "../lib/types";

type ViewerModalProps = {
  asset?: AssetDetail;
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  onClose: () => void;
};

export function ViewerModal({ asset, hasPrevious, hasNext, onPrevious, onNext, onClose }: ViewerModalProps) {
  const [imageSrc, setImageSrc] = useState<string>();
  const [imageError, setImageError] = useState<string>();
  const [videoSrc, setVideoSrc] = useState<string>();
  const [videoError, setVideoError] = useState<string>();
  const [videoFallbackAttempted, setVideoFallbackAttempted] = useState(false);
  const [livePhotoMotionSrc, setLivePhotoMotionSrc] = useState<string>();
  const [livePhotoMotionError, setLivePhotoMotionError] = useState<string>();
  const [livePhotoFallbackAttempted, setLivePhotoFallbackAttempted] = useState(false);
  const [showLivePhotoMotion, setShowLivePhotoMotion] = useState(false);
  const [forceRenderedFrame, setForceRenderedFrame] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [zoomMode, setZoomMode] = useState<"fit" | "custom">("fit");
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number }>();
  const [viewportSize, setViewportSize] = useState<{ width: number; height: number }>();
  const viewerMediaRef = useRef<HTMLDivElement | null>(null);
  const imageFrameRef = useRef<HTMLDivElement | null>(null);
  const assetId = asset?.id;
  const isPhoto = asset && asset.media_kind !== "video";
  const isVideo = asset?.media_kind === "video";
  const primaryPath = asset?.primary_path ? convertFileSrc(asset.primary_path) : undefined;
  const livePhotoPath = asset?.live_photo_video_path
    ? convertFileSrc(asset.live_photo_video_path)
    : undefined;
  const canUsePrimaryImageDirectly = isDirectImagePath(asset?.primary_path);
  const shouldPreferBackendVideo = shouldUseBackendVideo(asset?.primary_path);
  const shouldPreferBackendLivePhoto = shouldUseBackendVideo(asset?.live_photo_video_path);

  useEffect(() => {
    setForceRenderedFrame(false);
    setVideoFallbackAttempted(false);
    setLivePhotoFallbackAttempted(false);
    setShowLivePhotoMotion(false);
    setVideoSrc(undefined);
    setVideoError(undefined);
    setLivePhotoMotionSrc(undefined);
    setLivePhotoMotionError(undefined);
    setZoomMode("fit");
    setZoom(1);
    setNaturalSize(undefined);
    setImageError(undefined);
    setImageSrc(undefined);
  }, [assetId]);

  useEffect(() => {
    const element = viewerMediaRef.current;
    if (!element) return;
    const publishSize = () => {
      setViewportSize({
        width: Math.max(0, element.clientWidth - 24),
        height: Math.max(0, element.clientHeight - 24),
      });
    };
    publishSize();
    const observer = new ResizeObserver(publishSize);
    observer.observe(element);
    return () => observer.disconnect();
  }, [assetId, imageSrc]);

  const fitZoom = useMemo(() => {
    if (!naturalSize || !viewportSize?.width || !viewportSize?.height) {
      return 1;
    }
    return Math.min(
      viewportSize.width / naturalSize.width,
      viewportSize.height / naturalSize.height,
    );
  }, [naturalSize, viewportSize]);

  const effectiveZoom = isPhoto
    ? zoomMode === "fit"
      ? fitZoom
      : zoom
    : 1;
  const isScrollable =
    !!naturalSize &&
    !!viewportSize &&
    (naturalSize.width * effectiveZoom > viewportSize.width ||
      naturalSize.height * effectiveZoom > viewportSize.height);

  function setActualSize() {
    setZoomMode("custom");
    setZoom(1);
  }

  function setFitMode() {
    setZoomMode("fit");
  }

  function adjustZoom(delta: number) {
    setZoomMode("custom");
    setZoom((current) => {
      const base = zoomMode === "fit" ? fitZoom : current;
      return Math.min(Math.max(base + delta, 0.1), 8);
    });
  }

  useEffect(() => {
    let cancelled = false;
    if (!assetId || !isVideo) {
      setVideoSrc(undefined);
      setVideoError(undefined);
      setVideoFallbackAttempted(false);
      return;
    }

    setVideoError(undefined);
    if (shouldPreferBackendVideo) {
      setVideoSrc(undefined);
      setVideoFallbackAttempted(true);
      void api
        .loadViewerVideo(assetId)
        .then((backendPath) => {
          if (cancelled) return;
          if (backendPath) {
            setVideoSrc(convertFileSrc(backendPath));
            setVideoError(undefined);
          } else {
            setVideoError("Video playback unavailable");
          }
        })
        .catch((error) => {
          if (!cancelled) {
            setVideoError(String(error));
          }
        });
    } else {
      setVideoSrc(primaryPath);
      setVideoFallbackAttempted(false);
    }

    return () => {
      cancelled = true;
    };
  }, [assetId, isVideo, primaryPath, shouldPreferBackendVideo]);

  useEffect(() => {
    let cancelled = false;
    if (!assetId || !livePhotoPath) {
      setLivePhotoMotionSrc(undefined);
      setLivePhotoMotionError(undefined);
      setLivePhotoFallbackAttempted(false);
      return;
    }

    setLivePhotoMotionError(undefined);
    if (shouldPreferBackendLivePhoto) {
      setLivePhotoMotionSrc(undefined);
      setLivePhotoFallbackAttempted(true);
      void api
        .loadLivePhotoMotion(assetId)
        .then((backendPath) => {
          if (cancelled) return;
          if (backendPath) {
            setLivePhotoMotionSrc(convertFileSrc(backendPath));
            setLivePhotoMotionError(undefined);
          } else {
            setLivePhotoMotionError("Live photo playback unavailable");
          }
        })
        .catch((error) => {
          if (!cancelled) {
            setLivePhotoMotionError(String(error));
          }
        });
    } else {
      setLivePhotoMotionSrc(livePhotoPath);
      setLivePhotoFallbackAttempted(false);
    }

    return () => {
      cancelled = true;
    };
  }, [assetId, livePhotoPath, shouldPreferBackendLivePhoto]);

  useEffect(() => {
    let cancelled = false;
    if (!assetId || !isPhoto) {
      setImageSrc(undefined);
      setImageError(undefined);
      setNaturalSize(undefined);
      return;
    }
    if (!forceRenderedFrame && canUsePrimaryImageDirectly && primaryPath) {
      setImageSrc(primaryPath);
      setImageError(undefined);
      return;
    }

    setImageSrc(undefined);
    setImageError(undefined);

    void api
      .loadViewerFrame(assetId)
      .then((src) => {
        if (cancelled) return;
        if (src) {
          setImageSrc(src.startsWith("data:") ? src : convertFileSrc(src));
          setImageError(undefined);
        } else {
          setImageSrc(undefined);
          setImageError("Image preview unavailable");
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setImageSrc(undefined);
          setImageError(String(error));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [assetId, canUsePrimaryImageDirectly, forceRenderedFrame, isPhoto, primaryPath]);

  async function fallbackVideoToBackend() {
    if (!assetId || videoFallbackAttempted) {
      return;
    }
    setVideoFallbackAttempted(true);
    setVideoError(undefined);
    try {
      const backendPath = await api.loadViewerVideo(assetId);
      if (backendPath) {
        setVideoSrc(convertFileSrc(backendPath));
        return;
      }
      setVideoError("Video playback unavailable");
    } catch (error) {
      setVideoError(String(error));
    }
  }

  async function fallbackLivePhotoToBackend() {
    if (!assetId || livePhotoFallbackAttempted) {
      return;
    }
    setLivePhotoFallbackAttempted(true);
    setLivePhotoMotionError(undefined);
    try {
      const backendPath = await api.loadLivePhotoMotion(assetId);
      if (backendPath) {
        setLivePhotoMotionSrc(convertFileSrc(backendPath));
        return;
      }
      setLivePhotoMotionError("Live photo playback unavailable");
    } catch (error) {
      setLivePhotoMotionError(String(error));
    }
  }

  useEffect(() => {
    if (!asset) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "ArrowLeft" && hasPrevious) {
        event.preventDefault();
        onPrevious();
      } else if (event.key === "ArrowRight" && hasNext) {
        event.preventDefault();
        onNext();
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if ((event.key === "+" || event.key === "=") && isPhoto) {
        event.preventDefault();
        adjustZoom(0.25);
      } else if (event.key === "-" && isPhoto) {
        event.preventDefault();
        adjustZoom(-0.25);
      } else if (event.key === "0" && isPhoto) {
        event.preventDefault();
        setActualSize();
      } else if ((event.key === "f" || event.key === "F") && isPhoto) {
        event.preventDefault();
        setFitMode();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [asset, fitZoom, hasNext, hasPrevious, isPhoto, onClose, onNext, onPrevious, zoomMode]);

  if (!asset) return null;

  return (
    <div className="viewer-backdrop" onClick={onClose}>
      <div className="viewer-card" onClick={(event) => event.stopPropagation()}>
        <div className="viewer-toolbar">
          <strong>{asset.title ?? "Untitled asset"}</strong>
          <div className="button-row">
            {isPhoto ? (
              <>
                {livePhotoPath ? (
                  <button className="button-secondary" onClick={() => setShowLivePhotoMotion((value) => !value)}>
                    {showLivePhotoMotion ? "Show Photo" : "Play Live Photo"}
                  </button>
                ) : null}
                <button className="button-secondary" onClick={() => adjustZoom(-0.25)}>
                  Zoom -
                </button>
                <button className="button-secondary" onClick={setActualSize}>
                  100%
                </button>
                <button className="button-secondary" onClick={setFitMode}>
                  Fit
                </button>
                <button className="button-secondary" onClick={() => adjustZoom(0.25)}>
                  Zoom +
                </button>
              </>
            ) : null}
            <button className="button-secondary" onClick={onPrevious} disabled={!hasPrevious}>
              Previous
            </button>
            <button className="button-secondary" onClick={onNext} disabled={!hasNext}>
              Next
            </button>
            <button className="button-danger" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
        <div className="viewer-media" ref={viewerMediaRef}>
          {asset.media_kind === "video" ? (
            videoSrc ? (
              <video
                src={videoSrc}
                controls
                autoPlay
                onError={() => {
                  if (!videoFallbackAttempted) {
                    void fallbackVideoToBackend();
                    return;
                  }
                  setVideoError("Video playback unavailable");
                }}
              />
            ) : videoError ? (
              <div className="muted">{videoError}</div>
            ) : (
              <div className="muted">Loading video…</div>
            )
          ) : showLivePhotoMotion ? (
            livePhotoMotionSrc ? (
              <video
                src={livePhotoMotionSrc}
                controls
                autoPlay
                muted
                loop
                onError={() => {
                  if (!livePhotoFallbackAttempted) {
                    void fallbackLivePhotoToBackend();
                    return;
                  }
                  setLivePhotoMotionError("Live photo playback unavailable");
                }}
              />
            ) : livePhotoMotionError ? (
              <div className="muted">{livePhotoMotionError}</div>
            ) : (
              <div className="muted">Loading live photo…</div>
            )
          ) : imageSrc && assetId === asset.id ? (
            <div
              ref={imageFrameRef}
              className={`viewer-image-frame${isScrollable ? " zoomed" : ""}`}
            >
              {livePhotoPath ? (
                <button
                  className="viewer-live-photo-button"
                  type="button"
                  onClick={() => setShowLivePhotoMotion(true)}
                >
                  Play Live Photo
                </button>
              ) : null}
              <img
                src={imageSrc}
                alt={asset.title ?? "asset"}
                className={livePhotoPath ? "viewer-live-photo-still" : undefined}
                style={
                  naturalSize
                    ? {
                        width: `${Math.max(1, Math.round(naturalSize.width * effectiveZoom))}px`,
                        height: `${Math.max(1, Math.round(naturalSize.height * effectiveZoom))}px`,
                      }
                    : undefined
                }
                onLoad={(event) => {
                  setNaturalSize({
                    width: event.currentTarget.naturalWidth,
                    height: event.currentTarget.naturalHeight,
                  });
                }}
                onClick={() => {
                  if (livePhotoPath) {
                    setShowLivePhotoMotion(true);
                  }
                }}
                onError={() => {
                  if (!forceRenderedFrame && canUsePrimaryImageDirectly) {
                    setForceRenderedFrame(true);
                    return;
                  }
                  setImageSrc(undefined);
                  setImageError("Image preview unavailable");
                }}
              />
            </div>
          ) : imageError ? (
            <div className="muted">{imageError}</div>
          ) : (
            <div className="muted">Loading image…</div>
          )}
        </div>
        <div className="viewer-meta">
          <p className="muted">
            {asset.taken_at_utc ? dayjs(asset.taken_at_utc).format("YYYY-MM-DD HH:mm:ss") : "Unknown capture time"}
            {isPhoto ? ` • zoom ${Math.round(effectiveZoom * 100)}%` : ""}
            {naturalSize ? ` • ${naturalSize.width}x${naturalSize.height}` : ""}
            {asset.file_size ? ` • ${formatFileSize(asset.file_size)}` : ""}
          </p>
          <div className="chips">
            <span className="chip">{asset.media_kind}</span>
            <span className="chip">{asset.display_type}</span>
            {asset.albums.map((album) => (
              <span className="chip" key={album}>
                {album}
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function isDirectImagePath(path?: string | null) {
  const extension = path?.split(".").pop()?.toLowerCase();
  return extension !== undefined && ["jpg", "jpeg", "png", "webp", "gif"].includes(extension);
}

function shouldUseBackendVideo(path?: string | null) {
  const extension = path?.split(".").pop()?.toLowerCase();
  return extension !== undefined && !["mp4", "m4v", "webm"].includes(extension);
}

function formatFileSize(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value >= 100 ? 0 : value >= 10 ? 1 : 2)} ${units[unitIndex]}`;
}
