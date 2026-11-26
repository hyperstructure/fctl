// Setting up the instance's Slack app, for its admins: Slack's form to create it from its manifest,
// with this instance's addresses filled in, the manifest to paste instead, the credentials the
// instance takes (and which it has), and what follows. Admin → Slack shows it, and so does a
// workspace's Integrations → Slack to the instance's admins while the instance has no app.

import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight, Check, Minus } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { Mono, Step } from "@/components/claude/guide";
import { Banner, buttonClass, CopyButton, Skeleton, SlackIcon } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys } from "@/lib/queries";

/** The self-hosting guide's part on Slack. */
export const SLACK_GUIDE = "https://fctl.sh/docs/self-hosting#connect-slack-optional";

/** The instance's Slack app, as its admins read it. */
export const useAdminSlack = (enabled = true) =>
  useQuery({ queryKey: keys.adminSlack, queryFn: api.adminSlack, enabled });

/** One of the credentials the instance takes: its name, where Slack shows it, and whether it is set. */
function Credential({ name, set, children }: { name: string; set: boolean; children: ReactNode }) {
  return (
    <li className="flex gap-3 py-2.5">
      <span
        className={cn(
          "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border",
          set ? "border-accent/40 bg-accent-soft text-accent" : "border-line-strong text-ink-4",
        )}
      >
        {set ? <Check className="size-3" strokeWidth={3} /> : <Minus className="size-3" />}
        <span className="sr-only">{set ? "Set" : "Not set"}</span>
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2">
          <code className="font-mono text-[12px] text-ink">{name}</code>
          <span className={cn("text-[12px]", set ? "text-accent" : "text-ink-4")}>
            {set ? "set" : "not set"}
          </span>
        </div>
        <p className="mt-0.5 text-[12.5px] leading-relaxed text-ink-3">{children}</p>
      </div>
    </li>
  );
}

/** The manifest, as YAML or JSON, to paste into Slack's form: in a box that scrolls, with Copy. */
export function ManifestBox({ app }: { app: G.AdminSlack }) {
  const [format, setFormat] = useState<"yaml" | "json">("yaml");
  const text = format === "yaml" ? app.manifest_yaml : JSON.stringify(app.manifest, null, 2);
  return (
    <div className="overflow-hidden rounded-md border border-line-strong bg-sunken">
      <div className="flex items-center gap-1 border-b border-line px-1.5 py-1">
        <fieldset className="flex gap-0.5">
          <legend className="sr-only">The manifest's format</legend>
          {(["yaml", "json"] as const).map((f) => (
            <button
              key={f}
              type="button"
              aria-pressed={format === f}
              onClick={() => setFormat(f)}
              className={cn(
                "rounded-sm px-2 py-0.5 font-mono text-[11.5px] transition-colors",
                format === f
                  ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                  : "text-ink-3 hover:bg-hover hover:text-ink",
              )}
            >
              {f === "yaml" ? "YAML" : "JSON"}
            </button>
          ))}
        </fieldset>
        <CopyButton text={text} label={format === "yaml" ? "Copy YAML" : "Copy JSON"} className="ml-auto" />
      </div>
      {/* Long enough to read through, short enough not to bury the steps: it scrolls. */}
      <pre className="max-h-72 overflow-auto px-3 py-2 font-mono text-[11.5px] leading-relaxed text-ink-2">
        {text}
      </pre>
    </div>
  );
}

/**
 * How the instance gets its Slack app: create it in Slack from its manifest, give the instance its
 * credentials, then connect Slack workspaces. Each step ticks as the instance has what it needs.
 * `standalone` says the last step where no workspace's page is around it.
 */
export function SlackAppSetup({ standalone }: { standalone?: boolean }) {
  const admin = useAdminSlack();
  const [manifest, setManifest] = useState(false);
  const shown = useId();
  if (admin.isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-6 w-64" />
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
      </div>
    );
  if (admin.isError)
    return (
      <Banner tone="error">
        The Slack app's setup didn't load: {admin.error.message}{" "}
        <button type="button" className="underline underline-offset-2" onClick={() => void admin.refetch()}>
          Try again
        </button>
      </Banner>
    );
  const a = admin.data;
  const events = a.signing_secret || a.socket_mode;
  return (
    <ol className="flex flex-col">
      <Step n={1} done={a.configured} title="Create the app in Slack">
        <p>
          Slack's form opens with the app filled in: its name, the least scopes it needs, and this instance's
          addresses
          {a.reachable ? "" : ", with Socket Mode on, since Slack can't reach this instance"}. Choose the
          Slack workspace that owns it, then Create.
        </p>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <a
            href={a.create_url}
            target="_blank"
            rel="noreferrer"
            className={buttonClass({ variant: a.configured ? "secondary" : "primary" })}
          >
            <SlackIcon className="mr-2 size-3.5" />
            Create the Slack app
            <ArrowUpRight className="ml-1.5 size-3.5 opacity-70" />
          </a>
          <button
            type="button"
            aria-expanded={manifest}
            aria-controls={shown}
            onClick={() => setManifest(!manifest)}
            className="text-[12.5px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline"
          >
            {manifest ? "Hide the manifest" : "Or paste its manifest yourself"}
          </button>
        </div>
        {manifest && (
          <div id={shown} className="animate-rise">
            <ManifestBox app={a} />
          </div>
        )}
      </Step>
      <Step n={2} done={a.configured && events} title="Give this instance its credentials">
        <p>
          In the app's settings on Slack, Basic Information shows them under App Credentials. The instance
          needs the first two, and one of the others for Slack's events to reach it.
        </p>
        <ul className="divide-y divide-line border-y border-line">
          <Credential name="SLACK_CLIENT_ID" set={a.configured}>
            The Client ID: Slack's sign-ins that add the app to a Slack workspace and link people's accounts.
          </Credential>
          <Credential name="SLACK_CLIENT_SECRET" set={a.configured}>
            The Client Secret, which goes with it.
          </Credential>
          <Credential name="SLACK_SIGNING_SECRET" set={a.signing_secret}>
            The Signing Secret: Slack's events, button clicks and commands come over HTTP, signed with it.
            {!a.reachable &&
              " Slack can't reach this instance's address, so they can't come this way: use Socket Mode."}
          </Credential>
          <Credential name="SLACK_APP_TOKEN" set={a.socket_mode}>
            Or, for Socket Mode, an app-level token with <Mono>connections:write</Mono>, under App-Level
            Tokens: fctl opens a connection to Slack, for an instance Slack can't reach.
          </Credential>
        </ul>
        <p>
          On your own machine or cluster, set them as environment variables and start the server again. On
          Cloudflare, the client ID goes in the instance file under <Mono>[slack] client_id</Mono>, the others
          with <Mono>cargo xtask secrets</Mono>, then <Mono>cargo xtask deploy</Mono>.{" "}
          <a
            href={SLACK_GUIDE}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-0.5 text-ink-2 underline underline-offset-2 hover:text-ink"
          >
            The self-hosting guide
            <ArrowUpRight className="size-3" />
          </a>
        </p>
      </Step>
      <Step n={3} done={(a.teams?.length ?? 0) > 0} title="Add it to a Slack workspace">
        <p>
          {standalone
            ? "Then a workspace's admins add fctl to Slack from the workspace's Integrations tab: Slack asks them to sign in, and may ask one of its own admins to approve the app."
            : "Then add fctl to Slack here: Slack asks you to sign in, and may ask one of its own admins to approve the app."}
        </p>
      </Step>
    </ol>
  );
}
