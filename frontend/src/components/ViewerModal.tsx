import { useEffect, useState } from "react";

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
  const [zoom, setZoom] = useState(1);
  const [zoomAssetId, setZoomAssetId] = useState<number>();
  const assetId = asset?.id;
  const isPhoto = asset && asset.media_kind !== "video";

  if (assetId !== zoomAssetId) {
    setZoomAssetId(assetId);
    if (zoom !== 1) {
      setZoom(1);
    }
  }

  useEffect(() => {
    let cancelled = false;
    if (!assetId || !isPhoto) {
      return;
    }

    void api
      .loadViewerFrame(assetId)
      .then((src) => {
        if (cancelled) return;
        if (src) {
          setImageSrc(src);
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
  }, [assetId, isPhoto]);

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
        setZoom((current) => Math.min(current + 0.25, 4));
      } else if (event.key === "-" && isPhoto) {
        event.preventDefault();
        setZoom((current) => Math.max(current - 0.25, 0.5));
      } else if (event.key === "0" && isPhoto) {
        event.preventDefault();
        setZoom(1);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [asset, hasNext, hasPrevious, isPhoto, onClose, onNext, onPrevious]);

  if (!asset) return null;

  const primaryPath = asset.primary_path ? convertFileSrc(asset.primary_path) : undefined;
  const livePhotoPath = asset.live_photo_video_path
    ? convertFileSrc(asset.live_photo_video_path)
    : undefined;

  return (
    <div className="viewer-backdrop" onClick={onClose}>
      <div className="viewer-card" onClick={(event) => event.stopPropagation()}>
        <div className="viewer-toolbar">
          <strong>{asset.title ?? "Untitled asset"}</strong>
          <div className="button-row">
            {isPhoto ? (
              <>
                <button className="button-secondary" onClick={() => setZoom((current) => Math.max(current - 0.25, 0.5))}>
                  Zoom -
                </button>
                <button className="button-secondary" onClick={() => setZoom(1)}>
                  100%
                </button>
                <button className="button-secondary" onClick={() => setZoom((current) => Math.min(current + 0.25, 4))}>
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
        <div className="viewer-media">
          {asset.media_kind === "video" && primaryPath ? (
            <video src={primaryPath} controls autoPlay />
          ) : imageSrc && assetId === asset.id ? (
            <img
              src={imageSrc}
              alt={asset.title ?? "asset"}
              style={{ transform: `scale(${zoom})`, transformOrigin: "center center" }}
            />
          ) : imageError ? (
            <div className="muted">{imageError}</div>
          ) : (
            <div className="muted">Loading image…</div>
          )}
        </div>
        <div className="viewer-meta">
          <p className="muted">
            {asset.taken_at_utc ? dayjs(asset.taken_at_utc).format("YYYY-MM-DD HH:mm:ss") : "Unknown capture time"}
            {isPhoto ? ` • zoom ${Math.round(zoom * 100)}%` : ""}
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
          {livePhotoPath ? (
            <>
              <p className="muted">Live photo motion companion</p>
              <video src={livePhotoPath} controls muted />
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
