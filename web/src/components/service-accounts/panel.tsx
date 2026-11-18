// A workspace's Service accounts tab: programs of the workspace's own (a portal where people start
// agents, a chat bot, a script) that start sessions for its people who allow them. Its admins see
// what one does before there are any, make them (the secret shows once, with how a program uses it),
// and open each one's page; everyone sees them with their own answer, which they change here.

import { useMutation, useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  CircleCheck,
  Eye,
  KeyRound,
  Plus,
  ShieldCheck,
  UserRound,
  UserRoundCog,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { Avatar, Banner, Button, CodeBox, Field, Meta, Skeleton, Textarea } from "@/components/ui";
import { ROW_ABOVE, ROW_COVER } from "@/components/ui/list-row";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShortWords } from "@/lib/format";
import { keys } from "@/lib/queries";
import { accountState, answerWords, tallyWords } from "@/lib/service-accounts";
import { ServiceAccountPage } from "./page";
import { AccountGlyph, AnswerControl, refreshServiceAccounts, StatePill } from "./parts";
import { UsageHow } from "./usage";

export function ServiceAccountsPanel({
  workspace: w,
  open,
  onOpen,
}: {
  workspace: G.Workspace;
  /** The service account whose page shows, from the address. */
  open?: string;
  onOpen: (id?: string) => void;
}) {
  const admin = w.role === "owner" || w.role === "admin";
  const list = useQuery({
    queryKey: keys.serviceAccounts(w.id),
    queryFn: () => api.serviceAccounts(w.id),
    // Its people answer on their own, and its program uses it: how each stands shows within the minute.
    refetchInterval: 60_000,
  });
  const [making, setMaking] = useState(false);
  const [made, setMade] = useState<G.CreatedServiceAccount>();

  if (open && admin) return <ServiceAccountPage id={open} workspace={w} onBack={() => onOpen(undefined)} />;
  if (list.isError) return <Banner tone="error">{list.error.message}</Banner>;
  if (!list.data)
    return (
      <div className="space-y-3">
        <Skeleton className="h-14" />
        <Skeleton className="h-14" />
      </div>
    );
  const { service_accounts: accounts, consent, limit } = list.data;
  if (!admin) return <Members workspace={w} accounts={accounts} consent={consent} />;
  const full = accounts.length >= limit;
  const form = (
    <MakeForm
      workspace={w}
      consent={consent}
      onDone={(created) => {
        setMaking(false);
        if (created) setMade(created);
      }}
    />
  );
  if (accounts.length === 0 && !making && !made)
    return <Intro workspace={w} consent={consent} onMake={() => setMaking(true)} />;
  return (
    <div className="flex flex-col gap-6">
      {made && (
        <Made
          made={made}
          workspace={w}
          onOpen={() => {
            setMade(undefined);
            onOpen(made.service_account.id);
          }}
          onDone={() => setMade(undefined)}
        />
      )}
      {making ? (
        form
      ) : (
        <Header
          accounts={accounts}
          limit={limit}
          full={full}
          onMake={() => {
            setMade(undefined);
            setMaking(true);
          }}
        />
      )}
      {accounts.length > 0 && (
        <ul className="border-t border-line">
          {accounts.map((a) => (
            <Row key={a.id} account={a} onOpen={() => onOpen(a.id)} />
          ))}
        </ul>
      )}
      {!consent && <UnaskedNote workspace={w.name} />}
    </div>
  );
}

/** Where the instance doesn't ask people, said once under the list. */
function UnaskedNote({ workspace }: { workspace: string }) {
  return (
    <p className="flex items-start gap-2 text-[12.5px] leading-relaxed text-ink-3">
      <ShieldCheck className="mt-0.5 size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
      <span>
        This instance's admins chose that people don't choose: {workspace}'s service accounts start sessions
        for anyone here, on their own Claude subscription, without asking them.
      </span>
    </p>
  );
}

/** How many there are, which stopped, and the button that makes another. */
function Header({
  accounts,
  limit,
  full,
  onMake,
}: {
  accounts: G.ServiceAccount[];
  limit: number;
  full: boolean;
  onMake: () => void;
}) {
  const stopped = accounts.filter((a) => a.stopped).length;
  const facts = [
    `${accounts.length} ${accounts.length === 1 ? "service account" : "service accounts"}`,
    stopped > 0 ? `${stopped} stopped` : undefined,
  ].filter(Boolean);
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-[13px] text-ink-3">{facts.join(" · ")}</p>
      <div className="flex items-center gap-3">
        {full && (
          <span className="text-[12.5px] text-ink-4">The workspace has {limit}, the most it may have</span>
        )}
        <Button size="sm" icon={<Plus className="size-3.5" />} onClick={onMake} disabled={full}>
          Make a service account
        </Button>
      </div>
    </div>
  );
}

/**
 * A service account in a line, for its admins: how it stands, who allowed it, what it did lately, and
 * their own answer, which they give or change here as members do: the badge, Needs you and Settings
 * lead them to this list. Its name opens its page from anywhere on the row (`ROW_COVER`); the answer
 * lies over it (`ROW_ABOVE`).
 */
function Row({ account: a, onOpen }: { account: G.ServiceAccount; onOpen: () => void }) {
  const state = accountState(a);
  const asks = a.consent && !a.answer;
  const facts = [
    a.people && tallyWords(a.people, a.consent),
    a.sessions ? `${a.sessions} ${a.sessions === 1 ? "session" : "sessions"} started` : undefined,
    a.last_used_at ? `used ${agoShortWords(a.last_used_at)}` : "not used yet",
  ].filter(Boolean);
  return (
    <li className="group relative isolate border-b border-line transition-colors hover:bg-hover/30 has-[[data-cover]:focus-visible]:bg-hover/30">
      <div className="flex flex-wrap items-start gap-x-3.5 gap-y-3 px-3 py-3.5">
        <AccountGlyph state={state} />
        <div className="min-w-0 flex-1 max-sm:basis-[calc(100%-3rem)]">
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-cover
              onClick={onOpen}
              className={cn(ROW_COVER, "min-w-0 truncate text-left text-[14.5px] text-ink")}
            >
              {a.name}
            </button>
            <StatePill state={state} />
          </div>
          {a.description && (
            <p className="mt-0.5 truncate text-[12.5px] leading-snug text-ink-2">{a.description}</p>
          )}
          <p className="mt-1 text-[12px] text-ink-4">
            {asks && <span className="text-blue">Asks to start agents for you · </span>}
            {facts.join(" · ")}
          </p>
          {a.stopped && (
            <p className="mt-1 text-[12.5px] leading-relaxed text-amber [overflow-wrap:anywhere]">
              Stopped: {a.stopped}
            </p>
          )}
        </div>
        {a.consent && (
          <div className={cn(ROW_ABOVE, "flex shrink-0 items-center max-sm:w-full max-sm:justify-end")}>
            <AnswerControl account={a} />
          </div>
        )}
        <ArrowRight
          aria-hidden="true"
          className="mt-2 size-3.5 shrink-0 text-ink-4 transition-transform duration-200 group-hover:translate-x-0.5 max-sm:hidden"
        />
      </div>
    </li>
  );
}

const POINTS: { key: string; icon: ReactNode; text: (workspace: string, consent: boolean) => ReactNode }[] = [
  {
    key: "person",
    icon: <UserRound />,
    text: () =>
      "It acts for one person at a time, named in each request: the session it starts is theirs, on their Claude subscription, within what they may do in the project.",
  },
  {
    key: "consent",
    icon: <ShieldCheck />,
    text: (workspace, consent) =>
      consent
        ? `People choose: it starts sessions only for those who allowed it. fctl asks each of ${workspace}'s people, and each one who joins later, and they change their answer whenever they like.`
        : `This instance doesn't ask people: it starts sessions for anyone in ${workspace}, on their own Claude subscription.`,
  },
  {
    key: "reads",
    icon: <Eye />,
    text: () =>
      "For nobody in particular, it reads the workspace's projects, sessions and schedules, and runs schedules for the people they run for. It changes nothing else: no settings, people, secrets or tokens.",
  },
  {
    key: "keeper",
    icon: <KeyRound />,
    text: () =>
      "Its secret works while the admin who made it is an admin here. If they leave, it stops until another admin makes it a new one.",
  },
];

/** What service accounts do, before there are any, with a program's request drawn beside it. */
function Intro({
  workspace: w,
  consent,
  onMake,
}: {
  workspace: G.Workspace;
  consent: boolean;
  onMake: () => void;
}) {
  return (
    <div className="flex animate-rise flex-col gap-10 py-2">
      <div className="grid items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_320px]">
        <div>
          <h2 className="font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
            Agents started from your own tools
          </h2>
          <p className="mt-3 mb-6 max-w-xl text-[15px] leading-relaxed text-ink-3">
            A service account lets a program of {w.name}'s, such as a portal where your people start agents, a
            chat bot or a script, start sessions for them through fctl's API and CLI, and follow what their
            agents do.
          </p>
          <ul className="flex max-w-xl flex-col gap-3">
            {POINTS.map((p) => (
              <li key={p.key} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                  {p.icon}
                </span>
                <span className="min-w-0">{p.text(w.name, consent)}</span>
              </li>
            ))}
          </ul>
          <div className="mt-8 flex flex-wrap items-center gap-x-4 gap-y-3">
            <Button variant="primary" size="lg" icon={<Plus className="size-4" />} onClick={onMake}>
              Make a service account
            </Button>
            <p className="max-w-sm text-[12.5px] leading-snug text-ink-3">
              {w.name}'s admins make them. Every session one starts says so, in the app, the audit log and
              webhooks.
            </p>
          </div>
        </div>
        <Preview workspace={w.slug} name={w.name} consent={consent} />
      </div>
    </div>
  );
}

/**
 * What a service account does, drawn: a portal's request for Ada as it goes out, the session it
 * started, hers, and under it whom it acts for: those who allowed it. Made up, in the world the
 * app's previews share (the login redirect of web#12), with this workspace's slug and name.
 */
function Preview({ workspace, name, consent }: { workspace: string; name: string; consent: boolean }) {
  const card = "overflow-hidden rounded-xl border border-line bg-surface";
  const people: [string, string, "allowed" | "asked" | "declined"][] = [
    ["Ada Lovelace", "Allowed it", "allowed"],
    ["Grace Hopper", "Asked", "asked"],
    ["Linus Torvalds", "Didn't allow it", "declined"],
  ];
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className={cn(card, "shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]")}>
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5">
          <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-[10.5px] font-semibold text-ink-2">
            POST
          </span>
          <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-2">
            /projects/web/sessions
          </span>
          <span className="inline-flex items-center gap-1 font-mono text-[11px] text-checks-passed">
            <CircleCheck className="size-3" />
            201
          </span>
        </div>
        <div className="border-b border-line px-3.5 py-2 font-mono text-[10.5px] leading-[1.7] text-ink-4">
          <div>
            authorization: <span className="text-ink-2">Bearer fctl_svc_••••</span>
          </div>
          <div>
            fctl-on-behalf-of: <span className="text-blue">ada@{workspace}.dev</span>
          </div>
          <div className="truncate">
            {"{ "}
            <span className="text-blue">"prompt"</span>
            {": "}
            <span className="text-ink-2">"Fix the login redirect"</span>
            {" }"}
          </div>
        </div>
        <div className="flex items-start gap-2.5 px-3.5 py-2.5">
          <span className="mt-1.5 size-2 shrink-0 rounded-full bg-accent shadow-[0_0_6px_var(--c-accent)]" />
          <div className="min-w-0">
            <div className="truncate text-[12.5px] text-ink">
              <span className="text-ink-3">web#12</span> Fix the login redirect
            </div>
            <div className="mt-0.5 flex items-center gap-1 text-[11px] text-ink-4">
              <UserRoundCog className="size-3" />
              Ada Lovelace, through {name} portal
            </div>
          </div>
        </div>
      </div>
      <div className={cn(card, "mt-3 shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]")}>
        <div className="flex flex-col gap-1.5 px-3.5 py-2.5 text-[11.5px]">
          {people.map(([name, says, kind]) => (
            <div key={name} className="flex items-center gap-2">
              <span
                className={cn(
                  "size-1.5 shrink-0 rounded-full",
                  kind === "allowed" || !consent
                    ? "bg-checks-passed"
                    : kind === "asked"
                      ? "bg-blue"
                      : "bg-ink-4",
                )}
              />
              <span className="min-w-0 flex-1 truncate text-ink-2">{name}</span>
              <span
                className={cn(
                  "shrink-0",
                  kind === "asked" && consent
                    ? "text-blue"
                    : kind === "declined" && consent
                      ? "text-ink-4"
                      : "text-ink-3",
                )}
              >
                {consent ? says : "It acts for them"}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Making a service account: its name, and what it is, for the people it asks. */
function MakeForm({
  workspace: w,
  consent,
  onDone,
}: {
  workspace: G.Workspace;
  consent: boolean;
  onDone: (made?: G.CreatedServiceAccount) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [touched, setTouched] = useState(false);
  const make = useMutation({
    mutationFn: () =>
      api.createServiceAccount(w.id, {
        name: name.trim(),
        description: description.trim() || undefined,
      }),
    onSuccess: (made) => {
      refreshServiceAccounts(made.service_account.id);
      onDone(made);
    },
  });
  const failure = make.error instanceof ApiFailure ? make.error : undefined;
  const fieldError = (f: string) => (failure?.field === f ? failure.message : undefined);
  const missing = !name.trim() ? "Name it for what uses it." : undefined;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (!missing && !make.isPending) make.mutate();
      }}
      aria-label="New service account"
      className="flex animate-rise scroll-mt-24 flex-col gap-6 rounded-2xl border border-line bg-surface/70 p-4 shadow-[0_24px_60px_-40px_rgb(0_0_0/0.5)] sm:p-5"
    >
      <div className="flex flex-col gap-4">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Name it: what uses it"
          aria-label="Name"
          autoFocus
          maxLength={60}
          className="w-full border-b border-transparent bg-transparent pb-1 font-display text-[20px] leading-snug font-medium tracking-[-0.01em] text-ink outline-none transition-colors placeholder:text-ink-4 hover:border-line focus:border-line-strong"
        />
        {(fieldError("name") || (touched && missing)) && (
          <p className="-mt-3 text-[12.5px] text-danger">{fieldError("name") ?? missing}</p>
        )}
        <Field
          label="What it is"
          error={fieldError("description")}
          hint={
            consent
              ? `${w.name}'s people read it as fctl asks them to let it start sessions for them: say what it is and where they use it.`
              : `${w.name}'s people read it where fctl shows it: say what it is and where they use it.`
          }
        >
          {(id) => (
            <Textarea
              id={id}
              rows={2}
              maxLength={280}
              placeholder={`Our agent portal at portal.${w.slug}.dev: start agents there, and follow them in fctl.`}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          )}
        </Field>
      </div>
      {make.error && !failure?.field && <Banner tone="error">{make.error.message}</Banner>}
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3 border-t border-line pt-4">
        <p className="min-w-0 text-[12.5px] leading-snug text-ink-3">
          {consent
            ? `Its secret shows once, as it's made. Then fctl asks ${w.name}'s people to let it start sessions for them.`
            : "Its secret shows once, as it's made. It starts sessions for anyone here from then on."}
        </p>
        <div className="flex gap-2 max-sm:ml-auto">
          <Button variant="ghost" onClick={() => onDone()}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={make.isPending} disabled={touched && !!missing}>
            Make service account
          </Button>
        </div>
      </div>
    </form>
  );
}

/** A service account just made: its secret, shown once here, and how a program uses it. */
export function Made({
  made,
  workspace: w,
  again,
  onOpen,
  onDone,
}: {
  made: G.CreatedServiceAccount;
  workspace: G.Workspace;
  /** A new secret for one that was there: it says so, and has no page to open. */
  again?: boolean;
  onOpen?: () => void;
  onDone: () => void;
}) {
  const a = made.service_account;
  const asked = a.people ? a.people.unanswered + a.people.declined + a.people.allowed : w.members;
  return (
    <section
      aria-labelledby="made-heading"
      className="flex animate-rise flex-col gap-5 rounded-2xl border border-line bg-surface/70 p-4 shadow-[0_24px_60px_-40px_rgb(0_0_0/0.5)] sm:p-5"
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full bg-accent-soft text-accent">
          <KeyRound className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <h3 id="made-heading" className="text-[15px] font-semibold text-ink [overflow-wrap:anywhere]">
            {again ? `${a.name}'s new secret` : `Made ${a.name}`}
          </h3>
          <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
            {again
              ? "Its old secret stopped working: give your program this one. You keep it from now on."
              : a.consent
                ? `fctl asks ${w.name}'s ${asked} ${asked === 1 ? "person" : "people"} to let it start sessions for them. It starts none for anyone until they do.`
                : `It starts sessions for anyone in ${w.name} from now on.`}
          </p>
        </div>
      </div>
      <Banner tone="warn">
        Copy its secret now: it shows once, here. Whoever has it starts sessions for the people who allowed
        it, so keep it where your program reads it, and nowhere else.
      </Banner>
      <CodeBox value={made.secret} />
      <UsageHow workspace={w} secret={made.secret} />
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line pt-4">
        <Button variant={onOpen ? "ghost" : "primary"} onClick={onDone}>
          Done
        </Button>
        {onOpen && (
          <Button variant="primary" icon={<ArrowRight className="size-3.5" />} onClick={onOpen}>
            See who it acts for
          </Button>
        )}
      </div>
    </section>
  );
}

/** The tab for the workspace's members: its service accounts, each with your answer to change. */
function Members({
  workspace: w,
  accounts,
  consent,
}: {
  workspace: G.Workspace;
  accounts: G.ServiceAccount[];
  consent: boolean;
}) {
  if (accounts.length === 0)
    return (
      <div className="flex animate-rise flex-col items-start gap-2 py-2">
        <h2 className="text-[15px] font-semibold text-ink">No service accounts</h2>
        <p className="max-w-xl text-[13.5px] leading-relaxed text-ink-3">
          {w.name}'s admins make them for programs of the workspace's own, such as a portal where people start
          agents. {consent ? "Each one asks you before it starts sessions for you." : ""}
        </p>
      </div>
    );
  return (
    <div className="flex animate-rise flex-col gap-6">
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Programs of {w.name}'s own that start sessions for its people, through fctl's API.{" "}
        {consent
          ? "Each starts sessions for you only if you allow it: they're yours, on your Claude subscription, and you follow them as you follow your own. It messages, interrupts and stops their agents for you, and nothing else. Change your answer whenever you like."
          : "This instance's admins chose that people don't choose: they start sessions for anyone here, on their own Claude subscription."}
      </p>
      <ul className="border-t border-line">
        {accounts.map((a) => (
          <li
            key={a.id}
            className="flex flex-wrap items-start gap-x-3.5 gap-y-3 border-b border-line px-3 py-3.5"
          >
            <AccountGlyph />
            <div className="min-w-0 flex-1 max-sm:basis-[calc(100%-3rem)]">
              <div className="text-[14.5px] text-ink [overflow-wrap:anywhere]">{a.name}</div>
              {a.description && (
                <p className="mt-0.5 text-[13px] leading-relaxed text-ink-2 [overflow-wrap:anywhere]">
                  {a.description}
                </p>
              )}
              {/* Where the facts wrap, the next line starts with one, not with a dot. */}
              <Meta
                parts={[
                  {
                    key: "answer",
                    node: answerWords(a),
                    className: cn("text-[12px]", a.consent && !a.answer && "text-blue"),
                  },
                  a.created_by && {
                    key: "maker",
                    node: (
                      <span className="inline-flex items-center gap-1">
                        made by <Avatar user={a.created_by} size={14} /> {a.created_by.name}
                      </span>
                    ),
                    className: "text-[12px]",
                  },
                  !!a.answered_at && {
                    key: "answered",
                    node: `answered ${agoShortWords(a.answered_at)}`,
                    className: "text-[12px]",
                  },
                ]}
              />
            </div>
            <div className="flex shrink-0 items-center max-sm:w-full max-sm:justify-end">
              <AnswerControl account={a} />
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
