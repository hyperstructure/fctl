// A workspace's People tab, for its admins: whether each member's sessions can run on Claude (#627).
// The summary and the pills above the list, a facts line under each person that opens, in place (a
// sheet on a phone), to the whole of it and the way out; an introduction while nobody but you has
// Claude connected; and, where its admins see nothing of it, one quiet line saying so. Never a token,
// its account or its label: the control plane sends none.

import { type UseQueryResult, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, BellRing, ChevronDown, Eye, KeyRound } from "lucide-react";
import { useId } from "react";
import { Avatar, Banner, Button, CodeBox, CopyButton, Skeleton } from "@/components/ui";
import { Sheet } from "@/components/ui/sheet";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import {
  adminTokenCommand,
  blocked,
  detail,
  type Fact,
  facts,
  factsText,
  frequent,
  type Narrow,
  needsAction,
  nobody,
  pills,
  SEATS_HELP,
  settingsLink,
  summary,
} from "@/lib/claude-oversight";
import { cn } from "@/lib/cn";
import { useMedia } from "@/lib/media";
import { keys, useInstance, useMe } from "@/lib/queries";

/** What the workspace's admins see of its members' Claude connections; undefined for members. */
export function useClaudeOverview(w: G.Workspace): UseQueryResult<G.ClaudeOverview> | undefined {
  const admin = w.role === "owner" || w.role === "admin";
  // Control planes before it say nothing of it, and answer nothing.
  const on = admin && !!w.claude_oversight && w.claude_oversight.oversight !== "off";
  const q = useQuery({
    queryKey: keys.workspaceClaude(w.id),
    queryFn: () => workspaces.claude(w.id),
    enabled: on,
  });
  return on ? q : undefined;
}

const TONE: Record<Fact["tone"], string> = { plain: "", warn: "text-amber", danger: "text-danger" };

/** The facts line, each fact in its tone. */
function FactsLine({ list }: { list: Fact[] }) {
  return (
    <>
      {list.map((f, i) => (
        <span key={f.text} className={TONE[f.tone]}>
          {i > 0 && <span className="text-ink-4"> · </span>}
          {f.text}
        </span>
      ))}
    </>
  );
}

/**
 * Above the list: the summary, the pills, and where seats are given. While it loads, their shape;
 * when it fails, why, with Try again, and the people stay listed; while nobody but you has Claude
 * connected, the introduction; where admins see nothing, one quiet line.
 */
export function ClaudeOverviewHeader({
  w,
  overview,
  narrow,
  onNarrow,
}: {
  w: G.Workspace;
  overview: UseQueryResult<G.ClaudeOverview> | undefined;
  narrow: Narrow;
  onNarrow: (n: Narrow) => void;
}) {
  const { data: me } = useMe();
  const settings = w.claude_oversight;
  const admin = w.role === "owner" || w.role === "admin";
  if (!admin || !settings) return null;
  if (settings.oversight === "off") return <Off w={w} instanceAdmin={me?.user.role !== "member"} />;
  if (!overview) return null;
  if (overview.isPending)
    return (
      <div className="mt-8 flex flex-col gap-3" aria-busy="true">
        <span className="sr-only">Looking up Claude connections</span>
        <Skeleton className="h-4 w-[min(100%,26rem)]" />
        <div className="flex gap-1.5">
          <Skeleton className="h-6.5 w-24 rounded-full" />
          <Skeleton className="h-6.5 w-20 rounded-full" />
          <Skeleton className="h-6.5 w-28 rounded-full" />
        </div>
      </div>
    );
  if (overview.isError)
    return (
      <Banner tone="error" className="mt-8">
        Claude connections didn't load: {overview.error.message}{" "}
        <button
          type="button"
          className="underline underline-offset-2"
          onClick={() => void overview.refetch()}
        >
          Try again
        </button>
      </Banner>
    );
  const data = overview.data;
  const members = data.members;
  if (data.oversight === "off") return <Off w={w} instanceAdmin={me?.user.role !== "member"} />;
  const others = members.filter((m) => m.user.id !== me?.user.id);
  if (others.length > 0 && others.every(blocked))
    return <Introduction w={w} overview={data} connected={members.length - members.filter(blocked).length} />;
  const now = Date.now();
  const list = pills(members, data, narrow, now);
  return (
    <div className="mt-8 flex flex-col gap-3">
      <p className="text-[13.5px] text-ink-2" aria-live="polite">
        {summary(members, w.name, now)}
      </p>
      {list.length > 1 && (
        <fieldset className="-mx-6 overflow-x-auto px-6 [scrollbar-width:none]">
          <legend className="sr-only">Show the people of {w.name} who</legend>
          <div className="flex items-center gap-1">
            {list.map((p) => (
              <button
                key={p.narrow}
                type="button"
                aria-pressed={narrow === p.narrow}
                onClick={() => onNarrow(p.narrow)}
                className={cn(
                  "shrink-0 rounded-full px-3 py-1 text-[12.5px] whitespace-nowrap transition-colors pointer-coarse:py-1.5",
                  narrow === p.narrow
                    ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                    : "text-ink-3 hover:bg-hover hover:text-ink",
                )}
              >
                {p.label} <span className="text-ink-4">{p.count}</span>
              </button>
            ))}
          </div>
        </fieldset>
      )}
      <Seats adminTokens={data.admin_tokens} />
    </div>
  );
}

/** Where Claude's seats are given, and, where admin tokens set people's tokens, that they do. */
function Seats({ adminTokens }: { adminTokens: boolean }) {
  return (
    <p className="text-[12px] leading-relaxed text-ink-4">
      Claude Team and Enterprise seats are given in Claude's organization settings (
      <a
        href={SEATS_HELP}
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-0.5 underline underline-offset-2 hover:text-ink-2"
      >
        how
        <ArrowUpRight aria-hidden="true" className="size-3" />
      </a>
      ); each person connects theirs in Settings → Claude
      {adminTokens ? ", or an admin token sets it for them" : ""}.
    </p>
  );
}

/** Admins see nothing of it: one quiet line, and where that's decided. */
function Off({ w, instanceAdmin }: { w: G.Workspace; instanceAdmin: boolean }) {
  const instance = w.claude_oversight?.most === "off";
  return (
    <p className="mt-8 border-y border-line py-3 text-[12.5px] leading-relaxed text-ink-4">
      {instance ? (
        <>
          This instance shows workspace admins nothing of their members' Claude connections
          {instanceAdmin ? (
            <>
              {" "}
              (
              <Link to="/admin" search={{ tab: "settings" }} className="underline underline-offset-2">
                Admin → Settings
              </Link>
              )
            </>
          ) : null}
          .
        </>
      ) : (
        <>
          {w.name}'s admins see nothing of its members' Claude connections. Change that in{" "}
          <Link
            to="/w/$slug"
            params={{ slug: w.slug }}
            search={{ tab: "settings" }}
            className="underline underline-offset-2"
          >
            Settings
          </Link>
          .
        </>
      )}
    </p>
  );
}

/** While nobody but you has Claude connected: what it is, with two made-up rows as its picture. */
function Introduction({
  w,
  overview,
  connected,
}: {
  w: G.Workspace;
  overview: G.ClaudeOverview;
  connected: number;
}) {
  const total = overview.members.length;
  const details = overview.oversight === "details";
  const email = useInstance().data?.features.email;
  const link = settingsLink(window.location.origin);
  return (
    <section className="mt-10 flex animate-rise flex-col gap-8 md:flex-row md:items-start">
      <div className="min-w-0 flex-1">
        <h2 className="font-display text-[28px] leading-[1.1] font-medium tracking-[-0.02em] text-ink">
          Claude for everyone in {w.name}
        </h2>
        <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-ink-3">
          {w.name}'s sessions run on each person's own Claude subscription: {connected} of its {total}{" "}
          {total === 1 ? "person" : "people"} {connected === 1 ? "has" : "have"} connected one. You see
          whether their sessions can run and when their tokens run out, never the tokens.
        </p>
        <ul className="mt-6 flex flex-col gap-3 text-[13.5px] text-ink-2">
          <li className="flex gap-2.5">
            <KeyRound aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ink-3" />
            Each person connects theirs in Settings → Claude: a long-lived token from claude setup-token, or a
            Claude login.
          </li>
          <li className="flex gap-2.5">
            <Eye aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ink-3" />
            {details
              ? "Under each person: what kind it is, its plan, when Claude last answered it and when it runs out."
              : "Under each person: whether their sessions can run, and when their token runs out."}
          </li>
          <li className="flex gap-2.5">
            <BellRing aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ink-3" />
            fctl tells each person before their token runs out and when Claude stops taking it
            {email && w.claude_oversight?.admin_email !== "off"
              ? w.claude_oversight?.admin_email === "each"
                ? ", and emails you each time."
                : ", and emails you once a week while one does."
              : ", and this tab shows you."}
          </li>
        </ul>
        <div className="mt-6 flex flex-wrap items-center gap-3">
          <CopyButton
            text={link}
            label="Copy the link to Settings → Claude"
            className="border border-line-strong px-3 py-1.5 text-[12.5px] text-ink"
          />
          <span className="text-[12.5px] text-ink-4">
            Send it to them: it opens where they connect Claude.
          </span>
        </div>
      </div>
      <Preview />
    </section>
  );
}

/** Two made-up people as the tab will show them, drawn with the facts line's own words. */
function Preview() {
  const now = Date.now();
  const day = 86_400_000;
  const rows: G.MemberClaude[] = [
    {
      user: { id: "usr_preview_grace", name: "Grace Hopper" },
      email: "grace@example.com",
      state: "works",
      runs_out_at: now + 21 * day,
      runs_out_estimated: true,
      connection: {
        kind: "token",
        set_by_admin_token: true,
        plan: "team",
        connected_at: now - 344 * day,
        answered_at: now - 2 * 3_600_000,
      },
    },
    {
      user: { id: "usr_preview_ada", name: "Ada Lovelace" },
      email: "ada@example.com",
      state: "works",
      connection: { kind: "login", plan: "max", connected_at: now - 40 * day, answered_at: now - 5 * 60_000 },
    },
  ];
  return (
    <div
      aria-hidden="true"
      className="w-full shrink-0 select-none rounded-xl border border-line bg-surface p-4 shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)] max-md:max-w-sm md:w-[310px]"
    >
      <p className="text-[12px] text-ink-3">{summary(rows, "Acme", now)}</p>
      <ul className="mt-3 divide-y divide-line border-t border-line">
        {rows.map((m) => (
          <li key={m.user.id} className="flex gap-2.5 py-2.5">
            <Avatar user={m.user} size={24} />
            <div className="min-w-0">
              <div className="text-[13px] text-ink">{m.user.name}</div>
              <div className="text-[11.5px] leading-snug text-ink-4">
                <FactsLine list={facts(m, "Acme", now)} />
              </div>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Under a person's address: their facts line, which opens the whole of it. A skeleton while it loads;
 * nothing when it failed (the header says so) or where admins see nothing.
 */
export function ClaudeFacts({
  w,
  member,
  loading,
  open,
  onToggle,
  panel,
}: {
  w: G.Workspace;
  member: G.MemberClaude | undefined;
  loading: boolean;
  open: boolean;
  onToggle: () => void;
  /** The id of what it opens. */
  panel: string;
}) {
  if (loading) return <Skeleton className="mt-1.5 h-3 w-[min(100%,20rem)]" />;
  if (!member) return null;
  const list = facts(member, w.name);
  return (
    <button
      type="button"
      aria-expanded={open}
      aria-controls={panel}
      onClick={onToggle}
      className="-mx-1 mt-0.5 flex max-w-full items-start gap-1 rounded px-1 text-left text-[12px] leading-snug text-ink-3 hover:bg-hover pointer-coarse:py-1"
    >
      <span className="sr-only">Claude: </span>
      <span className="min-w-0">
        <FactsLine list={list} />
      </span>
      <ChevronDown
        aria-hidden="true"
        className={cn(
          "mt-[1px] size-3.5 shrink-0 text-ink-4 transition-transform duration-200 motion-reduce:transition-none",
          open && "rotate-180",
        )}
      />
    </button>
  );
}

/** The whole of someone's connection, and what to do about it: in place, or in a sheet on a phone. */
export function ClaudeDetail({
  w,
  overview,
  member,
  id,
  onClose,
}: {
  w: G.Workspace;
  overview: G.ClaudeOverview;
  member: G.MemberClaude;
  id: string;
  onClose: () => void;
}) {
  const wide = useMedia("(min-width: 48rem)");
  const body = <DetailBody w={w} overview={overview} member={member} />;
  if (!wide)
    return (
      <Sheet
        title={`${member.user.name}'s Claude connection`}
        description={factsText(member, w.name)}
        onClose={onClose}
      >
        {body}
      </Sheet>
    );
  return (
    <div id={id} className="animate-rise pb-1 pl-[42px]">
      {body}
    </div>
  );
}

function DetailBody({
  w,
  overview,
  member,
}: {
  w: G.Workspace;
  overview: G.ClaudeOverview;
  member: G.MemberClaude;
}) {
  const id = useId();
  const sentences = detail(member, w.name, overview.limit_pauses);
  const name = member.user.name;
  const act = needsAction(member);
  return (
    <div className="flex flex-col gap-3 text-[13px] leading-relaxed text-ink-2">
      <p id={id}>{sentences.join(" ")}</p>
      {act &&
        (overview.admin_tokens ? (
          <div className="flex flex-col gap-2">
            <p>Set them a new one with an admin token, from a token claude setup-token printed:</p>
            <CodeBox value={adminTokenCommand(member.email, w.name)} />
            <p className="text-[12.5px] text-ink-3">
              Or {name} connects one of their own in Settings → Claude
              {member.state === "none" ? "." : ": fctl has told them too."}
            </p>
          </div>
        ) : (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <p>
              {member.state === "none"
                ? `${name} connects one in Settings → Claude.`
                : `${name} connects a new token in Settings → Claude: fctl has told them too.`}
            </p>
            <CopyButton
              text={settingsLink(window.location.origin)}
              label="Copy the link to their Settings → Claude"
              className="border border-line px-2.5"
            />
          </div>
        ))}
      {frequent(member) && (
        <p className="text-[12.5px] text-ink-3">
          Seats are changed in Claude's organization settings:{" "}
          <a href={SEATS_HELP} target="_blank" rel="noreferrer" className="underline underline-offset-2">
            how
          </a>
          .
        </p>
      )}
    </div>
  );
}

/** What a narrowed list says in its place when nobody is under the pill. */
export function NobodyHere({ narrow, w, onAll }: { narrow: Narrow; w: G.Workspace; onAll: () => void }) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line py-4 text-[13.5px] text-ink-3">
      {nobody(narrow, w.name)}
      <Button size="sm" variant="ghost" onClick={onAll}>
        Show everyone
      </Button>
    </li>
  );
}
