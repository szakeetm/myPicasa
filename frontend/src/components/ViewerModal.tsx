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
  const assetId = asset?.id;
  const isPhoto = asset && asset.media_kind !== "video";

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
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [asset, hasNext, hasPrevious, onClose, onNext, onPrevious]);

  if (!asset) return null;

  const primaryPath = asset.primary_path ? convertFileSrc(asset.primary_path) : undefined;
  const livePhotoPath = asset.live_photo_video_path
    ? convertFileSrc(asset.live_photo_video_path)
    : undefined;

  return (
    <div className="viewer-backdrop" onClick={onClose}>
      <div className="viewer-card" onClick={(event) => event.stopPropagation()}>
        <div className="viewer-media">
          {asset.media_kind === "video" && primaryPath ? (
            <video src={primaryPath} controls autoPlay />
          ) : imageSrc && assetId === asset.id ? (
            <img src={imageSrc} alt={asset.title ?? "asset"} />
          ) : imageError ? (
            <div className="muted">{imageError}</div>
          ) : (
            <div className="muted">Loading image…</div>
          )}
        </div>
        <div className="viewer-meta">
          <div className="button-row" style={{ justifyContent: "space-between" }}>
            <strong>{asset.title ?? "Untitled asset"}</strong>
            <div className="button-row">
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
          <p className="muted">
            {asset.taken_at_utc ? dayjs(asset.taken_at_utc).format("YYYY-MM-DD HH:mm:ss") : "Unknown capture time"}
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
