import { convertFileSrc } from "@tauri-apps/api/core";

const WINDOWS_ABSOLUTE_PATH = /^[a-zA-Z]:[\\/]/;
const FILE_URI = /^file:\/\//i;

export function materializeImageSrc(src?: string | null) {
  if (!src) {
    return src;
  }
  if (shouldConvertLocalFilePath(src)) {
    return convertFileSrc(src);
  }
  return src;
}

export async function materializeVideoSrc(
  src: string,
  objectUrlRef: React.MutableRefObject<string | undefined>,
) {
  if (objectUrlRef.current) {
    URL.revokeObjectURL(objectUrlRef.current);
    objectUrlRef.current = undefined;
  }
  if (shouldConvertLocalFilePath(src)) {
    return convertFileSrc(src);
  }
  if (!src.startsWith("data:video/")) {
    return src;
  }
  const [header, base64Payload] = src.split(",", 2);
  const mimeType = header.slice("data:".length).split(";")[0] || "video/mp4";
  const payload = base64Payload ?? "";
  const binary = atob(payload);
  const chunkSize = 1024 * 1024;
  const chunks: Uint8Array[] = [];
  for (let offset = 0; offset < binary.length; offset += chunkSize) {
    const chunk = binary.slice(offset, offset + chunkSize);
    const bytes = new Uint8Array(chunk.length);
    for (let index = 0; index < chunk.length; index += 1) {
      bytes[index] = chunk.charCodeAt(index);
    }
    chunks.push(bytes);
  }
  const blob = new Blob(chunks as BlobPart[], { type: mimeType });
  const objectUrl = URL.createObjectURL(blob);
  objectUrlRef.current = objectUrl;
  return objectUrl;
}

function shouldConvertLocalFilePath(src: string) {
  return src.startsWith("/") || WINDOWS_ABSOLUTE_PATH.test(src) || FILE_URI.test(src);
}