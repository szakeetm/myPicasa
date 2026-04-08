import dayjs from "dayjs";
import { convertFileSrc } from "@tauri-apps/api/core";

import type { AssetDetail } from "../lib/types";

type ViewerModalProps = {
  asset?: AssetDetail;
  onClose: () => void;
};

export function ViewerModal({ asset, onClose }: ViewerModalProps) {
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
          ) : primaryPath ? (
            <img src={primaryPath} alt={asset.title ?? "asset"} />
          ) : (
            <div className="muted">Source media unavailable.</div>
          )}
        </div>
        <div className="viewer-meta">
          <div className="button-row" style={{ justifyContent: "space-between" }}>
            <strong>{asset.title ?? "Untitled asset"}</strong>
            <button className="button-danger" onClick={onClose}>
              Close
            </button>
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
