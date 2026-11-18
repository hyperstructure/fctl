// A webhook's signing secret: shown once as it's added, then to those who manage it on asking (which
// takes confirming it's them, where the instance asks for that), rotated with a day's overlap; and
// how a receiver checks what fctl posts with it, in a few languages.

import { useMutation } from "@tanstack/react-query";
import { Eye, RefreshCw } from "lucide-react";
import { useState } from "react";
import { CodeBlock } from "@/components/markdown";
import { Banner, Button, CodeBox, Dialog, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { when } from "@/lib/format";
import { type SnippetLanguage, verifySnippet } from "@/lib/webhooks";
import { refreshWebhooks } from "./form";

const LANGUAGES: { value: SnippetLanguage; label: string; lang: string }[] = [
  { value: "node", label: "Node.js", lang: "javascript" },
  { value: "python", label: "Python", lang: "python" },
  { value: "any", label: "Any language", lang: "text" },
];

/** How a receiver checks that a request is fctl's, and where it comes from, before acting on it. */
export function VerifyHow({ workspaceId, projectId }: { workspaceId: string; projectId?: string }) {
  const [language, setLanguage] = useState<SnippetLanguage>("node");
  const chosen = LANGUAGES.find((l) => l.value === language) ?? LANGUAGES[0];
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h4 className="text-[13px] font-semibold text-ink">Check that a request is fctl's</h4>
          <p className="mt-0.5 max-w-xl text-[12.5px] leading-relaxed text-ink-3">
            Before acting on one, your receiver checks its signature with the secret, kept in{" "}
            <code className="font-mono text-[12px]">FCTL_WEBHOOK_SECRET</code>, and where it says it comes
            from. Standard Webhooks' libraries check signatures too.
          </p>
        </div>
        <div
          role="tablist"
          aria-label="Language"
          className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
        >
          {LANGUAGES.map((l) => (
            <button
              key={l.value}
              type="button"
              role="tab"
              aria-selected={language === l.value}
              onClick={() => setLanguage(l.value)}
              className={cn(
                "h-7 rounded-full px-3 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                language === l.value
                  ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                  : "text-ink-3 hover:text-ink",
              )}
            >
              {l.label}
            </button>
          ))}
        </div>
      </div>
      <CodeBlock
        code={verifySnippet(language, workspaceId, projectId)}
        lang={chosen?.lang ?? "text"}
        label={chosen?.label}
      />
    </div>
  );
}

/** The secret, hidden until asked for, and rotating it. */
export function SecretSection({ webhook: w }: { webhook: G.Webhook }) {
  const [secret, setSecret] = useState<G.WebhookSecret>();
  const [rotating, setRotating] = useState(false);
  const show = useMutation({
    mutationFn: () => api.webhookSecret(w.id),
    onSuccess: setSecret,
    onError: (e) => toast(e.message, "error"),
  });
  const rotate = useMutation({
    mutationFn: () => api.rotateWebhookSecret(w.id),
    onSuccess: (s) => {
      setSecret(s);
      setRotating(false);
      refreshWebhooks(w.id);
      toast("Rotated: the new secret signs from now on, and the old one for a day more");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const overlap = secret?.previous_until ?? w.previous_secret_until;
  return (
    <div className="flex flex-col gap-4">
      {secret ? (
        <CodeBox value={secret.secret} />
      ) : (
        <div className="flex items-center gap-2 rounded-md border border-line-strong bg-sunken py-1.5 pr-1.5 pl-3">
          <code className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-ink-4">
            <span aria-hidden="true">whsec_••••••••••••••••••••••••••••••••</span>
            <span className="sr-only">Hidden until you show it</span>
          </code>
          <Button
            size="sm"
            variant="ghost"
            icon={<Eye className="size-3.5" />}
            loading={show.isPending}
            onClick={() => show.mutate()}
          >
            Show
          </Button>
        </div>
      )}
      {overlap && overlap > Date.now() && (
        <Banner tone="info">
          Its previous secret signs requests too until {when(overlap)}, beside the new one: switch your
          receiver to the new one by then.
        </Banner>
      )}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <Button size="sm" icon={<RefreshCw className="size-3.5" />} onClick={() => setRotating(true)}>
          Rotate secret…
        </Button>
        <p className="text-[12.5px] text-ink-3">For when it may have got out, or as a habit.</p>
      </div>
      <Dialog
        open={rotating}
        onClose={() => setRotating(false)}
        title="Rotate its secret?"
        description="A new secret signs its requests from now on. The current one signs them too for a day, so that your receiver switches without missing one; after that, a receiver that checks with it alone refuses them."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setRotating(false)}>
            Cancel
          </Button>
          <Button variant="primary" loading={rotate.isPending} onClick={() => rotate.mutate()}>
            Rotate
          </Button>
        </div>
      </Dialog>
    </div>
  );
}
