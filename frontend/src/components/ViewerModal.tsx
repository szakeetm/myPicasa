import { useEffect, useMemo, useRef, useState } from "react";

import dayjs from "dayjs";
import { convertFileSrc } from "@tauri-apps/api/core";

import { logClient } from "../lib/logger";
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
  const [videoFallbackAvailable, setVideoFallbackAvailable] = useState(false);
  const [videoTranscoding, setVideoTranscoding] = useState(false);
  const [livePhotoMotionSrc, setLivePhotoMotionSrc] = useState<string>();
  const [livePhotoMotionError, setLivePhotoMotionError] = useState<string>();
  const [livePhotoFallbackAttempted, setLivePhotoFallbackAttempted] = useState(false);
  const [livePhotoFallbackAvailable, setLivePhotoFallbackAvailable] = useState(false);
  const [livePhotoTranscoding, setLivePhotoTranscoding] = useState(false);
  const [showLivePhotoMotion, setShowLivePhotoMotion] = useState(false);
  const [forceRenderedFrame, setForceRenderedFrame] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [zoomMode, setZoomMode] = useState<"fit" | "custom">("fit");
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number }>();
  const [viewportSize, setViewportSize] = useState<{ width: number; height: number }>();
  const viewerMediaRef = useRef<HTMLDivElement | null>(null);
  const imageFrameRef = useRef<HTMLDivElement | null>(null);
  const videoElementRef = useRef<HTMLVideoElement | null>(null);
  const livePhotoVideoElementRef = useRef<HTMLVideoElement | null>(null);
  const videoObjectUrlRef = useRef<string | undefined>(undefined);
  const livePhotoObjectUrlRef = useRef<string | undefined>(undefined);
  const videoSourceLabelRef = useRef<string>("unset");
  const livePhotoSourceLabelRef = useRef<string>("unset");
  const assetId = asset?.id;
  const isPhoto = asset && asset.media_kind !== "video";
  const isVideo = asset?.media_kind === "video";
  const canFullscreenVideo =
    isVideo &&
    typeof document !== "undefined" &&
    typeof document.fullscreenEnabled !== "undefined";
  const primaryPath = asset?.primary_path ? convertFileSrc(asset.primary_path) : undefined;
  const livePhotoPath = asset?.live_photo_video_path
    ? convertFileSrc(asset.live_photo_video_path)
    : undefined;
  const canUsePrimaryImageDirectly = isDirectImagePath(asset?.primary_path);
  const shouldPreferOriginalVideoBytes = shouldPreferOriginalVideoBytesForPath(asset?.primary_path);
  const shouldPreferOriginalLivePhotoBytes = shouldPreferOriginalVideoBytesForPath(
    asset?.live_photo_video_path,
  );

  useEffect(() => {
    setForceRenderedFrame(false);
    setVideoFallbackAttempted(false);
    setLivePhotoFallbackAttempted(false);
    setShowLivePhotoMotion(false);
    setVideoSrc(undefined);
    setVideoError(undefined);
    setVideoTranscoding(false);
    setVideoFallbackAttempted(false);
    setVideoFallbackAvailable(false);
    setLivePhotoMotionSrc(undefined);
    setLivePhotoMotionError(undefined);
    setLivePhotoTranscoding(false);
    setLivePhotoFallbackAttempted(false);
    setLivePhotoFallbackAvailable(false);
    setZoomMode("fit");
    setZoom(1);
    setNaturalSize(undefined);
    setImageError(undefined);
    setImageSrc(undefined);
    if (videoObjectUrlRef.current) {
      URL.revokeObjectURL(videoObjectUrlRef.current);
      videoObjectUrlRef.current = undefined;
    }
    if (livePhotoObjectUrlRef.current) {
      URL.revokeObjectURL(livePhotoObjectUrlRef.current);
      livePhotoObjectUrlRef.current = undefined;
    }
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
      setVideoTranscoding(false);
      return;
    }

    setVideoError(undefined);
    setVideoSrc(undefined);
    setVideoFallbackAttempted(false);
    setVideoFallbackAvailable(false);
    setVideoTranscoding(true);
    void logClient(
      "viewer.video",
      `viewer-v2 asset ${assetId} loading backend video for ${asset?.primary_path ?? "unknown"} (${describeVideoSupport(asset?.primary_path)} source=${shouldPreferOriginalVideoBytes ? "original_bytes" : "transcode"})`,
    );
    void api
      .loadViewerVideo(assetId, shouldPreferOriginalVideoBytes)
      .then(async (backendPath) => {
        if (cancelled) return;
        setVideoTranscoding(false);
        if (backendPath) {
          const sourceLabel =
            backendPath.startsWith("data:video/quicktime")
              ? "original_quicktime"
              : backendPath.startsWith("data:video/mp4") && shouldPreferOriginalVideoBytes
                ? "original_mp4"
                : "transcoded_mp4";
          void logClient(
            "viewer.video",
            `viewer-v2 asset ${assetId} backend video ready source=${sourceLabel}`,
          );
          videoSourceLabelRef.current = sourceLabel;
          const materialized = await materializeVideoSrc(backendPath, videoObjectUrlRef);
          if (cancelled) {
            if (materialized.startsWith("blob:")) {
              URL.revokeObjectURL(materialized);
            }
            return;
          }
          setVideoSrc(materialized);
          setVideoError(undefined);
        } else {
          void logClient("viewer.video", `asset ${assetId} backend video unavailable`, "error");
          setVideoError("Video playback unavailable");
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setVideoTranscoding(false);
          void logClient("viewer.video", `asset ${assetId} backend video failed: ${String(error)}`, "error");
          setVideoError(String(error));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [assetId, asset?.primary_path, isVideo, shouldPreferOriginalVideoBytes]);

  useEffect(() => {
    let cancelled = false;
    if (!assetId || !livePhotoPath) {
      setLivePhotoMotionSrc(undefined);
      setLivePhotoMotionError(undefined);
      setLivePhotoFallbackAttempted(false);
      setLivePhotoTranscoding(false);
      return;
    }

    setLivePhotoMotionError(undefined);
    setLivePhotoMotionSrc(undefined);
    setLivePhotoFallbackAttempted(false);
    setLivePhotoFallbackAvailable(false);
    setLivePhotoTranscoding(true);
    void logClient(
      "viewer.live_photo",
      `viewer-v2 asset ${assetId} loading backend motion for ${asset?.live_photo_video_path ?? "unknown"} (${describeVideoSupport(asset?.live_photo_video_path)} source=${shouldPreferOriginalLivePhotoBytes ? "original_bytes" : "transcode"})`,
    );
    void api
      .loadLivePhotoMotion(assetId, shouldPreferOriginalLivePhotoBytes)
      .then(async (backendPath) => {
        if (cancelled) return;
        setLivePhotoTranscoding(false);
        if (backendPath) {
          const sourceLabel =
            backendPath.startsWith("data:video/quicktime")
              ? "original_quicktime"
              : backendPath.startsWith("data:video/mp4") && shouldPreferOriginalLivePhotoBytes
                ? "original_mp4"
                : "transcoded_mp4";
          void logClient(
            "viewer.live_photo",
            `viewer-v2 asset ${assetId} backend motion ready source=${sourceLabel}`,
          );
          livePhotoSourceLabelRef.current = sourceLabel;
          const materialized = await materializeVideoSrc(backendPath, livePhotoObjectUrlRef);
          if (cancelled) {
            if (materialized.startsWith("blob:")) {
              URL.revokeObjectURL(materialized);
            }
            return;
          }
          setLivePhotoMotionSrc(materialized);
          setLivePhotoMotionError(undefined);
        } else {
          void logClient("viewer.live_photo", `asset ${assetId} backend motion unavailable`, "error");
          setLivePhotoMotionError("Live photo playback unavailable");
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setLivePhotoTranscoding(false);
          void logClient("viewer.live_photo", `asset ${assetId} backend motion failed: ${String(error)}`, "error");
          setLivePhotoMotionError(String(error));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [assetId, asset?.live_photo_video_path, livePhotoPath, shouldPreferOriginalLivePhotoBytes]);

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
    setVideoFallbackAvailable(false);
    setVideoError(undefined);
    setVideoTranscoding(true);
    void logClient("viewer.video", `asset ${assetId} switching to transcoded backend playback after backend-original error`);
    try {
      const backendPath = await api.loadViewerVideo(assetId, false);
      setVideoTranscoding(false);
      if (backendPath) {
        void logClient("viewer.video", `asset ${assetId} transcoded backend playback ready after backend-original error`);
        videoSourceLabelRef.current = "transcoded_mp4";
        setVideoSrc(await materializeVideoSrc(backendPath, videoObjectUrlRef));
        return;
      }
      void logClient("viewer.video", `asset ${assetId} transcoded backend playback unavailable after backend-original error`, "error");
      setVideoError("Video playback unavailable");
    } catch (error) {
      setVideoTranscoding(false);
      void logClient("viewer.video", `asset ${assetId} transcoded backend playback failed after backend-original error: ${String(error)}`, "error");
      setVideoError(String(error));
    }
  }

  async function fallbackLivePhotoToBackend() {
    if (!assetId || livePhotoFallbackAttempted) {
      return;
    }
    setLivePhotoFallbackAttempted(true);
    setLivePhotoFallbackAvailable(false);
    setLivePhotoMotionError(undefined);
    setLivePhotoTranscoding(true);
    void logClient("viewer.live_photo", `asset ${assetId} switching to transcoded backend motion after backend-original error`);
    try {
      const backendPath = await api.loadLivePhotoMotion(assetId, false);
      setLivePhotoTranscoding(false);
      if (backendPath) {
        void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion ready after backend-original error`);
        livePhotoSourceLabelRef.current = "transcoded_mp4";
        setLivePhotoMotionSrc(await materializeVideoSrc(backendPath, livePhotoObjectUrlRef));
        return;
      }
      void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion unavailable after backend-original error`, "error");
      setLivePhotoMotionError("Live photo playback unavailable");
    } catch (error) {
      setLivePhotoTranscoding(false);
      void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion failed after backend-original error: ${String(error)}`, "error");
      setLivePhotoMotionError(String(error));
    }
  }

  async function toggleVideoFullscreen() {
    const element = videoElementRef.current;
    if (!element || typeof document === "undefined") {
      return;
    }
    if (document.fullscreenElement === element) {
      await document.exitFullscreen();
      return;
    }
    await element.requestFullscreen();
  }

  function handleVideoDiagnostics(
    scope: "viewer.video" | "viewer.live_photo",
    label: string,
    event: React.SyntheticEvent<HTMLVideoElement>,
  ) {
    const element = event.currentTarget;
    const errorCode = element.error?.code;
    const errorName =
      errorCode === 1
        ? "aborted"
        : errorCode === 2
          ? "network"
          : errorCode === 3
            ? "decode"
            : errorCode === 4
              ? "src_not_supported"
              : "none";
    void logClient(
      scope,
      `${label} src="${summarizeMediaSrc(element.currentSrc)}" readyState=${element.readyState} networkState=${element.networkState} error=${errorName} source=${scope === "viewer.video" ? videoSourceLabelRef.current : livePhotoSourceLabelRef.current}`,
      errorCode ? "error" : "info",
    );
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
            {canFullscreenVideo ? (
              <button className="button-secondary" onClick={() => void toggleVideoFullscreen()}>
                Fullscreen
              </button>
            ) : null}
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
                key={videoSrc}
                ref={videoElementRef}
                controls
                autoPlay
                src={videoSrc}
                preload="metadata"
                onLoadedMetadata={(event) => handleVideoDiagnostics("viewer.video", "loadedmetadata", event)}
                onCanPlay={(event) => {
                  handleVideoDiagnostics("viewer.video", "canplay", event);
                  void event.currentTarget.play().catch((error) => {
                    void logClient("viewer.video", `asset ${assetId} play() rejected: ${String(error)}`, "error");
                  });
                }}
                onWaiting={(event) => handleVideoDiagnostics("viewer.video", "waiting", event)}
                onStalled={(event) => handleVideoDiagnostics("viewer.video", "stalled", event)}
                onPlay={() => {
                  void logClient("viewer.video", `asset ${assetId} video started playing`);
                }}
                onError={(event) => {
                  handleVideoDiagnostics("viewer.video", `asset ${assetId} video element error`, event);
                  if (!videoFallbackAttempted) {
                    void logClient(
                      "viewer.video",
                      `asset ${assetId} backend-original playback failed; waiting for manual transcode request`,
                      "error",
                    );
                    setVideoFallbackAvailable(true);
                    setVideoError("This video failed to decode natively in the viewer.");
                    return;
                  }
                  setVideoError("Video playback unavailable");
                }}
              />
            ) : videoError ? (
              <div className="viewer-loading-state">
                <div className="muted">{videoError}</div>
                {videoFallbackAvailable ? (
                  <button className="button-secondary" onClick={() => void fallbackVideoToBackend()}>
                    Transcode For Playback
                  </button>
                ) : null}
              </div>
            ) : videoTranscoding ? (
              <div className="viewer-loading-state">Transcoding video...</div>
            ) : (
              <div className="muted">Loading video…</div>
            )
          ) : showLivePhotoMotion ? (
            livePhotoMotionSrc ? (
              <video
                key={livePhotoMotionSrc}
                ref={livePhotoVideoElementRef}
                controls
                autoPlay
                muted
                loop
                src={livePhotoMotionSrc}
                preload="metadata"
                onLoadedMetadata={(event) =>
                  handleVideoDiagnostics("viewer.live_photo", "loadedmetadata", event)
                }
                onCanPlay={(event) => {
                  handleVideoDiagnostics("viewer.live_photo", "canplay", event);
                  void event.currentTarget.play().catch((error) => {
                    void logClient("viewer.live_photo", `asset ${assetId} live photo play() rejected: ${String(error)}`, "error");
                  });
                }}
                onWaiting={(event) =>
                  handleVideoDiagnostics("viewer.live_photo", "waiting", event)
                }
                onStalled={(event) =>
                  handleVideoDiagnostics("viewer.live_photo", "stalled", event)
                }
                onPlay={() => {
                  void logClient("viewer.live_photo", `asset ${assetId} live photo motion started playing`);
                }}
                onError={(event) => {
                  handleVideoDiagnostics(
                    "viewer.live_photo",
                    `asset ${assetId} live photo motion element error`,
                    event,
                  );
                  if (!livePhotoFallbackAttempted) {
                    void logClient(
                      "viewer.live_photo",
                      `asset ${assetId} backend-original motion failed; waiting for manual transcode request`,
                      "error",
                    );
                    setLivePhotoFallbackAvailable(true);
                    setLivePhotoMotionError("This live photo motion clip failed to decode natively in the viewer.");
                    return;
                  }
                  setLivePhotoMotionError("Live photo playback unavailable");
                }}
              />
            ) : livePhotoMotionError ? (
              <div className="viewer-loading-state">
                <div className="muted">{livePhotoMotionError}</div>
                {livePhotoFallbackAvailable ? (
                  <button className="button-secondary" onClick={() => void fallbackLivePhotoToBackend()}>
                    Transcode Live Photo
                  </button>
                ) : null}
              </div>
            ) : livePhotoTranscoding ? (
              <div className="viewer-loading-state">Transcoding live photo...</div>
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

function summarizeMediaSrc(src?: string) {
  if (!src) return "none";
  if (src.startsWith("data:video/")) {
    const format = src.slice("data:".length).split(";")[0] ?? "video";
    return `${format} data-url`;
  }
  return src;
}

async function materializeVideoSrc(src: string, objectUrlRef: React.MutableRefObject<string | undefined>) {
  if (objectUrlRef.current) {
    URL.revokeObjectURL(objectUrlRef.current);
    objectUrlRef.current = undefined;
  }
  if (!src.startsWith("data:video/")) {
    return src;
  }
  const response = await fetch(src);
  const blob = await response.blob();
  const objectUrl = URL.createObjectURL(blob);
  objectUrlRef.current = objectUrl;
  return objectUrl;
}

function describeVideoSupport(src?: string | null) {
  if (typeof document === "undefined") {
    return "video support unknown";
  }
  const probe = document.createElement("video");
  const requestedType = inferVideoMimeType(src) ?? "unknown";
  const mp4 = probe.canPlayType("video/mp4") || "no";
  const mov = probe.canPlayType("video/quicktime") || "no";
  const webm = probe.canPlayType("video/webm") || "no";
  return `requested=${requestedType} canPlayType(mp4=${mp4}, mov=${mov}, webm=${webm})`;
}

function isDirectImagePath(path?: string | null) {
  const extension = path?.split(".").pop()?.toLowerCase();
  return extension !== undefined && ["jpg", "jpeg", "png", "webp", "gif"].includes(extension);
}

function shouldPreferOriginalVideoBytesForPath(path?: string | null) {
  const extension = path?.split(".").pop()?.toLowerCase();
  return extension !== undefined && ["mp4", "m4v", "mov", "webm"].includes(extension);
}

function inferVideoMimeType(src?: string | null) {
  if (!src) return undefined;
  if (src.startsWith("data:video/")) {
    return src.slice("data:".length).split(";")[0];
  }
  const extension = src.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "mov":
      return "video/quicktime";
    case "webm":
      return "video/webm";
    case "m4v":
    case "mp4":
      return "video/mp4";
    default:
      return undefined;
  }
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
