import { Download, X } from "lucide-react";
import { type MouseEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { playable } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { bytes } from "@/lib/format";
import { useHoldPage } from "@/lib/hold-page";

/** What a lightbox shows. Attachments and published screenshots are these. */
export interface LightboxFile {
  name: string;
  /** A video type plays in a player; anything else shows as an image. */
  media_type?: string;
  /** Bytes. */
  size?: number;
  width?: number;
  height?: number;
}

/**
 * Whether a click on a link to a file may show it in a lightbox: a plain
 * click. With a modifier key, the browser opens the link in a new tab or
 * window as usual.
 */
export function plainClick(
  e: Pick<MouseEvent, "button" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
): boolean {
  return e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey;
}

/**
 * A screenshot or a video at full size over the page, on the native modal
 * <dialog>. The X at the top right, a click anywhere outside the picture
 * and its caption, or Escape closes it, and focus goes back to what opened
 * it.
 */
export function Lightbox({
  file,
  src,
  download,
  onClose,
}: {
  file: LightboxFile;
  src: string;
  /**
   * Offers the file as a download, except on touch screens, where iOS
   * answers a download with a sheet of its own.
   */
  download?: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const stage = useRef<HTMLElement>(null);
  // A press that starts on the picture and ends outside it, selecting or
  // dragging, doesn't close it.
  const pressedOutside = useRef(false);
  const [failed, setFailed] = useState(false);
  const video = playable(file.media_type ?? "");
  // The dialog focuses itself as it opens, not the X: Safari shows a focus
  // ring on what a dialog focuses, even when a tap opened it. Tab reaches
  // the X. (Chrome focuses the X anyway, with a ring only for the keyboard.)
  // React doesn't set the `autofocus` attribute from `autoFocus`.
  useEffect(() => {
    ref.current?.setAttribute("autofocus", "");
    ref.current?.showModal();
  }, []);
  useHoldPage();
  // Through the dialog, which hands focus back; its `close` event tells the owner.
  const close = () => ref.current?.close();
  const outside = (target: EventTarget) => target === ref.current || target === stage.current;
  const facts = [
    file.width && file.height ? `${file.width}×${file.height}` : "",
    file.size === undefined ? "" : bytes(file.size),
  ]
    .filter(Boolean)
    .join(" · ");
  // Room above for the X, and below for the caption.
  const fit =
    "max-h-[calc(100dvh-8rem)] max-w-[calc(100vw-2rem)] rounded-lg border border-line-strong shadow-[0_30px_120px_rgb(0_0_0/0.55)]";
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes outside the picture only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      aria-label={file.name}
      onClose={onClose}
      onPointerDown={(e) => {
        pressedOutside.current = outside(e.target);
      }}
      onClick={(e) => {
        if (pressedOutside.current && outside(e.target)) close();
      }}
      className="m-0 size-full max-h-none max-w-none overflow-hidden bg-transparent p-0 outline-none backdrop:animate-fade backdrop:bg-black/75 backdrop:backdrop-blur-[3px]"
    >
      <button
        type="button"
        onClick={close}
        aria-label="Close"
        className="group fixed top-3 right-3 z-10 rounded-full border border-line-strong bg-surface/90 p-1 backdrop-blur-xl"
      >
        <span className="flex size-7 items-center justify-center rounded-full text-ink-3 transition-colors duration-150 group-hover:bg-hover group-hover:text-ink">
          <X className="size-4" />
        </span>
      </button>
      <figure
        ref={stage}
        className="flex size-full animate-zoom-in flex-col items-center justify-center gap-3 px-4 pt-14 pb-4"
      >
        {failed ? (
          <p className="rounded-lg border border-line-strong bg-surface px-5 py-4 text-[13.5px] text-ink-2">
            {video ? `This browser can't play ${file.name}.` : `${file.name} didn't load.`}
          </p>
        ) : video ? (
          // biome-ignore lint/a11y/useMediaCaption: recordings people attach come without captions.
          <video
            src={src}
            controls
            autoPlay
            playsInline
            onError={() => setFailed(true)}
            className={cn(fit, "bg-black")}
          />
        ) : (
          <img
            src={src}
            alt={file.name}
            decoding="async"
            onError={() => setFailed(true)}
            className={cn(fit, "h-auto w-auto bg-raised object-contain")}
          />
        )}
        <figcaption
          className={cn(
            "flex h-9 max-w-[calc(100vw-2rem)] items-center gap-3 rounded-full border border-line-strong bg-surface/90 pl-4 text-[12.5px] text-ink-2 backdrop-blur-xl",
            download ? "pr-1 pointer-coarse:pr-4" : "pr-4",
          )}
        >
          <span className="truncate">{file.name}</span>
          {facts && <span className="shrink-0 font-mono text-[11px] text-ink-4">{facts}</span>}
          {download && (
            <a
              href={src}
              download={file.name}
              aria-label={`Download ${file.name}`}
              title="Download"
              className="flex size-7 shrink-0 items-center justify-center rounded-full text-ink-3 hover:bg-hover hover:text-ink pointer-coarse:hidden"
            >
              <Download className="size-3.5" />
            </a>
          )}
        </figcaption>
      </figure>
    </dialog>,
    document.body,
  );
}
