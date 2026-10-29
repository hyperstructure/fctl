// Which GitHub App review agents post their reviews as, on each account a workspace works in: the
// instance's review app (fctl-review) where it's installed, so that people tell its findings apart
// from what agents write as the instance's GitHub App (fctl-sh), which posts them where it isn't.
// The workspace's admins install it from here, on GitHub, which brings them back
// (`api::review_app`). Nothing to connect: fctl uses it only on what the workspace connected.

import { useQuery } from "@tanstack/react-query";
import { Bot, Check, Settings2 } from "lucide-react";
import { useId, useState } from "react";
import { GithubAvatar } from "@/components/github-avatar";
import { Banner, buttonClass } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys } from "@/lib/queries";
import { sized } from "@/lib/review-bots";

/** Where the review app is among the workspace's accounts. */
export function useReviewApp(workspace: string | undefined) {
  return useQuery({
    queryKey: keys.workspaceReviewApp(workspace ?? ""),
    queryFn: () => workspaces.reviewApp(workspace ?? ""),
    enabled: !!workspace,
    staleTime: 60_000,
  });
}

/** A GitHub App's logo in a rounded square, as GitHub shows apps'; a bot when it doesn't load. */
export function AppAvatar({
  app,
  size = 16,
  className,
}: {
  app: G.GithubAppRef;
  size?: number;
  className?: string;
}) {
  const [broken, setBroken] = useState<string>();
  const box = cn("inline-block shrink-0 rounded-[24%] bg-raised align-[-0.2em]", className);
  if (!app.avatar_url || broken === app.avatar_url) {
    return (
      <span
        aria-hidden="true"
        className={cn(box, "inline-flex items-center justify-center border border-line text-ink-4")}
        style={{ width: size, height: size }}
      >
        <Bot className="size-[62%]" />
      </span>
    );
  }
  return (
    <img
      src={sized(app.avatar_url, size * 2)}
      alt=""
      width={size}
      height={size}
      className={cn(box, "object-cover")}
      style={{ width: size, height: size }}
      referrerPolicy="no-referrer"
      onError={() => setBroken(app.avatar_url)}
    />
  );
}

/** An app named in a sentence, with its logo. */
export function AppName({ app }: { app: G.GithubAppRef }) {
  return (
    <span className="inline-flex items-baseline gap-1.5 whitespace-nowrap font-medium text-ink-2">
      <AppAvatar app={app} size={16} className="self-center" />
      {app.slug}
    </span>
  );
}

/** "docs", "docs and site", "docs, site and 3 more". */
export function someNames(names: readonly string[]) {
  if (names.length > 3) return `${names.slice(0, 2).join(", ")} and ${names.length - 2} more`;
  return names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : (names[0] ?? "");
}

/**
 * The workspace's GitHub tab's section on the review app: what it is, with a pull request's thread
 * to show it, and on each account the workspace works in, which app posts the reviews there, with
 * the way to install it for the workspace's admins. Nothing without a review app or an account.
 */
export function ReviewAppSection({ w, admin }: { w: G.Workspace; admin: boolean }) {
  const heading = useId();
  const q = useReviewApp(w.id);
  const r = q.data;
  if (!r?.configured || !r.app || !r.github_app || r.accounts.length === 0) return null;
  const { app, github_app: ours } = r;
  const missing = r.accounts.filter((a) => a.state === "not_installed" || a.state === "partly");
  return (
    <section aria-labelledby={heading} className="mt-12">
      <div className="grid items-start gap-x-10 gap-y-6 md:grid-cols-[minmax(0,1fr)_17rem]">
        <div>
          <h3 id={heading} className="font-display text-xl text-ink">
            Reviews on GitHub
          </h3>
          <p className="mt-1 max-w-xl text-[14px] leading-relaxed text-ink-3">
            Review agents post their findings as <AppName app={app} /> and your agents answer them as{" "}
            <AppName app={ours} />, so that each pull request shows who said what. Where {app.slug} isn't
            installed, reviews post as {ours.slug} too.
          </p>
          {!admin && missing.length > 0 && (
            <p className="mt-2 max-w-xl text-[13px] leading-relaxed text-ink-4">
              {w.name}'s admins install {app.slug} from here.
            </p>
          )}
        </div>
        <Thread app={app} ours={ours} />
      </div>
      <ul className="mt-6 border-t border-line">
        {r.accounts.map((a) => (
          <AccountRow key={a.account} w={w} a={a} app={app} ours={ours} admin={admin} />
        ))}
      </ul>
    </section>
  );
}

/** One account: which app posts the reviews there, and for admins, the way to change that on GitHub. */
function AccountRow({
  w,
  a,
  app,
  ours,
  admin,
}: {
  w: G.Workspace;
  a: G.ReviewAppAccount;
  app: G.GithubAppRef;
  ours: G.GithubAppRef;
  admin: boolean;
}) {
  const url = admin ? a.install_url : undefined;
  const line = {
    installed: (
      <>
        {app.slug} reaches every repository {w.name}'s projects use here.
      </>
    ),
    partly: (
      <>
        Except in <span className="font-mono">{someNames(a.unreached)}</span>, which {app.slug} doesn't reach:{" "}
        {a.unreached.length === 1 ? "its" : "their"} reviews post as {ours.slug}.
      </>
    ),
    not_installed: <>{app.slug} isn't installed here.</>,
    unknown: <>GitHub didn't say whether {app.slug} is installed here.</>,
  }[a.state];
  const poster = a.state === "not_installed" ? ours : app;
  return (
    <li className="flex flex-wrap items-center gap-x-3.5 gap-y-2.5 border-b border-line py-3.5">
      <GithubAvatar account={a.account} />
      <div className="min-w-0 flex-1 basis-48">
        <p className="text-[14px] text-ink [overflow-wrap:anywhere]">{a.account}</p>
        <p className="mt-0.5 text-[12px] leading-relaxed text-ink-4">{line}</p>
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-2 max-sm:w-full max-sm:pl-[46px]">
        {a.state !== "unknown" && (
          <span
            className={cn(
              "inline-flex items-center gap-1.5 text-[12.5px]",
              a.state === "installed" ? "text-accent" : "text-ink-3",
            )}
          >
            {a.state === "installed" && <Check className="size-3.5" />}
            Posts as
            <AppAvatar app={poster} size={16} />
            <span className={cn("font-medium", a.state !== "installed" && "text-ink-2")}>{poster.slug}</span>
          </span>
        )}
        {url && (a.state === "installed" || a.state === "unknown") && (
          <a
            href={url}
            aria-label={`Choose the repositories ${app.slug} reaches on ${a.account}, on GitHub`}
            title="Its repositories, on GitHub"
            className="rounded p-1.5 text-ink-4 hover:text-ink"
          >
            <Settings2 className="size-3.5" />
          </a>
        )}
        {url && (a.state === "not_installed" || a.state === "partly") && (
          <Install url={url} app={app} state={a.state} unreached={a.unreached} />
        )}
      </div>
    </li>
  );
}

/** Installs the review app on the account, or gives it the repositories it doesn't reach, on GitHub. */
function Install({
  url,
  app,
  state,
  unreached,
}: {
  url: string;
  app: G.GithubAppRef;
  state: G.ReviewAppState;
  unreached: readonly string[];
}) {
  const label =
    state === "partly"
      ? unreached.length === 1
        ? `Add ${unreached[0]}`
        : `Add ${unreached.length} repositories`
      : `Install ${app.slug}`;
  return (
    // A link that looks like a button: it goes on to GitHub.
    <a href={url} className={buttonClass({ size: "sm", className: "max-w-56" })}>
      <AppAvatar app={app} size={14} />
      <span className="truncate">{label}</span>
    </a>
  );
}

/** A pull request's thread on GitHub, to show what it changes: a finding, and the agent's answer. */
function Thread({ app, ours }: { app: G.GithubAppRef; ours: G.GithubAppRef }) {
  const comment = (who: G.GithubAppRef, did: string, text: string) => (
    <div className="flex items-start gap-2.5 px-3.5 py-3">
      <AppAvatar app={who} size={22} />
      <div className="min-w-0 text-[12px] leading-snug">
        <p className="flex items-center gap-1.5">
          <span className="font-semibold text-ink">{who.slug}</span>
          <span className="rounded-full border border-line px-1.5 text-[10px] leading-[15px] text-ink-3">
            Bot
          </span>
          <span className="text-ink-4">{did}</span>
        </p>
        <p className="mt-1 text-ink-3">{text}</p>
      </div>
    </div>
  );
  return (
    <div
      aria-hidden="true"
      className="overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)] select-none max-md:max-w-sm"
    >
      {comment(app, "reviewed", "Retries never stop when the server is down.")}
      <div className="mx-3.5 border-t border-line" />
      {comment(ours, "replied", "Fixed in 3f2a1c9: it gives up after five tries.")}
    </div>
  );
}

/** How installing the review app went, when GitHub brings the browser back to the GitHub tab. */
export function ReviewAppReturned({
  w,
  outcome,
  account,
}: {
  w: G.Workspace;
  outcome: "review_installed" | "review_asked" | "review_changed" | "review_expired" | "review_failed";
  account?: string;
}) {
  const q = useReviewApp(w.id);
  const r = q.data;
  const app = r?.app?.slug ?? "The review app";
  const ours = r?.github_app?.slug ?? "the GitHub App";
  const a = account ? r?.accounts.find((x) => x.account.toLowerCase() === account.toLowerCase()) : undefined;
  switch (outcome) {
    case "review_installed":
      if (!r) return null;
      if (a?.state === "installed")
        return (
          <Banner tone="ok">
            {app} is installed on {a.account}: its reviews post as {app} from the next review on.
          </Banner>
        );
      if (a?.state === "partly")
        return (
          <Banner tone="info">
            {app} is installed on {a.account}, but doesn't reach {someNames(a.unreached)}, whose reviews still
            post as {ours}. Add {a.unreached.length === 1 ? "it" : "them"} below.
          </Banner>
        );
      if (a?.state === "not_installed")
        return (
          <Banner tone="info">
            GitHub doesn't show {app} on {a.account} yet, so its reviews still post as {ours}. If you
            installed it there, look again in a minute.
          </Banner>
        );
      return <Banner tone="info">{app} changed on GitHub. Below is where it posts reviews now.</Banner>;
    case "review_changed":
      return <Banner tone="info">{app} changed on GitHub. Below is where it posts reviews now.</Banner>;
    case "review_asked":
      return (
        <Banner tone="info">
          GitHub asked {account ? `${account}'s` : "the organization's"} owners to install {app}. Once they
          do, reviews there post as {app}; until then, as {ours}.
        </Banner>
      );
    case "review_expired":
      return (
        <Banner tone="info">
          That link to install {app}
          {account ? ` on ${account}` : ""} ran out. Try again below.
        </Banner>
      );
    case "review_failed":
      return (
        <Banner tone="error">
          Installing {app}
          {account ? ` on ${account}` : ""} didn't work: {w.name} doesn't work in that account now, or you
          aren't one of its admins.
        </Banner>
      );
  }
}
