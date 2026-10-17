// Writing to the agent. Enter sends, Shift+Enter breaks the line.

import { ArrowUp, Square } from "lucide-react";
import { type KeyboardEvent, useRef, useState } from "react";
import { Textarea, toast } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";

export function Composer({
  status,
  role,
  viewers,
  meId,
  onSend,
  onInterrupt,
  onTyping,
  onResume,
}: {
  status?: G.SessionStatus;
  role?: G.SessionRole;
  viewers: G.Presence[];
  meId?: string;
  onSend: (text: string) => Promise<void>;
  onInterrupt: () => void;
  onTyping: () => void;
  onResume: (text?: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const ended = status === "stopped" || status === "failed";
  const canWrite = role === "editor" || role === "owner";
  const running = status === "running" || status === "provisioning" || status === "preparing";
  const typing = viewers.filter((v) => v.user.id !== meId && v.typing_at && Date.now() - v.typing_at < 6000);

  const submit = async () => {
    const t = text.trim();
    if (!t || sending) return;
    setSending(true);
    setText("");
    try {
      if (ended) await onResume(t);
      else await onSend(t);
    } catch (e) {
      setText(t);
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

  if (!canWrite) {
    return (
      <div className="rounded-xl border border-line bg-surface/80 px-4 py-3 text-center text-[13px] text-ink-4">
        You can follow this session. Ask its owner for the editor role to message the agent.
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
          "border-line-strong focus-within:border-accent/45 focus-within:shadow-[0_10px_40px_rgb(0_0_0/0.25),0_0_0_4px_var(--c-accent-soft)]",
        )}
      >
        <Textarea
          ref={ref}
          bare
          value={text}
          minRows={1}
          maxRows={12}
          onChange={(e) => {
            setText(e.target.value);
            onTyping();
          }}
          onKeyDown={onKey}
          placeholder={
            ended
              ? "Resume the session with a message…"
              : running
                ? "Message the agent (it reads it after this step)…"
                : "Message the agent…"
          }
          className="block px-4 pt-3.5 pb-12 text-[14.5px] leading-[1.55] text-ink"
          aria-label="Message the agent"
        />
        <div className="absolute right-2.5 bottom-2.5 flex items-center gap-1.5">
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
            aria-label={ended ? "Resume" : "Send"}
            disabled={!text.trim() || sending}
            onClick={() => void submit()}
            className={cn(
              "flex size-8 items-center justify-center rounded-full transition-all duration-200 ease-[var(--ease-spring)]",
              text.trim() ? "scale-100 bg-ink text-canvas hover:scale-105" : "scale-90 bg-raised text-ink-4",
            )}
          >
            <ArrowUp className="size-4" />
          </button>
        </div>
        <div className="pointer-events-none absolute bottom-3.5 left-4 text-[11.5px] text-ink-4 opacity-0 transition-opacity group-focus-within:opacity-100">
          {ended ? "↵ resume" : "↵ send · ⇧↵ new line"}
        </div>
      </div>
    </div>
  );
}
