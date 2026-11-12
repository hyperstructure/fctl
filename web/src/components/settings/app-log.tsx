// Settings → App log: what the app did in this browser lately (lib/app-log.ts), to copy into a
// report when it misbehaves, as the app on an iPhone's Home Screen does when it comes back black.

import { Check, Copy } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { Button, Section, toast } from "@/components/ui";
import { clearLog, logText, useAppLog } from "@/lib/app-log";

/** The browser and screen the log is from, its first line when copied. */
function about(): string {
  return `${navigator.userAgent}; screen ${window.screen.width}×${window.screen.height} at ${window.devicePixelRatio}x`;
}

export function AppLog() {
  const log = useAppLog();
  const [copied, setCopied] = useState(false);
  const box = useRef<HTMLPreElement>(null);
  const text = logText(log, about());
  // The newest entries are at the end: the box opens there.
  // biome-ignore lint/correctness/useExhaustiveDependencies: it follows the log as it grows
  useLayoutEffect(() => {
    const el = box.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [log.length]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast("Couldn’t copy the log", "error");
    }
  };

  return (
    <Section
      title="App log"
      description="What the app did in this browser lately: how it started, when it went to the background and came back, and what went wrong. It stays in this browser. When the app misbehaves, copy the log into a message about it."
    >
      <div className="flex flex-wrap items-center gap-3">
        <Button
          onClick={copy}
          disabled={!log.length}
          icon={copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
        >
          {copied ? "Copied" : "Copy the log"}
        </Button>
        <Button variant="ghost" onClick={clearLog} disabled={!log.length}>
          Clear
        </Button>
        <span className="font-mono text-[12px] text-ink-4">
          {log.length === 1 ? "1 entry" : `${log.length} entries`}
        </span>
      </div>
      {log.length ? (
        <pre
          ref={box}
          className="mt-4 max-h-[60vh] overflow-auto rounded-md border border-line bg-sunken p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-words text-ink-2"
        >
          {text}
        </pre>
      ) : (
        <p className="mt-4 text-[13.5px] text-ink-3">Nothing yet. The app adds to it as it runs.</p>
      )}
    </Section>
  );
}
