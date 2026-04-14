import { useEffect, useRef, useState } from "react";

import dayjs from "dayjs";

import { logClient } from "../lib/logger";
import { materializeImageSrc, materializeVideoSrc } from "../lib/mediaSrc";
import { api } from "../lib/tauri";
import type { AssetDetail } from "../lib/types";

type ViewerModalProps = {
  asset?: AssetDetail;
  viewerPreviewSize: number;
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  onClose: () => void;
  onViewerPreviewReady?: (assetId: number) => void;
};

export function ViewerModal({
  asset,
  viewerPreviewSize,
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
  onClose,
  onViewerPreviewReady,
}: ViewerModalProps) {
  const isMacOs = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
  const isWindows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
  const [imageSrc, setImageSrc] = useState<string>();
  const [imageError, setImageError] = useState<string>();
  const [videoSrc, setVideoSrc] = useState<string>();
  const [videoError, setVideoError] = useState<string>();
  const [videoFallbackAttempted, setVideoFallbackAttempted] = useState(false);
  const [videoFallbackAvailable, setVideoFallbackAvailable] = useState(false);
  const [videoTranscoding, setVideoTranscoding] = useState(false);
  const [videoTranscodeStatus, setVideoTranscodeStatus] = useState<{
    codec?: string;
    encoder?: string;
    elapsedMs?: number;
    timeoutMs?: number;
    sourceBytes?: number;
    outputBytes?: number;
  }>({});
  const [livePhotoMotionSrc, setLivePhotoMotionSrc] = useState<string>();
  const [livePhotoMotionError, setLivePhotoMotionError] = useState<string>();
  const [livePhotoFallbackAttempted, setLivePhotoFallbackAttempted] = useState(false);
  const [livePhotoFallbackAvailable, setLivePhotoFallbackAvailable] = useState(false);
  const [livePhotoTranscoding, setLivePhotoTranscoding] = useState(false);
  const [livePhotoTranscodeStatus, setLivePhotoTranscodeStatus] = useState<{
    codec?: string;
    encoder?: string;
    elapsedMs?: number;
    timeoutMs?: number;
    sourceBytes?: number;
    outputBytes?: number;
  }>({});
  const [showLivePhotoMotion, setShowLivePhotoMotion] = useState(false);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number }>();
  const [displaySourceLabel, setDisplaySourceLabel] = useState<string>("Loading");
  const videoElementRef = useRef<HTMLVideoElement | null>(null);
  const livePhotoVideoElementRef = useRef<HTMLVideoElement | null>(null);
  const videoObjectUrlRef = useRef<string | undefined>(undefined);
  const livePhotoObjectUrlRef = useRef<string | undefined>(undefined);
  const videoSourceLabelRef = useRef<string>("unset");
  const livePhotoSourceLabelRef = useRef<string>("unset");
  const imageSourceLabelRef = useRef<string>("unset");
  const onViewerPreviewReadyRef = useRef<typeof onViewerPreviewReady>(onViewerPreviewReady);
  const assetId = asset?.id;
  const isPhoto = asset && asset.media_kind !== "video";
  const isVideo = asset?.media_kind === "video";
  const livePhotoPath = materializeImageSrc(asset?.live_photo_video_path) ?? undefined;
  const shouldPreferOriginalVideoBytes = shouldPreferOriginalVideoBytesForPath(asset?.primary_path);
  const shouldPreferOriginalLivePhotoBytes = shouldPreferOriginalVideoBytesForPath(
    asset?.live_photo_video_path,
  );
  const revealActionLabel = isWindows
    ? "Show In Explorer"
    : isMacOs
      ? "Show In Finder"
      : "Show In File Manager";
  const previewActionLabel = isWindows
    ? "Open Folder In Explorer"
    : isMacOs
      ? isVideo
        ? "Open With QuickTime"
        : "Open With Quick Look"
      : "Open Preview";
  const canonicalImageSize =
    asset?.width && asset?.height
      ? { width: asset.width, height: asset.height }
      : naturalSize;

  useEffect(() => {
    onViewerPreviewReadyRef.current = onViewerPreviewReady;
  }, [onViewerPreviewReady]);

  useEffect(() => {
    setVideoFallbackAttempted(false);
    setShowLivePhotoMotion(false);
    setVideoSrc(undefined);
    setVideoError(undefined);
    setVideoTranscoding(false);
    setVideoFallbackAvailable(false);
    setLivePhotoMotionSrc(undefined);
    setLivePhotoMotionError(undefined);
    setLivePhotoFallbackAttempted(false);
    setLivePhotoTranscoding(false);
    setLivePhotoFallbackAvailable(false);
    setVideoTranscodeStatus({});
    setLivePhotoTranscodeStatus({});
    setNaturalSize(undefined);
    setDisplaySourceLabel("Loading");
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

  function setImageSourceLabel(label: string) {
    imageSourceLabelRef.current = label;
    if (!showLivePhotoMotion && isPhoto) {
      setDisplaySourceLabel(label);
    }
  }

  function setVideoSourceLabel(label: string) {
    videoSourceLabelRef.current = label;
    if (isVideo) {
      setDisplaySourceLabel(label);
    }
  }

  function setLivePhotoSourceLabel(label: string) {
    livePhotoSourceLabelRef.current = label;
    if (showLivePhotoMotion) {
      setDisplaySourceLabel(label);
    }
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
    setVideoTranscodeStatus({});
    setVideoSourceLabel("transcoding");
    void logClient(
      "viewer.video",
      `viewer-v2 asset ${assetId} loading backend video for ${asset?.primary_path ?? "unknown"} (${describeVideoSupport(asset?.primary_path)} source=${shouldPreferOriginalVideoBytes ? "original_bytes" : "transcode"})`,
    );
    void (async () => {
      let loggedPending = false;
      try {
        while (!cancelled) {
          const backendMedia = await api.loadViewerVideo(assetId, shouldPreferOriginalVideoBytes);
          if (cancelled) return;
          if (backendMedia.status === "ready" && backendMedia.src && backendMedia.source) {
            setVideoTranscoding(false);
            setVideoTranscodeStatus({});
            void logClient(
              "viewer.video",
              `viewer-v2 asset ${assetId} backend video ready source=${backendMedia.source}`,
            );
            setVideoSourceLabel(backendMedia.source);
            const materialized = await materializeVideoSrc(backendMedia.src, videoObjectUrlRef);
            if (cancelled) {
              if (materialized.startsWith("blob:")) {
                URL.revokeObjectURL(materialized);
              }
              return;
            }
            setVideoSrc(materialized);
            setVideoError(undefined);
            return;
          }
          if (backendMedia.status === "pending") {
            setVideoTranscoding(true);
            setVideoError(undefined);
            setVideoSourceLabel(backendMedia.source ?? "transcoding");
            setVideoTranscodeStatus({
              codec: backendMedia.codec ?? undefined,
              encoder: backendMedia.encoder ?? undefined,
              elapsedMs: backendMedia.elapsed_ms ?? undefined,
              timeoutMs: backendMedia.timeout_ms ?? undefined,
              sourceBytes: backendMedia.source_bytes ?? undefined,
              outputBytes: backendMedia.output_bytes ?? undefined,
            });
            if (!loggedPending) {
              loggedPending = true;
              void logClient(
                "viewer.video",
                `viewer-v2 asset ${assetId} background video transcode in progress`,
              );
            }
            await new Promise((resolve) => window.setTimeout(resolve, 750));
            continue;
          }

          setVideoTranscoding(false);
          setVideoTranscodeStatus({
            codec: backendMedia.codec ?? undefined,
            encoder: backendMedia.encoder ?? undefined,
            elapsedMs: backendMedia.elapsed_ms ?? undefined,
            timeoutMs: backendMedia.timeout_ms ?? undefined,
            sourceBytes: backendMedia.source_bytes ?? undefined,
            outputBytes: backendMedia.output_bytes ?? undefined,
          });
          void logClient(
            "viewer.video",
            `asset ${assetId} backend video unavailable: ${backendMedia.message ?? "unavailable"}`,
            "error",
          );
          setVideoError(backendMedia.message ?? "Video playback unavailable");
          return;
        }
      } catch (error) {
        if (!cancelled) {
          setVideoTranscoding(false);
          setVideoTranscodeStatus({});
          void logClient("viewer.video", `asset ${assetId} backend video failed: ${String(error)}`, "error");
          setVideoError(String(error));
        }
      }
    })();

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
    setLivePhotoTranscodeStatus({});
    setLivePhotoSourceLabel("transcoding");
    void logClient(
      "viewer.live_photo",
      `viewer-v2 asset ${assetId} loading backend motion for ${asset?.live_photo_video_path ?? "unknown"} (${describeVideoSupport(asset?.live_photo_video_path)} source=${shouldPreferOriginalLivePhotoBytes ? "original_bytes" : "transcode"})`,
    );
    void (async () => {
      let loggedPending = false;
      try {
        while (!cancelled) {
          const backendMedia = await api.loadLivePhotoMotion(assetId, shouldPreferOriginalLivePhotoBytes);
          if (cancelled) return;
          if (backendMedia.status === "ready" && backendMedia.src && backendMedia.source) {
            setLivePhotoTranscoding(false);
            setLivePhotoTranscodeStatus({});
            void logClient(
              "viewer.live_photo",
              `viewer-v2 asset ${assetId} backend motion ready source=${backendMedia.source}`,
            );
            setLivePhotoSourceLabel(backendMedia.source);
            const materialized = await materializeVideoSrc(backendMedia.src, livePhotoObjectUrlRef);
            if (cancelled) {
              if (materialized.startsWith("blob:")) {
                URL.revokeObjectURL(materialized);
              }
              return;
            }
            setLivePhotoMotionSrc(materialized);
            setLivePhotoMotionError(undefined);
            return;
          }
          if (backendMedia.status === "pending") {
            setLivePhotoTranscoding(true);
            setLivePhotoMotionError(undefined);
            setLivePhotoSourceLabel(backendMedia.source ?? "transcoding");
            setLivePhotoTranscodeStatus({
              codec: backendMedia.codec ?? undefined,
              encoder: backendMedia.encoder ?? undefined,
              elapsedMs: backendMedia.elapsed_ms ?? undefined,
              timeoutMs: backendMedia.timeout_ms ?? undefined,
              sourceBytes: backendMedia.source_bytes ?? undefined,
              outputBytes: backendMedia.output_bytes ?? undefined,
            });
            if (!loggedPending) {
              loggedPending = true;
              void logClient(
                "viewer.live_photo",
                `viewer-v2 asset ${assetId} background motion transcode in progress`,
              );
            }
            await new Promise((resolve) => window.setTimeout(resolve, 750));
            continue;
          }

          setLivePhotoTranscoding(false);
          setLivePhotoTranscodeStatus({
            codec: backendMedia.codec ?? undefined,
            encoder: backendMedia.encoder ?? undefined,
            elapsedMs: backendMedia.elapsed_ms ?? undefined,
            timeoutMs: backendMedia.timeout_ms ?? undefined,
            sourceBytes: backendMedia.source_bytes ?? undefined,
            outputBytes: backendMedia.output_bytes ?? undefined,
          });
          void logClient(
            "viewer.live_photo",
            `asset ${assetId} backend motion unavailable: ${backendMedia.message ?? "unavailable"}`,
            "error",
          );
          setLivePhotoMotionError(backendMedia.message ?? "Live photo playback unavailable");
          return;
        }
      } catch (error) {
        if (!cancelled) {
          setLivePhotoTranscoding(false);
          setLivePhotoTranscodeStatus({});
          void logClient("viewer.live_photo", `asset ${assetId} backend motion failed: ${String(error)}`, "error");
          setLivePhotoMotionError(String(error));
        }
      }
    })();

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
    setImageError(undefined);
    setDisplaySourceLabel("Loading");
    void logClient(
      "viewer.image",
      `asset ${assetId} loading backend image source=preview path=${asset?.primary_path ?? "unknown"} (${describeImageSupport(asset?.primary_path)})`,
    );

    void api
      .requestThumbnail(assetId, viewerPreviewSize, true)
      .then((src) => {
        if (cancelled) return;
        if (src) {
          setImageSourceLabel("preview");
          void logClient(
            "viewer.image",
            `asset ${assetId} loaded ${viewerPreviewSize}px viewer preview thumbnail path=${asset?.primary_path ?? "unknown"}`,
          );
          onViewerPreviewReadyRef.current?.(assetId);
          setImageSrc(materializeImageSrc(src) ?? undefined);
          setImageError(undefined);
          return;
        }

        void logClient(
          "viewer.image",
          `asset ${assetId} ${viewerPreviewSize}px viewer preview thumbnail unavailable path=${asset?.primary_path ?? "unknown"}`,
          "error",
        );
        setImageSrc(undefined);
        setImageError("Image preview unavailable");
      })
      .catch((error) => {
        if (!cancelled) {
          void logClient(
            "viewer.image",
            `asset ${assetId} ${viewerPreviewSize}px viewer preview load failed: ${String(error)} path=${asset?.primary_path ?? "unknown"}`,
            "error",
          );
          setImageSrc(undefined);
          setImageError(String(error));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [assetId, asset?.primary_path, isPhoto, viewerPreviewSize]);

  function pauseModalPlayback() {
    videoElementRef.current?.pause();
    livePhotoVideoElementRef.current?.pause();
  }

  async function handleRevealInFileManager() {
    if (!asset?.primary_path) {
      return;
    }
    pauseModalPlayback();
    await api.revealAssetInFileManager(asset.id);
  }

  async function handleOpenInDefaultApp() {
    if (!asset?.primary_path) {
      return;
    }
    pauseModalPlayback();
    await api.openAssetWithDefaultApp(asset.id);
  }

  async function handleOpenAssetPreview() {
    if (!asset?.primary_path) {
      return;
    }
    pauseModalPlayback();
    await api.openAssetPreview(asset.id);
  }

  async function handleOpenOnGooglePhotos() {
    if (!asset?.google_photos_url) {
      return;
    }
    pauseModalPlayback();
    await api.openUrlInBrowser(asset.google_photos_url);
  }

  async function fallbackVideoToBackend() {
    if (!assetId || videoFallbackAttempted) {
      return;
    }
    setVideoFallbackAttempted(true);
    setVideoFallbackAvailable(false);
    setVideoError(undefined);
    setVideoTranscoding(true);
    setVideoTranscodeStatus({});
    setVideoSourceLabel("transcoding");
    void logClient("viewer.video", `asset ${assetId} switching to transcoded backend playback after backend-original error`);
    try {
      while (true) {
        const backendMedia = await api.loadViewerVideo(assetId, false);
        if (backendMedia.status === "ready" && backendMedia.src && backendMedia.source) {
          setVideoTranscoding(false);
          setVideoTranscodeStatus({});
          void logClient("viewer.video", `asset ${assetId} transcoded backend playback ready after backend-original error`);
          setVideoSourceLabel(backendMedia.source);
          setVideoSrc(await materializeVideoSrc(backendMedia.src, videoObjectUrlRef));
          return;
        }
        if (backendMedia.status === "pending") {
          setVideoTranscodeStatus({
            codec: backendMedia.codec ?? undefined,
            encoder: backendMedia.encoder ?? undefined,
            elapsedMs: backendMedia.elapsed_ms ?? undefined,
            timeoutMs: backendMedia.timeout_ms ?? undefined,
            sourceBytes: backendMedia.source_bytes ?? undefined,
            outputBytes: backendMedia.output_bytes ?? undefined,
          });
          await new Promise((resolve) => window.setTimeout(resolve, 750));
          continue;
        }
        setVideoTranscoding(false);
        setVideoTranscodeStatus({
          codec: backendMedia.codec ?? undefined,
          encoder: backendMedia.encoder ?? undefined,
          elapsedMs: backendMedia.elapsed_ms ?? undefined,
          timeoutMs: backendMedia.timeout_ms ?? undefined,
          sourceBytes: backendMedia.source_bytes ?? undefined,
          outputBytes: backendMedia.output_bytes ?? undefined,
        });
        void logClient("viewer.video", `asset ${assetId} transcoded backend playback unavailable after backend-original error`, "error");
        setVideoError(backendMedia.message ?? "Video playback unavailable");
        return;
      }
    } catch (error) {
      setVideoTranscoding(false);
      setVideoTranscodeStatus({});
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
    setLivePhotoTranscodeStatus({});
    setLivePhotoSourceLabel("transcoding");
    void logClient("viewer.live_photo", `asset ${assetId} switching to transcoded backend motion after backend-original error`);
    try {
      while (true) {
        const backendMedia = await api.loadLivePhotoMotion(assetId, false);
        if (backendMedia.status === "ready" && backendMedia.src && backendMedia.source) {
          setLivePhotoTranscoding(false);
          setLivePhotoTranscodeStatus({});
          void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion ready after backend-original error`);
          setLivePhotoSourceLabel(backendMedia.source);
          setLivePhotoMotionSrc(await materializeVideoSrc(backendMedia.src, livePhotoObjectUrlRef));
          return;
        }
        if (backendMedia.status === "pending") {
          setLivePhotoTranscodeStatus({
            codec: backendMedia.codec ?? undefined,
            encoder: backendMedia.encoder ?? undefined,
            elapsedMs: backendMedia.elapsed_ms ?? undefined,
            timeoutMs: backendMedia.timeout_ms ?? undefined,
            sourceBytes: backendMedia.source_bytes ?? undefined,
            outputBytes: backendMedia.output_bytes ?? undefined,
          });
          await new Promise((resolve) => window.setTimeout(resolve, 750));
          continue;
        }
        setLivePhotoTranscoding(false);
        setLivePhotoTranscodeStatus({
          codec: backendMedia.codec ?? undefined,
          encoder: backendMedia.encoder ?? undefined,
          elapsedMs: backendMedia.elapsed_ms ?? undefined,
          timeoutMs: backendMedia.timeout_ms ?? undefined,
          sourceBytes: backendMedia.source_bytes ?? undefined,
          outputBytes: backendMedia.output_bytes ?? undefined,
        });
        void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion unavailable after backend-original error`, "error");
        setLivePhotoMotionError(backendMedia.message ?? "Live photo playback unavailable");
        return;
      }
    } catch (error) {
      setLivePhotoTranscoding(false);
      setLivePhotoTranscodeStatus({});
      void logClient("viewer.live_photo", `asset ${assetId} transcoded backend motion failed after backend-original error: ${String(error)}`, "error");
      setLivePhotoMotionError(String(error));
    }
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
      const target = event.target as HTMLElement | null;
      const tagName = target?.tagName?.toLowerCase();
      const isEditable =
        tagName === "input" ||
        tagName === "textarea" ||
        tagName === "select" ||
        target?.isContentEditable;

      if (event.key === "ArrowLeft" && hasPrevious) {
        event.preventDefault();
        onPrevious();
      } else if (event.key === "ArrowRight" && hasNext) {
        event.preventDefault();
        onNext();
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if ((event.key === "f" || event.key === "F") && asset?.primary_path && !isEditable) {
        event.preventDefault();
        void handleRevealInFileManager();
      } else if ((event.key === "o" || event.key === "O") && asset?.primary_path && !isEditable) {
        event.preventDefault();
        void handleOpenInDefaultApp();
      } else if (event.key === " " && asset?.primary_path) {
        if (isEditable) {
          return;
        }
        event.preventDefault();
        void handleOpenAssetPreview();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [asset, hasNext, hasPrevious, isPhoto, onClose, onNext, onPrevious]);

  useEffect(() => {
    if (showLivePhotoMotion) {
      setDisplaySourceLabel(livePhotoSourceLabelRef.current);
    } else if (isVideo) {
      setDisplaySourceLabel(videoSourceLabelRef.current);
    } else if (isPhoto) {
      setDisplaySourceLabel(imageSourceLabelRef.current);
    } else {
      setDisplaySourceLabel("Loading");
    }
  }, [isPhoto, isVideo, showLivePhotoMotion]);

  if (!asset) return null;

  return (
    <div className="viewer-backdrop" onClick={onClose}>
      <div
        className="viewer-card viewer-card-immersive"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="viewer-toolbar">
          <strong>
            {asset.title ?? "Untitled asset"}
            {typeof asset.id === "number" ? ` • #${asset.id}` : ""}
          </strong>
          <div className="button-row">
            <span className="viewer-source-badge">{formatViewerSourceLabel(displaySourceLabel)}</span>
            {asset?.primary_path ? (
              <>
                <button
                  className="button-secondary"
                  onClick={() => void handleRevealInFileManager()}
                >
                  {revealActionLabel}
                </button>
                <button
                  className="button-secondary"
                  onClick={() => void handleOpenInDefaultApp()}
                >
                  Open In Default App
                </button>
                <button
                  className="button-secondary"
                  onClick={() => void handleOpenAssetPreview()}
                >
                  {previewActionLabel}
                </button>
              </>
            ) : null}
            {asset.google_photos_url ? (
              <button className="button-secondary" onClick={() => void handleOpenOnGooglePhotos()}>
                View On Google Photos
              </button>
            ) : null}
            {isPhoto ? (
              <>
                {livePhotoPath ? (
                  <button className="button-secondary" onClick={() => setShowLivePhotoMotion((value) => !value)}>
                    {showLivePhotoMotion ? "Show Photo" : "Play Live Photo"}
                  </button>
                ) : null}
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
        <div className="viewer-media viewer-media-immersive">
          {asset.media_kind === "video" ? (
            videoSrc ? (
              <video
                key={videoSrc}
                ref={videoElementRef}
                controls
                autoPlay
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
              >
                <source
                  src={videoSrc}
                  type={inferVideoMimeType(videoSrc) ?? "video/mp4"}
                />
              </video>
            ) : videoError ? (
              <div className="viewer-loading-state">
                <div className="muted">{videoError}</div>
                {videoTranscodeStatus.codec ? (
                  <div className="muted">Source codec: {videoTranscodeStatus.codec}</div>
                ) : null}
                {videoTranscodeStatus.encoder ? (
                  <div className="muted">Transcode encoder: {videoTranscodeStatus.encoder}</div>
                ) : null}
                {videoFallbackAvailable ? (
                  <button className="button-secondary" onClick={() => void fallbackVideoToBackend()}>
                    Transcode For Playback
                  </button>
                ) : null}
              </div>
            ) : videoTranscoding ? (
              <div className="viewer-loading-state">
                <div>Transcoding video...</div>
                <div className="muted">{formatTranscodeProgress(videoTranscodeStatus)}</div>
              </div>
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
              >
                <source
                  src={livePhotoMotionSrc}
                  type={inferVideoMimeType(livePhotoMotionSrc) ?? "video/mp4"}
                />
              </video>
            ) : livePhotoMotionError ? (
              <div className="viewer-loading-state">
                <div className="muted">{livePhotoMotionError}</div>
                {livePhotoTranscodeStatus.codec ? (
                  <div className="muted">Source codec: {livePhotoTranscodeStatus.codec}</div>
                ) : null}
                {livePhotoTranscodeStatus.encoder ? (
                  <div className="muted">Transcode encoder: {livePhotoTranscodeStatus.encoder}</div>
                ) : null}
                {livePhotoFallbackAvailable ? (
                  <button className="button-secondary" onClick={() => void fallbackLivePhotoToBackend()}>
                    Transcode Live Photo
                  </button>
                ) : null}
              </div>
            ) : livePhotoTranscoding ? (
              <div className="viewer-loading-state">
                <div>Transcoding live photo...</div>
                <div className="muted">{formatTranscodeProgress(livePhotoTranscodeStatus)}</div>
              </div>
            ) : (
              <div className="muted">Loading live photo…</div>
            )
          ) : imageSrc && assetId === asset.id ? (
            <div className="viewer-image-frame">
              {livePhotoPath ? (
                <div className="viewer-live-photo-label">Live photo</div>
              ) : null}
              <img
                src={imageSrc}
                alt={asset.title ?? "asset"}
                className={livePhotoPath ? "viewer-live-photo-still" : undefined}
                style={
                  canonicalImageSize
                    ? {
                        width: `${Math.max(1, Math.round(canonicalImageSize.width))}px`,
                        height: `${Math.max(1, Math.round(canonicalImageSize.height))}px`,
                      }
                    : undefined
                }
                onLoad={(event) => {
                  void logClient(
                    "viewer.image",
                    `asset ${assetId} image loaded source=${imageSourceLabelRef.current} size=${event.currentTarget.naturalWidth}x${event.currentTarget.naturalHeight}`,
                  );
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
                  void logClient(
                    "viewer.image",
                    `asset ${assetId} image unavailable source=preview path=${asset?.primary_path ?? "unknown"}`,
                    "error",
                  );
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
            {canonicalImageSize ? ` • ${canonicalImageSize.width}x${canonicalImageSize.height}` : ""}
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

function describeVideoSupport(src?: string | null) {
  if (typeof document === "undefined") {
    return "video support unknown";
  }
  const probe = document.createElement("video");
  const requestedType = inferVideoMimeType(src) ?? "unknown";
  const mp4 = probe.canPlayType("video/mp4") || "no";
  const mp4Avc = probe.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"') || "no";
  const mp4Hevc = probe.canPlayType('video/mp4; codecs="hvc1.1.6.L93.B0, mp4a.40.2"') || "no";
  const mov = probe.canPlayType("video/quicktime") || "no";
  const webm = probe.canPlayType("video/webm") || "no";
  const hls = probe.canPlayType("application/vnd.apple.mpegurl") || "no";
  const ts = probe.canPlayType("video/mp2t") || "no";
  const mseMp4Avc =
    typeof MediaSource !== "undefined" &&
    typeof MediaSource.isTypeSupported === "function"
      ? MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E, mp4a.40.2"')
      : false;
  const mseHls =
    typeof MediaSource !== "undefined" &&
    typeof MediaSource.isTypeSupported === "function"
      ? MediaSource.isTypeSupported("application/vnd.apple.mpegurl")
      : false;
  return `requested=${requestedType} canPlayType(mp4=${mp4}, mp4-avc=${mp4Avc}, mp4-hevc=${mp4Hevc}, mov=${mov}, webm=${webm}, hls=${hls}, ts=${ts}) mse(mp4-avc=${mseMp4Avc}, hls=${mseHls})`;
}

function describeImageSupport(src?: string | null) {
  if (typeof navigator === "undefined") {
    return `image support unknown format=${inferImageFormat(src) ?? "unknown"}`;
  }
  return `format=${inferImageFormat(src) ?? "unknown"} userAgent=${navigator.userAgent}`;
}

function shouldPreferOriginalVideoBytesForPath(path?: string | null) {
  if (!path) {
    return false;
  }
  if (typeof document === "undefined") {
    return false;
  }
  const mimeType = inferVideoMimeType(path);
  if (!mimeType) {
    return false;
  }
  const probe = document.createElement("video");
  return probe.canPlayType(mimeType) !== "";
}

function inferVideoMimeType(src?: string | null) {
  if (!src) return undefined;
  if (src.startsWith("data:video/")) {
    return src.slice("data:".length).split(";")[0];
  }
  const extension = src.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "m3u8":
      return "application/vnd.apple.mpegurl";
    case "mov":
      return "video/quicktime";
    case "ts":
      return "video/mp2t";
    case "webm":
      return "video/webm";
    case "mpg":
    case "mpeg":
      return "video/mpeg";
    case "m4v":
    case "mp4":
      return "video/mp4";
    default:
      return undefined;
  }
}

function inferImageFormat(src?: string | null) {
  if (!src) return undefined;
  const extension = src.split(".").pop()?.toLowerCase();
  return extension;
}

function formatViewerSourceLabel(label: string) {
  switch (label) {
    case "preview":
      return "Preview Thumbnail";
    case "original":
      return "Original Image";
    case "rendered":
      return "Rendered Fallback";
    case "original_mp4":
      return "Original Video (MP4)";
    case "original_quicktime":
      return "Original Video (MOV)";
    case "original_webm":
      return "Original Video (WEBM)";
    case "transcoded_mp4":
      return "Transcoded Video";
    case "transcoding":
      return "Transcoding In Background";
    case "unset":
    case "Loading":
    default:
      return "Loading Source";
  }
}

function formatTranscodeProgress(status: {
  codec?: string;
  encoder?: string;
  elapsedMs?: number;
  timeoutMs?: number;
  sourceBytes?: number;
  outputBytes?: number;
}) {
  const parts: string[] = [];
  if (status.codec) {
    parts.push(`Source codec: ${status.codec}`);
  }
  if (status.encoder) {
    parts.push(`Encoder: ${status.encoder}`);
  }
  if (typeof status.sourceBytes === "number" && status.sourceBytes > 0) {
    const written = typeof status.outputBytes === "number" ? status.outputBytes : 0;
    parts.push(`Written ${formatFileSize(written)} • source ${formatFileSize(status.sourceBytes)}`);
  } else if (typeof status.outputBytes === "number" && status.outputBytes > 0) {
    parts.push(`Written ${formatFileSize(status.outputBytes)}`);
  }
  if (typeof status.elapsedMs === "number" && typeof status.timeoutMs === "number") {
    parts.push(
      `Elapsed ${formatDurationSeconds(status.elapsedMs)} / ${formatDurationSeconds(status.timeoutMs)} timeout`,
    );
  } else if (typeof status.elapsedMs === "number") {
    parts.push(`Elapsed ${formatDurationSeconds(status.elapsedMs)}`);
  }
  return parts.join(" • ") || "Preparing transcode…";
}

function formatDurationSeconds(milliseconds: number) {
  return `${Math.max(0, milliseconds / 1000).toFixed(1)}s`;
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
