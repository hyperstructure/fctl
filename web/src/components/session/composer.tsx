// Writing to the agent. Enter sends, Shift+Enter breaks the line, Escape leaves
// the box for the page's keys, and r, on the page, comes back to it. Files come
// along with the paperclip, a paste, or a drop anywhere on the page. In an
// earlier session of an autopilot's own, a message asks first whether it goes
// to the session the autopilot works in now (moved-on.tsx).

import { ArrowUp, Square } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useRef, useState } from "react";
import { Spinner, Textarea, TextareaControls, toast } from "@/components/ui";
import type * as G from "@/gen";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { useShortcut } from "@/lib/shortcuts";
import {
  AttachButton,
  DraftTray,
  DropHint,
  type SentFiles,
  useAttachments,
  useFileDrop,
} from "./attachments";
import { type MovedOn, MovedOnCard, MovedOnLine } from "./moved-on";

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
  choices,
  onTakeOver,
  limitOf,
  movedOn,
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
  /** The model and effort pickers, beside the paperclip, for the session's owner. */
  choices?: ReactNode;
  /**
   * While it waits for its subscription's limit to reset, and you may move it to yours: asks to.
   * A message keeps it on the subscription it runs on.
   */
  onTakeOver?: () => void;
  /** Whose subscription's limit it waits for, when not yours. */
  limitOf?: string;
  /** An earlier session of an autopilot's own: where it works now, which a message goes to instead. */
  movedOn?: MovedOn;
}) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState<"here" | "there">();
  // The session whose earlier-session card is open, and the one sent to anyway: the page stays as
  // it goes to another session, and they are this one's.
  const [asking, setAsking] = useState<string>();
  const [anyway, setAnyway] = useState<string>();
  // Each Enter while it asks takes the focus back to the card's first way.
  const [asked, setAsked] = useState(0);
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
  useShortcut("reply", () => ref.current?.focus());
  // The card closes with the message (once it's emptied, unless on its way) and with the session:
  // a message typed afresh asks afresh. Sent here anyway, it asks no more until the reader leaves.
  if (asking !== undefined && ((!ready && !sending) || asking !== sessionId)) setAsking(undefined);
  if (anyway !== undefined && anyway !== sessionId) setAnyway(undefined);
  const asks = !!movedOn && anyway !== sessionId;
  const askOpen = asks && asking === sessionId;
  const keepEditing = () => {
    setAsking(undefined);
    ref.current?.focus();
  };

  /** Sends it here, or, from an earlier session of the autopilot's, where it works now. */
  const submit = async (to?: "here" | "there") => {
    const t = text.trim();
    if (!ready || sending) return;
    if (asks && !to) {
      setAsking(sessionId);
      setAsked((n) => n + 1);
      return;
    }
    const where = to ?? "here";
    setSending(where);
    let restore: (() => void) | undefined;
    try {
      // Waits for files still uploading.
      const taken = await files.take();
      restore = taken.restore;
      setText("");
      if (where === "there" && movedOn) await movedOn.send(t, taken.files);
      else if (failed) await onResume(t, taken.files);
      else await onSend(t, taken.files);
      if (where === "here" && movedOn) setAnyway(sessionId);
      setAsking(undefined);
    } catch (e) {
      setText(t);
      restore?.();
      toast(e instanceof Error ? e.message : "Not sent", "error");
    } finally {
      setSending(undefined);
      ref.current?.focus();
    }
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      // The card it asks in goes first, back to the message; then out of the box, where the
      // page's keys work again.
      if (askOpen) setAsking(undefined);
      else e.currentTarget.blur();
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
      {askOpen && movedOn && (
        <MovedOnCard
          to={movedOn}
          nudge={asked}
          sending={sending}
          resume={failed}
          onThere={() => void submit("there")}
          onHere={() => void submit("here")}
          onClose={keepEditing}
        />
      )}
      <div className="flex h-4 min-w-0 items-center gap-1.5 pl-3 text-[11.5px] text-ink-4">
        {typing.length > 0 ? (
          <span className="animate-fade truncate">
            {typing.map((v) => v.user.name).join(", ")} {typing.length === 1 ? "is" : "are"} typing…
          </span>
        ) : movedOn && !askOpen ? (
          <MovedOnLine to={movedOn} />
        ) : (
          onTakeOver && (
            <span className="animate-fade flex min-w-0 items-center gap-1.5">
              <span className="truncate">
                {limitOf ? `${limitOf}'s` : "This"} subscription is at its limit.
              </span>
              <button
                type="button"
                onClick={onTakeOver}
                className="shrink-0 rounded font-medium text-ink-2 underline decoration-ink-4/60 underline-offset-2 transition-colors hover:text-ink hover:decoration-ink"
              >
                Continue on mine
              </button>
            </span>
          )
        )}
      </div>
      <div
        data-composer=""
        className={cn(
          "group relative rounded-2xl border bg-surface/90 shadow-[0_10px_40px_rgb(0_0_0/0.25)] backdrop-blur-xl transition-[border-color,box-shadow] duration-200",
          "border-line-strong focus-within:shadow-[0_10px_44px_rgb(0_0_0/0.3)]",
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
            {choices}
            <span
              className={cn(
                "pointer-events-none truncate text-[11.5px] text-ink-4 opacity-0 transition-opacity group-focus-within:opacity-100",
                // A phone has room for the pickers and the buttons only.
                choices && "max-sm:hidden",
              )}
            >
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
              disabled={!ready || !!sending}
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
