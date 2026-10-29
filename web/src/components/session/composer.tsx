// Writing to the agent. Enter sends, Shift+Enter breaks the line. Files come
// along with the paperclip, a paste, or a drop anywhere on the page.

import { ArrowUp, Square } from "lucide-react";
import { type KeyboardEvent, useRef, useState } from "react";
import { Spinner, Textarea, TextareaControls, toast } from "@/components/ui";
import type * as G from "@/gen";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import {
  AttachButton,
  DraftTray,
  DropHint,
  type SentFiles,
  useAttachments,
  useFileDrop,
} from "./attachments";

export function Composer({
  status,
  role,
  viewers,
  meId,
  sessionId,
  onSend,
  onInterrupt,
  onTyping,
  onResume,
}: {
  status?: G.SessionStatus;
  role?: G.SessionRole;
  viewers: G.Presence[];
  meId?: string;
  sessionId: string;
  onSend: (text: string, files: SentFiles) => Promise<void>;
  onInterrupt: () => void;
  onTyping: () => void;
  onResume: (text: string, files: SentFiles) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  // A message resumes a stopped session; a failed one needs its owner to resume it.
  const resumes = status === "stopped" || status === "stopping";
  const failed = status === "failed";
  const canWrite = role === "editor" || role === "owner";
  const canSend = canWrite && (!failed || role === "owner");
  const running = status === "running" || status === "provisioning" || status === "preparing";
  const typing = viewers.filter((v) => v.user.id !== meId && v.typing_at && Date.now() - v.typing_at < 6000);
  const files = useAttachments(canSend ? { session: sessionId } : undefined);
  const dragging = useFileDrop((f) => {
    files.add(f);
    ref.current?.focus();
  }, canSend);
  const ready = !!text.trim() || files.drafts.length > 0;

  const submit = async () => {
    const t = text.trim();
    if (!ready || sending) return;
    setSending(true);
    let restore: (() => void) | undefined;
    try {
      // Waits for files still uploading.
      const taken = await files.take();
      restore = taken.restore;
      setText("");
      if (failed) await onResume(t, taken.files);
      else await onSend(t, taken.files);
    } catch (e) {
      setText(t);
      restore?.();
      toast(e instanceof Error ? e.message : "Not sent", "error");
    } finally {
      setSending(false);
      ref.current?.focus();
    }
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void submit();
    }
  };

  if (!canSend) {
    return (
      <div className="rounded-xl border border-line bg-surface/80 px-4 py-3 text-center text-[13px] text-ink-4">
        {canWrite
          ? "This session failed. Its owner can resume it."
          : "You can follow this session. Ask its owner for the editor role to message the agent."}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="h-4 pl-3 text-[11.5px] text-ink-4">
        {typing.length > 0 && (
          <span className="animate-fade">
            {typing.map((v) => v.user.name).join(", ")} {typing.length === 1 ? "is" : "are"} typing…
          </span>
        )}
      </div>
      <div
        className={cn(
          "group relative rounded-2xl border bg-surface/90 shadow-[0_10px_40px_rgb(0_0_0/0.25)] backdrop-blur-xl transition-[border-color,box-shadow] duration-200",
          "border-line-strong focus-within:shadow-[0_12px_48px_rgb(0_0_0/0.4)]",
          dragging ? "border-accent/50" : "focus-within:border-ink-4/60",
        )}
      >
        <DraftTray
          drafts={files.drafts}
          onRemove={files.remove}
          onRetry={files.retry}
          className="px-3 pt-3"
        />
        <Textarea
          ref={ref}
          bare
          value={text}
          minRows={1}
          maxRows={10}
          onChange={(e) => {
            setText(e.target.value);
            onTyping();
          }}
          onKeyDown={onKey}
          onPaste={(e) => {
            const pasted = filesFromPaste(e.clipboardData);
            if (!pasted.length) return;
            e.preventDefault();
            files.add(pasted, { pasted: true });
          }}
          placeholder={
            failed
              ? "Resume the session with a message…"
              : resumes
                ? "Message the agent to resume the session…"
                : running
                  ? "Message the agent (it reads it after this step)…"
                  : "Message the agent…"
          }
          className="block px-4 pt-3.5 text-[14.5px] leading-[1.55] text-ink"
          aria-label="Message the agent"
        />
        <TextareaControls textarea={ref} className="justify-between gap-1.5 px-2.5 pt-1.5 pb-2.5">
          <div className="flex min-w-0 items-center gap-1.5">
            <AttachButton
              onFiles={(f) => {
                files.add(f);
                ref.current?.focus();
              }}
            />
            <span className="pointer-events-none truncate text-[11.5px] text-ink-4 opacity-0 transition-opacity group-focus-within:opacity-100">
              {failed ? "↵ resume" : resumes ? "↵ send and resume · ⇧↵ new line" : "↵ send · ⇧↵ new line"}
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {running && (
              <button
                type="button"
                onClick={onInterrupt}
                className="flex h-8 items-center gap-1.5 rounded-full border border-line-strong bg-raised px-3 text-[12.5px] text-ink-2 transition-colors hover:border-danger/40 hover:text-danger"
              >
                <Square className="size-3 fill-current" /> Interrupt
              </button>
            )}
            <button
              type="button"
              aria-label={failed ? "Resume" : "Send"}
              disabled={!ready || sending}
              onClick={() => void submit()}
              className={cn(
                "flex size-8 items-center justify-center rounded-full transition-all duration-200 ease-[var(--ease-spring)]",
                ready ? "scale-100 bg-ink text-canvas hover:scale-105" : "scale-90 bg-raised text-ink-4",
              )}
            >
              {sending ? <Spinner className="size-4" /> : <ArrowUp className="size-4" />}
            </button>
          </div>
        </TextareaControls>
        <DropHint show={dragging} />
      </div>
    </div>
  );
}
