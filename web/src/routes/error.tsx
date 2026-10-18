import type { ErrorComponentProps } from "@tanstack/react-router";
import { useRouter } from "@tanstack/react-router";
import { Check, Copy } from "lucide-react";
import { useRef, useState } from "react";
import { StreamScreen } from "@/components/stream-screen";
import { Button } from "@/components/ui";
import { ApiFailure } from "@/lib/api";
import { redact, safeAddress } from "@/lib/redact";

/** What the error page says: the glyphs, the headline and a sentence. */
export function explain(error: unknown): { code: string; title: string; detail: string; bug: boolean } {
  if (error instanceof ApiFailure) {
    if (error.status === 0) {
      return {
        code: "off",
        title: "Can’t reach fctl.",
        detail:
          "The app couldn’t connect to its server. Check your connection. Your sessions keep running on their hosts.",
        bug: false,
      };
    }
    if (error.status >= 500) {
      return {
        code: String(error.status),
        title: "The factory lost power.",
        detail: `fctl’s server answered ${error.status} while loading this page. Your sessions keep running on their hosts.`,
        bug: false,
      };
    }
    return { code: String(error.status), title: "That didn’t work.", detail: error.message, bug: false };
  }
  return {
    code: "bug",
    title: "This page hit a bug.",
    detail: "Something in the app went wrong while showing it. Your sessions keep running on their hosts.",
    bug: true,
  };
}

const STILL = [
  "Still down. It isn’t you: the server keeps failing.",
  "Still down. Give it a minute, then try again.",
  "Still down. Agents keep working on their hosts meanwhile.",
];

/**
 * The router's error page: a request failed (the server, the network) or a
 * page crashed. "Try again" turns the power back on and asks the server
 * whether it answers; if it does, the page loads again.
 */
export function ErrorPage({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  const what = explain(error);
  const [power, setPower] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [copied, setCopied] = useState(false);
  const tries = useRef(0);

  const retry = async () => {
    setBusy(true);
    setPower(true);
    setStatus("Turning it back on…");
    const started = performance.now();
    let ok = false;
    try {
      ok = (await fetch("/api/v1/health", { cache: "no-store" })).ok;
    } catch {}
    // Let the lights come back before the verdict.
    await new Promise((r) => setTimeout(r, Math.max(0, 1500 - (performance.now() - started))));
    if (ok) {
      setStatus("It’s back.");
      reset();
      await router.invalidate();
      return;
    }
    setPower(false);
    setStatus(
      what.code === "off"
        ? "Still can’t reach it. Check your connection."
        : (STILL[Math.min(tries.current++, STILL.length - 1)] ?? ""),
    );
    setBusy(false);
  };

  // What broke, for a bug report: shown and copied without anything that
  // looks like a secret, and without the address's query and fragment, where
  // invitation and reset links carry their tokens.
  const details = redact(
    error instanceof Error ? `${error.name}: ${error.message}\n${error.stack ?? ""}`.trim() : String(error),
  ).slice(0, 4000);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(
        `${safeAddress(location.href)}\n${navigator.userAgent}\n\n${details}`,
      );
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {}
  };

  return (
    <StreamScreen code={what.code} power={power}>
      <h1 className="font-display text-[clamp(30px,4.4vw,48px)] font-medium leading-[1.08] tracking-[-0.025em] text-ink">
        <span className="sr-only">Error{/^\d+$/.test(what.code) ? ` ${what.code}` : ""}: </span>
        {what.title}
      </h1>
      <p className="mt-4 text-[15.5px] leading-relaxed text-ink-2">{what.detail}</p>
      <div className="mt-8 flex flex-wrap justify-center gap-3">
        <Button variant="primary" size="lg" onClick={retry} disabled={busy}>
          Try again
        </Button>
        {/* A full load: what broke may be the app's own state. */}
        <a href="/">
          <Button size="lg">Back to the floor</Button>
        </a>
      </div>
      <p className="mt-4 min-h-[1.6em] font-mono text-[12.5px] text-ink-3" role="status">
        {status}
      </p>
      {what.bug && (
        <details className="mx-auto mt-2 max-w-full text-left">
          <summary className="cursor-pointer text-center font-mono text-[12.5px] text-ink-3 hover:text-ink">
            What broke
          </summary>
          <div className="relative mt-3 rounded-md border border-line bg-sunken">
            <pre className="max-h-48 overflow-auto p-3 pr-12 font-mono text-[12px] leading-relaxed text-ink-2 whitespace-pre-wrap">
              {details}
            </pre>
            <button
              type="button"
              onClick={copy}
              aria-label="Copy the details"
              title="Copy the details"
              className="absolute top-2 right-2 inline-flex size-7 items-center justify-center rounded-sm text-ink-3 hover:bg-hover hover:text-ink"
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            </button>
          </div>
        </details>
      )}
    </StreamScreen>
  );
}
