// Attaching files to a message: the attach button, drag and drop anywhere on
// the page, pasting, the files waiting in the composer, and the files a
// message carries in the transcript.

import {
  Download,
  File,
  FileArchive,
  FileImage,
  FilePlay,
  FileText,
  Paperclip,
  Play,
  RotateCw,
  X,
} from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { IconButton, Lightbox, toast, useLightbox } from "@/components/ui";
import type * as G from "@/gen";
import { api, type UploadScope } from "@/lib/api";
import {
  attachmentUrl,
  dragHasFiles,
  MAX_FILE_BYTES,
  MAX_FILES,
  pastedName,
  playable,
  previewable,
  showsInline,
} from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { type DraftFile, deleteFile, loadFiles, pageFiles, saveFile, setPageFiles } from "@/lib/draft";
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
  /** Once it is done. */
  attachment?: G.Attachment;
  /** Once a message took its file, which the composer still shows (`take`'s `stay`). */
  taken?: boolean;
}

/**
 * The uploads of kept drafts (the home page's), by draft key. Finished ones
 * outlive the composer, so its files come back ready when the page does.
 */
const keptUploads = new Map<string, Upload>();

const preview = (file: File) => (previewable(file.type) ? URL.createObjectURL(file) : undefined);

/**
 * The files of the message being written. Uploads go to `scope` (and again
 * if it changes, like the home page's project picker); `take` waits for them and
 * empties the composer. With `keep` (the person's id), the files are kept in
 * this browser (`@/lib/draft`) and come back when the composer opens again.
 */
export function useAttachments(scope: UploadScope | undefined, { keep }: { keep?: string } = {}) {
  const to = scope ? JSON.stringify(scope) : undefined;
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const uploads = useRef(keep ? keptUploads : new Map<string, Upload>());
  const pasted = useRef(0);
  const live = useRef(drafts);
  live.current = drafts;
  const scopeNow = useRef(to);
  scopeNow.current = to;
  // Whether the kept files are back, so that what the composer has can be kept.
  const [restored, setRestored] = useState(!keep);
  const mounted = useRef(false);
  const restoring = useRef(false);

  const patch = useCallback((key: string, p: Partial<Draft>) => {
    setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...p } : d)));
  }, []);

  const start = useCallback(
    (d: Draft, to: string) => {
      uploads.current.get(d.key)?.abort();
      const up = api.uploadAttachment(JSON.parse(to), d.file, d.name, (progress) =>
        patch(d.key, { progress }),
      );
      const entry: Upload = { to, ...up };
      uploads.current.set(d.key, entry);
      patch(d.key, { status: "uploading", progress: 0, error: undefined, attachment: undefined });
      up.promise.then(
        (attachment) => {
          entry.attachment = attachment;
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

  // Files upload to the composer's scope: kept ones that come back, and
  // copies for another project (the home page's picker).
  useEffect(() => {
    if (!to) return;
    for (const d of drafts) {
      if (uploads.current.get(d.key)?.to !== to) start(d, to);
    }
  }, [to, drafts, start]);

  // The kept files, once, as the composer opens: those this page had, which
  // may not all have fitted in the browser's storage, or else the stored ones.
  useEffect(() => {
    mounted.current = true;
    if (keep && !restoring.current) {
      restoring.current = true;
      const back = (files: DraftFile[]) => {
        if (!mounted.current) return;
        const have = new Set(live.current.map((d) => d.key));
        const kept = files
          .filter((f) => !have.has(f.key))
          .map((f): Draft => {
            // Uploaded while the page was here before.
            const done = uploads.current.get(f.key);
            const attachment = done?.to === scopeNow.current ? done?.attachment : undefined;
            return {
              ...f,
              preview: preview(f.file),
              status: attachment ? "ready" : "uploading",
              progress: attachment ? 1 : 0,
              attachment,
            };
          });
        setDrafts((ds) => [...kept, ...ds]);
        setRestored(true);
      };
      const here = pageFiles(keep);
      if (here) back(here);
      else void loadFiles(keep).then(back);
    }
    return () => {
      mounted.current = false;
    };
  }, [keep]);

  useEffect(() => {
    if (keep && restored)
      setPageFiles(
        keep,
        drafts.map(({ key, name, file }) => ({ key, name, file })),
      );
  }, [keep, restored, drafts]);

  // Leaving the page cancels uploads under way and frees the previews. Kept
  // drafts hold on to their finished uploads, for when the page comes back,
  // unless a message took them.
  useEffect(
    () => () => {
      for (const [key, u] of uploads.current) {
        if (keep && u.attachment && !u.taken) continue;
        u.abort();
        uploads.current.delete(key);
      }
      for (const d of live.current) if (d.preview) URL.revokeObjectURL(d.preview);
    },
    [keep],
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
          preview: preview(file),
          status: "uploading",
          progress: 0,
        });
      }
      if (!added.length) return;
      setDrafts((ds) => [...ds, ...added]);
      if (to) for (const d of added) start(d, to);
      // Kept when it fits; otherwise for as long as the page is open.
      if (keep) for (const d of added) void saveFile(keep, d);
    },
    [to, start, keep],
  );

  const remove = useCallback(
    (key: string) => {
      uploads.current.get(key)?.abort();
      uploads.current.delete(key);
      if (keep) void deleteFile(keep, key);
      setDrafts((ds) => {
        const d = ds.find((x) => x.key === key);
        if (d?.preview) URL.revokeObjectURL(d.preview);
        return ds.filter((x) => x.key !== key);
      });
    },
    [keep],
  );

  const retry = useCallback(
    (key: string) => {
      const d = live.current.find((x) => x.key === key);
      if (d && to) start(d, to);
    },
    [to, start],
  );

  /**
   * Waits for the uploads and empties the composer, handing over the files
   * and their previews. With `stay`, the files stay in it, as a start form's
   * do until the session page replaces it, and it frees their previews as it
   * goes. `restore` puts them back if sending fails.
   */
  const take = useCallback(
    async ({ stay = false } = {}): Promise<{ files: SentFiles; restore: () => void }> => {
      const taken = live.current;
      const sent = taken.map((d) => uploads.current.get(d.key));
      const results = await Promise.allSettled(
        sent.map((u) => u?.promise ?? Promise.reject(new Error("not uploaded"))),
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
      // Files that were still uploading have changed since: they go by their keys, and come back
      // uploaded.
      const keys = new Set(taken.map((d) => d.key));
      const uploaded = taken.map(
        (d, i): Draft => ({ ...d, status: "ready", progress: 1, attachment: attachments[i] }),
      );
      if (stay) {
        // Their uploads stay too, so that the files don't upload again.
        for (const u of sent) if (u) u.taken = true;
      } else {
        for (const key of keys) uploads.current.delete(key);
        setDrafts((ds) => ds.filter((d) => !keys.has(d.key)));
      }
      return {
        files: { attachments, previews },
        restore: () => {
          taken.forEach((d, i) => {
            const u = sent[i];
            if (!u) return;
            u.taken = false;
            uploads.current.set(d.key, u);
          });
          if (!stay) setDrafts((ds) => [...uploaded, ...ds]);
        },
      };
    },
    [],
  );

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
  disabled,
  className,
}: {
  onFiles: (files: File[]) => void;
  disabled?: boolean;
  className?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <IconButton
        type="button"
        label="Attach files"
        title="Attach files (or paste, or drop them here)"
        disabled={disabled}
        onClick={() => input.current?.click()}
        // A row of controls that runs short on a phone squeezes the others first.
        className={cn("shrink-0", className)}
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
  if (type.startsWith("video/")) return <FilePlay className={c} />;
  if (/zip|gzip|x-tar|x-7z|x-rar/.test(type)) return <FileArchive className={c} />;
  if (type.startsWith("text/") || /json|yaml|toml|xml|pdf/.test(type)) return <FileText className={c} />;
  return <File className={c} />;
}

/** The files waiting in a composer. */
export function DraftTray({
  drafts,
  onRemove,
  onRetry,
  readOnly,
  className,
}: {
  drafts: Draft[];
  onRemove: (key: string) => void;
  onRetry: (key: string) => void;
  /** A start form's files as it starts: they go with the session, so they can't be removed. */
  readOnly?: boolean;
  className?: string;
}) {
  if (!drafts.length) return null;
  return (
    <ul className={cn("flex flex-wrap gap-2", className)} aria-label="Attached files">
      {drafts.map((d) => (
        <DraftChip
          key={d.key}
          d={d}
          onRemove={readOnly ? undefined : () => onRemove(d.key)}
          onRetry={() => onRetry(d.key)}
        />
      ))}
    </ul>
  );
}

function DraftChip({ d, onRemove, onRetry }: { d: Draft; onRemove?: () => void; onRetry: () => void }) {
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
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove ${d.name}`}
          className="absolute top-1 right-1 flex size-5 items-center justify-center rounded-full bg-ink/80 text-canvas opacity-0 transition-opacity duration-150 group-hover/chip:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100"
        >
          <X className="size-3" />
        </button>
      )}
    </li>
  );
}

/**
 * An image a message carries, which a click shows at full size, among the
 * other pictures of the transcript.
 */
function ImageFile({ file, src, download }: { file: G.Attachment; src?: string; download: boolean }) {
  const { ref, open, lightbox } = useLightbox<HTMLButtonElement>(src ? { file, src, download } : undefined);
  return (
    <>
      <button
        ref={ref}
        type="button"
        onClick={open}
        aria-label={`View ${file.name}`}
        className="cursor-zoom-in overflow-hidden rounded-lg border border-line bg-raised/50 transition-[border-color] duration-150 hover:border-line-strong"
      >
        <img
          src={src}
          alt={file.name}
          width={file.width}
          height={file.height}
          loading="lazy"
          decoding="async"
          className="h-36 w-auto max-w-[min(100%,20rem)] object-cover"
        />
      </button>
      {lightbox}
    </>
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
  const [playing, setPlaying] = useState<G.Attachment>();
  const src = (f: G.Attachment) =>
    previews?.[f.id] ?? (sessionId ? attachmentUrl(sessionId, f.id) : undefined);
  const playingSrc = playing && src(playing);
  // The session that has the files, once the message is sent.
  const sent = pending ? undefined : sessionId;
  const images = files.filter((f) => previews?.[f.id] || showsInline(f.media_type));
  // Videos play in a lightbox of their own once they are sent: the arrow
  // keys and fingers in it are the player's, not steps to other pictures.
  const videos = sent ? files.filter((f) => !images.includes(f) && playable(f.media_type)) : [];
  const others = files.filter((f) => !images.includes(f) && !videos.includes(f));
  const chip =
    "flex h-12 w-60 max-w-full items-center gap-2.5 rounded-lg border border-line bg-raised/50 px-3";
  const chipBody = (f: G.Attachment): ReactNode => (
    <>
      <FileIcon type={f.media_type} className="size-5 text-ink-3" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] text-ink">{f.name}</div>
        <div className="font-mono text-[10.5px] text-ink-4">{bytes(f.size)}</div>
      </div>
    </>
  );
  const chipHint =
    "size-3.5 shrink-0 text-ink-4 opacity-0 transition-opacity duration-150 group-hover/file:opacity-100";
  return (
    <div className="flex flex-wrap items-start gap-2">
      {images.map((f) => (
        <ImageFile key={f.id} file={f} src={src(f)} download={!pending} />
      ))}
      {videos.map((f) => (
        <button
          key={f.id}
          type="button"
          onClick={() => setPlaying(f)}
          aria-label={`Play ${f.name}`}
          title={`Play ${f.name}`}
          className={cn(
            chip,
            "group/file text-left transition-[border-color] duration-150 hover:border-line-strong",
          )}
        >
          {chipBody(f)}
          <Play className={chipHint} />
        </button>
      ))}
      {others.map((f) =>
        sent ? (
          <a
            key={f.id}
            href={attachmentUrl(sent, f.id)}
            download={f.name}
            title={`Download ${f.name}`}
            className={cn(chip, "group/file transition-[border-color] duration-150 hover:border-line-strong")}
          >
            {chipBody(f)}
            <Download className={chipHint} />
          </a>
        ) : (
          <div key={f.id} className={chip}>
            {chipBody(f)}
          </div>
        ),
      )}
      {playing && playingSrc && (
        <Lightbox
          items={[{ file: playing, src: playingSrc, download: !pending }]}
          onClose={() => setPlaying(undefined)}
        />
      )}
    </div>
  );
}
