// Files people attach to messages: limits, names and URLs.

/** What the control plane accepts (fctl-domain `files`). */
export const MAX_FILE_BYTES = 20_000_000;
export const MAX_FILES = 10;

/** Raster images the control plane serves for display; other files are downloads. */
export function showsInline(type: string): boolean {
  return type === "image/png" || type === "image/jpeg" || type === "image/gif" || type === "image/webp";
}

/**
 * Videos the lightbox plays. Except on touch screens, it offers the download
 * of one the browser can't play.
 */
export function playable(type: string): boolean {
  return type === "video/mp4" || type === "video/webm" || type === "video/quicktime";
}

/** Whether the browser can show a local file in an <img> before it is sent. */
export function previewable(type: string): boolean {
  return type.startsWith("image/") && !/^image\/(heic|heif|tiff)$/.test(type);
}

/** Browsers name every pasted image `image.png`; number them instead. */
export function pastedName(file: File, n: number): string {
  if (file.name && !/^image\.(png|jpe?g|gif|webp)$/i.test(file.name)) return file.name;
  const ext = (file.type.split("/")[1] ?? "png").replace("jpeg", "jpg").replace(/\+.*/, "");
  return n > 1 ? `pasted-image-${n}.${ext}` : `pasted-image.${ext}`;
}

/**
 * The files a paste attaches. Text copied from Word, Excel or Pages carries a
 * picture of itself as well: when there is rich text, the paste is text.
 */
export function filesFromPaste(data: DataTransfer | null): File[] {
  if (!data?.files.length) return [];
  const rich = data.types.includes("text/html") || data.types.includes("text/rtf");
  if (rich && data.getData("text/plain").trim()) return [];
  return Array.from(data.files);
}

/** Whether a drag carries files (not text or a link). */
export function dragHasFiles(e: DragEvent | React.DragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes("Files");
}

export function attachmentUrl(session: string, id: string): string {
  return `/api/v1/sessions/${encodeURIComponent(session)}/attachments/${encodeURIComponent(id)}`;
}
