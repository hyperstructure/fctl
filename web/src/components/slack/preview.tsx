// What fctl looks like in Slack, drawn with the app's own tokens so that it follows the theme: a
// thread that started a session (the card fctl keeps current in place, the agent's question with its
// answers, a reply that steers it), the direct message a notification is, and the post a project's
// event makes in a channel. Decoration: everything they show, the words beside them say too.

import { mark } from "@hyperstructure/design/brand";
import { CircleCheck, Eye, Hand, LoaderCircle, MessageCircle, Settings2 } from "lucide-react";
import type { ReactNode } from "react";
import { ChecksIcon, PullRequestIcon, SlackIcon } from "@/components/ui";
import { cn } from "@/lib/cn";
import { clockOf } from "@/lib/schedules";

/** The preview's card, as the app's previews are drawn ([explaining](design docs): Previews). */
const FRAME =
  "overflow-hidden rounded-xl border border-line bg-surface text-left shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]";

/** fctl's app as Slack shows it: its icon in a rounded square. */
function AppIcon({ size = 32 }: { size?: number }) {
  return (
    <svg viewBox={mark.viewBox} width={size} height={size} className="shrink-0 rounded-md" aria-hidden="true">
      <rect width="256" height="256" rx={mark.radius} fill={mark.background} />
      <path d={mark.path} fill={mark.foreground} fillRule="evenodd" />
    </svg>
  );
}

/** A person as Slack shows them without a picture: their initials in a rounded square. */
function PersonIcon({
  initials,
  tone,
  size = 32,
}: {
  initials: string;
  tone: "pink" | "blue";
  size?: number;
}) {
  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center rounded-md font-semibold text-highlight-ink",
        tone === "pink" ? "bg-pink" : "bg-blue",
      )}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.36) }}
    >
      {initials}
    </span>
  );
}

/** A message: who, when, and what, as Slack lays one out. */
function Message({
  icon,
  name,
  app,
  at,
  children,
}: {
  icon: ReactNode;
  name: string;
  app?: boolean;
  at: string;
  children: ReactNode;
}) {
  return (
    <div className="flex gap-2.5 px-3.5 py-2">
      {icon}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-1.5 leading-5">
          <span className="truncate text-[13px] font-bold text-ink">{name}</span>
          {app && (
            <span className="rounded-[3px] bg-ink/10 px-1 text-[9.5px] leading-[14px] font-semibold text-ink-3">
              App
            </span>
          )}
          <span className="shrink-0 text-[11px] text-ink-4">{at}</span>
        </div>
        <div className="mt-0.5 text-[12.5px] leading-[1.55] text-ink-2">{children}</div>
      </div>
    </div>
  );
}

/** A button of a Slack message. */
function SlackButton({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-7 items-center rounded-md border border-line-strong bg-surface px-2.5 text-[12px] font-semibold text-ink",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** A mention, as Slack marks one. */
const Mention = ({ children }: { children: ReactNode }) => (
  <span className="rounded-[3px] bg-blue-soft px-0.5 font-medium text-blue">{children}</span>
);

/** Context under a block: small and quiet. */
const Context = ({ children }: { children: ReactNode }) => (
  <p className="mt-1 text-[11.5px] leading-snug text-ink-3">{children}</p>
);

const PLAN: { title: string; done?: boolean; now?: boolean }[] = [
  { title: "Reproduce the redirect loop on Safari", done: true },
  { title: "Set SameSite on the session cookie", done: true },
  { title: "Open a pull request", done: true },
  { title: "Keep /signin as a redirect to /login", now: true },
];

/**
 * A thread in a channel that started a session: someone mentions fctl, fctl's card answers at once
 * and stays current (how it stands in words, the agent's plan as Slack draws one, its pull request
 * with its checks, the one button that opens it), the agent asks with a button per answer, and a
 * reply steers it. `channel` and the project's are this workspace's own where it has them.
 */
export function SlackThreadPreview({
  app = "fctl",
  channel = "web-team",
  project = "Web",
  slug = "web",
  className,
}: {
  /** fctl's app, as people mention it. */
  app?: string;
  channel?: string;
  project?: string;
  slug?: string;
  className?: string;
}) {
  return (
    <div aria-hidden="true" className={cn("relative select-none", className)}>
      <div className={FRAME}>
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5">
          <span className="text-[13px] font-bold text-ink">Thread</span>
          <span className="truncate text-[12px] text-ink-4">#{channel}</span>
          <SlackIcon className="ml-auto size-3.5 shrink-0" />
        </div>
        <div className="py-1.5">
          <Message icon={<PersonIcon initials="AL" tone="pink" />} name="Ada Lovelace" at={clockOf(10, 42)}>
            <Mention>@{app}</Mention> Signing in on Safari sends people back to /login. Can you fix the
            redirect loop?
          </Message>
          <div className="flex items-center gap-2 px-3.5 py-1 text-[11px] text-ink-4">
            3 replies
            <span className="h-px flex-1 bg-line" />
          </div>
          <Message icon={<AppIcon />} name={app} app at={clockOf(10, 42)}>
            <p className="font-bold text-blue">Fix the login redirect loop</p>
            <p className="flex items-center gap-1.5 text-ink-2">
              <Settings2 className="size-3.5 shrink-0 text-ink-3" />
              Running the tests
            </p>
            <Context>
              {project} · {slug}#12 · for Ada Lovelace
            </Context>
            <div className="mt-2 rounded-lg border border-line px-2.5 py-2">
              <p className="text-[11.5px] font-semibold text-ink">3 of 4 done</p>
              <ul className="mt-1 flex flex-col gap-1">
                {PLAN.map((t) => (
                  <li key={t.title} className="flex items-center gap-1.5 text-[11.5px] leading-4">
                    {t.done ? (
                      <CircleCheck className="size-3.5 shrink-0 text-checks-passed" />
                    ) : (
                      <LoaderCircle className="size-3.5 shrink-0 text-blue" />
                    )}
                    <span className={cn("truncate", t.done ? "text-ink-3" : "text-ink")}>{t.title}</span>
                  </li>
                ))}
              </ul>
            </div>
            <p className="mt-2 flex min-w-0 items-center gap-1.5 text-[12px]">
              <PullRequestIcon state="open" className="size-3.5 shrink-0 text-pr-open" />
              <span className="shrink-0 text-blue">{slug}#45</span>
              <span className="text-ink-4">·</span>
              <ChecksIcon state="passed" className="size-3.5 shrink-0 text-checks-passed" />
              <span className="truncate text-ink-3">checks passed</span>
            </p>
            <div className="mt-2">
              <SlackButton>Open in fctl</SlackButton>
            </div>
          </Message>
          <Message icon={<AppIcon />} name={app} app at={clockOf(10, 58)}>
            <p className="flex items-center gap-1.5 font-bold text-ink">
              <Hand className="size-3.5 shrink-0 text-ink-3" />
              The agent asks
            </p>
            <p>Remove the old /signin page, or keep it as a redirect to /login?</p>
            <div className="mt-2 flex flex-wrap gap-1.5">
              <SlackButton>Keep the redirect</SlackButton>
              <SlackButton>Remove it</SlackButton>
            </div>
            <Context>Click an answer, or reply in this thread.</Context>
          </Message>
          <Message icon={<PersonIcon initials="AL" tone="pink" />} name="Ada Lovelace" at={clockOf(11, 1)}>
            Keep the redirect: the iOS app still links there.
          </Message>
        </div>
      </div>
    </div>
  );
}

/**
 * The direct message a notification is, from fctl's app: what waits for the reader, the agent's last
 * words, and the session a click away.
 */
export function SlackDmPreview({
  app = "fctl",
  reference = "web#12",
  className,
}: {
  app?: string;
  reference?: string;
  className?: string;
}) {
  return (
    <div aria-hidden="true" className={cn("relative select-none", className)}>
      <div className={FRAME}>
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5">
          <AppIcon size={16} />
          <span className="text-[13px] font-bold text-ink">{app}</span>
          <span className="truncate text-[12px] text-ink-4">Direct messages</span>
        </div>
        <div className="py-1.5">
          <Message icon={<AppIcon />} name={app} app at={clockOf(11, 6)}>
            <p>
              <MessageCircle className="mr-1.5 inline size-3.5 align-[-2px] text-ink-3" />
              <span className="font-bold text-blue">{reference}</span> waits for your reply
            </p>
            <p className="text-ink-2">Fix the login redirect loop</p>
            <p className="mt-1.5 border-l-[3px] border-line-strong pl-2.5 text-ink-3">
              The session cookie lost its SameSite setting on Safari: I set it again and kept /signin as a
              redirect. The pull request is ready for review.
            </p>
            <div className="mt-2">
              <SlackButton>Open in fctl</SlackButton>
            </div>
          </Message>
        </div>
      </div>
    </div>
  );
}

/**
 * What a project's event looks like in its channel, as fctl's app posts one: what happened, the
 * session's title under it, and its button.
 */
export function SlackEventPreview({
  app = "fctl",
  channel,
  reference,
  className,
}: {
  app?: string;
  channel: string;
  /** The session, as `shop#12`. */
  reference: string;
  className?: string;
}) {
  const [slug] = reference.split("#");
  return (
    <div aria-hidden="true" className={cn("relative select-none", className)}>
      <div className={FRAME}>
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2">
          <span className="truncate text-[12.5px] font-bold text-ink">#{channel}</span>
        </div>
        <div className="py-1">
          <Message icon={<AppIcon size={28} />} name={app} app at={clockOf(14, 20)}>
            <p>
              <Eye className="mr-1.5 inline size-3.5 align-[-2px] text-ink-3" />
              {reference} is ready for review: <span className="text-blue">{slug}#45</span>
            </p>
            <Context>Fix the login redirect loop</Context>
            <div className="mt-1.5">
              <SlackButton>Open in fctl</SlackButton>
            </div>
          </Message>
        </div>
      </div>
    </div>
  );
}
