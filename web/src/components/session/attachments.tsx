// Attaching files to a message: the attach button, drag and drop anywhere on
// the page, pasting, the files waiting in the composer, and the files a
// message carries in the transcript.

import { Download, File, FileArchive, FileImage, FileText, Paperclip, RotateCw, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { IconButton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api, type UploadScope } from "@/lib/api";
import {
  attachmentUrl,
  dragHasFiles,
  MAX_FILE_BYTES,
  MAX_FILES,
  pastedName,
  previewable,
  showsInline,
} from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { bytes } from "@/lib/format";

/** A file in the composer, uploading as soon as it is added. */
export interface Draft {
  key: string;
  file: File;
  name: string;
  /** An object URL for images the browser can show. */
  preview?: string;
  status: "uploading" | "ready" | "failed";
  progress: number;
  attachment?: G.Attachment;
  error?: string;
}

/** What a sent message hands to the transcript until the server has it. */
export interface SentFiles {
  attachments: G.Attachment[];
  /** Local previews by attachment id. */
  previews: Record<string, string>;
}

interface Upload {
  to: string;
  promise: Promise<G.Attachment>;
  abort: () => void;
}

/**
 * The files of the message being written. Uploads go to `scope` (and again
 * if it changes, like the floor's project picker); `take` waits for them and
 * empties the composer.
 */
export function useAttachments(scope: UploadScope | undefined) {
  const to = scope ? JSON.stringify(scope) : undefined;
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const uploads = useRef(new Map<string, Upload>());
  const pasted = useRef(0);
  const live = useRef(drafts);
  live.current = drafts;

  const patch = useCallback((key: string, p: Partial<Draft>) => {
    setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...p } : d)));
  }, []);

  const start = useCallback(
    (d: Draft, to: string) => {
      uploads.current.get(d.key)?.abort();
      const up = api.uploadAttachment(JSON.parse(to), d.file, d.name, (progress) =>
        patch(d.key, { progress }),
      );
      const entry = { to, ...up };
      uploads.current.set(d.key, entry);
      patch(d.key, { status: "uploading", progress: 0, error: undefined, attachment: undefined });
      up.promise.then(
        (attachment) => {
          if (uploads.current.get(d.key) === entry)
            patch(d.key, { status: "ready", progress: 1, attachment });
        },
        (e: Error) => {
          if (uploads.current.get(d.key) === entry) patch(d.key, { status: "failed", error: e.message });
        },
      );
    },
    [patch],
  );

  // Another project (the floor's picker) needs its own copies.
  useEffect(() => {
    if (!to) return;
    for (const d of live.current) {
      if (uploads.current.get(d.key)?.to !== to) start(d, to);
    }
  }, [to, start]);

  // Leaving the page cancels uploads and frees the previews.
  useEffect(
    () => () => {
      for (const u of uploads.current.values()) u.abort();
      for (const d of live.current) if (d.preview) URL.revokeObjectURL(d.preview);
    },
    [],
  );

  const add = useCallback(
    (files: File[], opts: { pasted?: boolean } = {}) => {
      const room = MAX_FILES - live.current.length;
      if (files.length > room) toast(`Attach at most ${MAX_FILES} files to one message`, "error");
      const added: Draft[] = [];
      for (const file of files.slice(0, Math.max(0, room))) {
        if (file.size > MAX_FILE_BYTES) {
          toast(
            `${file.name} is ${bytes(file.size)}; files can be at most ${bytes(MAX_FILE_BYTES)}`,
            "error",
          );
          continue;
        }
        added.push({
          key: crypto.randomUUID(),
          file,
          name: opts.pasted ? pastedName(file, ++pasted.current) : file.name || "file",
          preview: previewable(file.type) ? URL.createObjectURL(file) : undefined,
          status: "uploading",
          progress: 0,
        });
      }
      if (!added.length) return;
      setDrafts((ds) => [...ds, ...added]);
      if (to) for (const d of added) start(d, to);
    },
    [to, start],
  );

  const remove = useCallback((key: string) => {
    uploads.current.get(key)?.abort();
    uploads.current.delete(key);
    setDrafts((ds) => {
      const d = ds.find((x) => x.key === key);
      if (d?.preview) URL.revokeObjectURL(d.preview);
      return ds.filter((x) => x.key !== key);
    });
  }, []);

  const retry = useCallback(
    (key: string) => {
      const d = live.current.find((x) => x.key === key);
      if (d && to) start(d, to);
    },
    [to, start],
  );

  /**
   * Waits for the uploads and empties the composer, handing over the files
   * and their previews. `restore` puts them back if sending fails.
   */
  const take = useCallback(async (): Promise<{ files: SentFiles; restore: () => void }> => {
    const taken = live.current;
    const results = await Promise.allSettled(
      taken.map((d) => uploads.current.get(d.key)?.promise ?? Promise.reject(new Error("not uploaded"))),
    );
    if (results.some((r) => r.status === "rejected")) {
      throw new Error("Some files did not upload. Try them again or remove them.");
    }
    const attachments = results.map((r) => (r as PromiseFulfilledResult<G.Attachment>).value);
    const previews: Record<string, string> = {};
    taken.forEach((d, i) => {
      const a = attachments[i];
      if (d.preview && a) previews[a.id] = d.preview;
    });
    setDrafts((ds) => ds.filter((d) => !taken.includes(d)));
    return {
      files: { attachments, previews },
      restore: () => setDrafts((ds) => [...taken, ...ds]),
    };
  }, []);

  return {
    drafts,
    add,
    remove,
    retry,
    take,
    uploading: drafts.some((d) => d.status === "uploading"),
    failed: drafts.some((d) => d.status === "failed"),
  };
}

/**
 * Files dragged over the page go to `onFiles` when dropped anywhere on it,
 * so a near miss never opens the file in the tab. Returns whether a drag is
 * in progress.
 */
export function useFileDrop(onFiles: (files: File[]) => void, enabled = true): boolean {
  const [dragging, setDragging] = useState(false);
  const depth = useRef(0);
  const handler = useRef(onFiles);
  handler.current = onFiles;
  useEffect(() => {
    if (!enabled) return;
    const enter = (e: DragEvent) => {
      if (!dragHasFiles(e)) return;
      depth.current++;
      setDragging(true);
    };
    const over = (e: DragEvent) => {
      if (!dragHasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const leave = (e: DragEvent) => {
      if (!dragHasFiles(e)) return;
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) setDragging(false);
    };
    const drop = (e: DragEvent) => {
      if (!dragHasFiles(e)) return;
      e.preventDefault();
      depth.current = 0;
      setDragging(false);
      const files = Array.from(e.dataTransfer?.files ?? []);
      if (files.length) handler.current(files);
    };
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragover", over);
    window.addEventListener("dragleave", leave);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragover", over);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("drop", drop);
      depth.current = 0;
      setDragging(false);
    };
  }, [enabled]);
  return dragging;
}

/** The paperclip: picks files with the system dialog. */
export function AttachButton({
  onFiles,
  className,
}: {
  onFiles: (files: File[]) => void;
  className?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <IconButton
        type="button"
        label="Attach files"
        title="Attach files (or paste, or drop them here)"
        onClick={() => input.current?.click()}
        className={className}
      >
        <Paperclip className="size-4" />
      </IconButton>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        tabIndex={-1}
        onChange={(e) => {
          onFiles(Array.from(e.target.files ?? []));
          e.target.value = "";
        }}
      />
    </>
  );
}

/** Shown over a composer while files are dragged over the page. */
export function DropHint({ show }: { show: boolean }) {
  return (
    <div
      aria-hidden
      className={cn(
        "pointer-events-none absolute inset-0 z-10 flex items-center justify-center gap-2 rounded-[inherit] border-2 border-dashed border-accent bg-surface/90 text-[14px] font-medium text-accent backdrop-blur-sm transition-opacity duration-150",
        show ? "opacity-100" : "opacity-0",
      )}
    >
      <Paperclip className="size-4" />
      Drop to attach
    </div>
  );
}

function FileIcon({ type, className }: { type: string; className?: string }) {
  const c = cn("shrink-0", className);
  if (type.startsWith("image/")) return <FileImage className={c} />;
  if (/zip|gzip|x-tar|x-7z|x-rar/.test(type)) return <FileArchive className={c} />;
  if (type.startsWith("text/") || /json|yaml|toml|xml|pdf/.test(type)) return <FileText className={c} />;
  return <File className={c} />;
}

/** The files waiting in a composer. */
export function DraftTray({
  drafts,
  onRemove,
  onRetry,
  className,
}: {
  drafts: Draft[];
  onRemove: (key: string) => void;
  onRetry: (key: string) => void;
  className?: string;
}) {
  if (!drafts.length) return null;
  return (
    <ul className={cn("flex flex-wrap gap-2", className)} aria-label="Attached files">
      {drafts.map((d) => (
        <DraftChip key={d.key} d={d} onRemove={() => onRemove(d.key)} onRetry={() => onRetry(d.key)} />
      ))}
    </ul>
  );
}

function DraftChip({ d, onRemove, onRetry }: { d: Draft; onRemove: () => void; onRetry: () => void }) {
  const failed = d.status === "failed";
  const status =
    d.status === "uploading"
      ? `Uploading ${Math.round(d.progress * 100)}%`
      : failed
        ? (d.error ?? "Upload failed")
        : bytes(d.file.size);
  return (
    <li
      title={`${d.name} · ${status}`}
      className={cn(
        "group/chip relative h-14 animate-rise overflow-hidden rounded-lg border bg-raised",
        failed ? "border-danger/60" : "border-line-strong",
        d.preview ? "w-14" : "flex w-52 items-center gap-2.5 px-3",
      )}
    >
      {d.preview ? (
        <img src={d.preview} alt={d.name} className="size-full object-cover" />
      ) : (
        <>
          <FileIcon type={d.file.type} className={cn("size-5", failed ? "text-danger" : "text-ink-3")} />
          <div className="min-w-0 flex-1">
            <div className="truncate text-[12.5px] text-ink">{d.name}</div>
            <div className={cn("truncate font-mono text-[10.5px]", failed ? "text-danger" : "text-ink-4")}>
              {status}
            </div>
          </div>
        </>
      )}
      {d.status === "uploading" && (
        <div className="absolute inset-x-0 bottom-0 h-0.5 bg-line">
          <div
            className="h-full bg-accent transition-[width] duration-150"
            style={{ width: `${Math.max(4, d.progress * 100)}%` }}
          />
        </div>
      )}
      {failed && (
        <button
          type="button"
          onClick={onRetry}
          aria-label={`Upload ${d.name} again`}
          title={d.error ? `${d.error}. Upload again.` : "Upload again"}
          className={cn(
            "flex items-center justify-center text-danger",
            d.preview ? "absolute inset-0 bg-canvas/70" : "-mr-1 size-6 rounded-md hover:bg-danger-soft",
          )}
        >
          <RotateCw className="size-3.5" />
        </button>
      )}
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Remove ${d.name}`}
        className="absolute top-1 right-1 flex size-5 items-center justify-center rounded-full bg-ink/80 text-canvas opacity-0 transition-opacity duration-150 group-hover/chip:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100"
      >
        <X className="size-3" />
      </button>
    </li>
  );
}

/** The files a message carries, in the transcript. */
export function MessageFiles({
  sessionId,
  files,
  previews,
  pending,
}: {
  sessionId?: string;
  files: G.Attachment[];
  previews?: Record<string, string>;
  pending?: boolean;
}) {
  const [open, setOpen] = useState<G.Attachment>();
  const src = (f: G.Attachment) =>
    previews?.[f.id] ?? (sessionId ? attachmentUrl(sessionId, f.id) : undefined);
  const images = files.filter((f) => previews?.[f.id] || showsInline(f.media_type));
  const others = files.filter((f) => !images.includes(f));
  return (
    <div className="flex flex-wrap items-start gap-2">
      {images.map((f) => (
        <button
          key={f.id}
          type="button"
          onClick={() => setOpen(f)}
          aria-label={`View ${f.name}`}
          className="overflow-hidden rounded-lg border border-line bg-raised/50 transition-[border-color] duration-150 hover:border-line-strong"
        >
          <img
            src={src(f)}
            alt={f.name}
            width={f.width}
            height={f.height}
            loading="lazy"
            decoding="async"
            className="h-36 w-auto max-w-[min(100%,20rem)] object-cover"
          />
        </button>
      ))}
      {others.map((f) => {
        const body: ReactNode = (
          <>
            <FileIcon type={f.media_type} className="size-5 text-ink-3" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] text-ink">{f.name}</div>
              <div className="font-mono text-[10.5px] text-ink-4">{bytes(f.size)}</div>
            </div>
          </>
        );
        const chip =
          "flex h-12 w-60 max-w-full items-center gap-2.5 rounded-lg border border-line bg-raised/50 px-3";
        return pending || !sessionId ? (
          <div key={f.id} className={chip}>
            {body}
          </div>
        ) : (
          <a
            key={f.id}
            href={attachmentUrl(sessionId, f.id)}
            download={f.name}
            title={`Download ${f.name}`}
            className={cn(chip, "group/file transition-[border-color] duration-150 hover:border-line-strong")}
          >
            {body}
            <Download className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-opacity duration-150 group-hover/file:opacity-100" />
          </a>
        );
      })}
      {open && (
        <ImageViewer file={open} src={src(open)} download={!pending} onClose={() => setOpen(undefined)} />
      )}
    </div>
  );
}

/** A full-size look at an image, on the native <dialog> (Escape closes it). */
function ImageViewer({
  file,
  src,
  download,
  onClose,
}: {
  file: G.Attachment;
  src?: string;
  download: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes on the backdrop only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
      aria-label={file.name}
      className="m-auto max-h-none max-w-none bg-transparent p-0 backdrop:bg-black/75 backdrop:backdrop-blur-[3px] open:animate-rise"
    >
      <figure className="flex flex-col items-center gap-3">
        <img
          src={src}
          alt={file.name}
          className="max-h-[82vh] max-w-[92vw] rounded-lg border border-line-strong bg-raised object-contain shadow-[0_30px_120px_rgb(0_0_0/0.55)]"
        />
        <figcaption className="flex max-w-[92vw] items-center gap-3 rounded-full border border-line-strong bg-surface/90 py-1 pr-1 pl-4 text-[12.5px] text-ink-2 backdrop-blur-xl">
          <span className="truncate">{file.name}</span>
          <span className="shrink-0 font-mono text-[11px] text-ink-4">
            {file.width && file.height ? `${file.width}×${file.height} · ` : ""}
            {bytes(file.size)}
          </span>
          {download && src && (
            <a
              href={src}
              download={file.name}
              className="flex size-7 shrink-0 items-center justify-center rounded-full text-ink-3 hover:bg-hover hover:text-ink"
              aria-label={`Download ${file.name}`}
              title="Download"
            >
              <Download className="size-3.5" />
            </a>
          )}
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="flex size-7 shrink-0 items-center justify-center rounded-full text-ink-3 hover:bg-hover hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </figcaption>
      </figure>
    </dialog>,
    document.body,
  );
}
