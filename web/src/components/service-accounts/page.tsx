// A service account's page, for its workspace's admins: what it is and how it stands, what's wrong
// when it stopped (its keeper left) with the way out beside it, whom it acts for (each person's
// answer, yours to change), the sessions it started, its secret and how a program uses it, and
// deleting it.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ChevronLeft, Ellipsis, KeyRound, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { SessionRow } from "@/components/session/row";
import {
  Avatar,
  Banner,
  Button,
  DangerZone,
  Dialog,
  Field,
  IconButton,
  Menu,
  MenuItem,
  MenuSeparator,
  Section,
  SectionLayout,
  Skeleton,
  Textarea,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShortWords, stamp } from "@/lib/format";
import { keys, useMe } from "@/lib/queries";
import { accountState, tallyWords } from "@/lib/service-accounts";
import { Made } from "./panel";
import { AccountGlyph, AnswerControl, refreshServiceAccounts, StatePill } from "./parts";
import { UsageHow } from "./usage";

export function ServiceAccountPage({
  id,
  workspace: w,
  onBack,
}: {
  id: string;
  workspace: G.Workspace;
  onBack: () => void;
}) {
  const account = useQuery({
    queryKey: keys.serviceAccount(id),
    queryFn: () => api.serviceAccount(id),
    refetchInterval: 30_000,
  });
  const [editing, setEditing] = useState(false);
  const [renewing, setRenewing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [renewed, setRenewed] = useState<G.CreatedServiceAccount>();
  const back = (
    <button
      type="button"
      onClick={onBack}
      className="-ml-1.5 inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[13px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
    >
      <ChevronLeft className="size-4" aria-hidden="true" />
      Service accounts
    </button>
  );
  if (account.isError)
    return (
      <div className="flex flex-col items-start gap-4">
        {back}
        <Banner tone="error" className="self-stretch">
          {account.error.message}
        </Banner>
      </div>
    );
  if (!account.data)
    return (
      <div className="flex flex-col gap-4">
        {back}
        <Skeleton className="h-16" />
        <Skeleton className="h-40" />
      </div>
    );
  const a = account.data;
  const state = accountState(a);
  return (
    <div className="flex animate-rise flex-col gap-8">
      <div className="flex flex-col gap-4">
        {back}
        {editing ? (
          <EditForm account={a} onDone={() => setEditing(false)} />
        ) : (
          // On a phone its buttons go under it, rather than squeeze its name into a column.
          <header className="flex flex-wrap items-start gap-x-4 gap-y-3">
            <AccountGlyph state={state} />
            <div className="min-w-0 flex-1 max-sm:basis-[calc(100%-3rem)]">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="min-w-0 font-display text-[22px] leading-tight font-medium tracking-[-0.015em] text-ink [overflow-wrap:anywhere]">
                  {a.name}
                </h2>
                <StatePill state={state} />
              </div>
              {a.description ? (
                <p className="mt-2 max-w-2xl text-[13.5px] leading-relaxed text-ink-2 [overflow-wrap:anywhere]">
                  {a.description}
                </p>
              ) : (
                <p className="mt-2 max-w-2xl text-[13px] leading-relaxed text-ink-4">
                  It says nothing of what it is.{" "}
                  <button
                    type="button"
                    onClick={() => setEditing(true)}
                    className="text-ink-3 underline underline-offset-2 hover:text-ink"
                  >
                    Say what it is
                  </button>
                  , for the people it asks.
                </p>
              )}
              <p className="mt-1.5 flex flex-wrap items-center gap-x-1.5 text-[12px] text-ink-4">
                <span>{w.name}'s</span>
                {a.created_by && (
                  <>
                    <span aria-hidden="true">·</span>
                    <span className="inline-flex items-center gap-1">
                      made by <Avatar user={a.created_by} size={14} /> {a.created_by.name}
                    </span>
                  </>
                )}
                <span aria-hidden="true">·</span>
                <span title={stamp(a.created_at)}>{agoShortWords(a.created_at)}</span>
                <span aria-hidden="true">·</span>
                <span title={a.last_used_at ? stamp(a.last_used_at) : undefined}>
                  {a.last_used_at ? `used ${agoShortWords(a.last_used_at)}` : "not used yet"}
                </span>
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5 max-sm:w-full max-sm:pl-12">
              <Button size="sm" icon={<Pencil className="size-3.5" />} onClick={() => setEditing(true)}>
                Edit
              </Button>
              <Menu
                trigger={(open, menuOpen) => (
                  <IconButton
                    label="Service account actions"
                    aria-haspopup="menu"
                    aria-expanded={menuOpen}
                    onClick={open}
                  >
                    <Ellipsis className="size-4" />
                  </IconButton>
                )}
              >
                {(close) => (
                  <>
                    <MenuItem
                      icon={<RefreshCw />}
                      onClick={() => {
                        close();
                        setRenewing(true);
                      }}
                    >
                      Make a new secret…
                    </MenuItem>
                    <MenuSeparator />
                    <MenuItem
                      danger
                      icon={<Trash2 />}
                      onClick={() => {
                        close();
                        setDeleting(true);
                      }}
                    >
                      Delete…
                    </MenuItem>
                  </>
                )}
              </Menu>
            </div>
          </header>
        )}
        {a.stopped && !renewed && (
          <Banner tone="warn">
            <span className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
              <span className="min-w-0">
                It stopped: {a.stopped} Until then, your program's requests are refused, and it starts no
                sessions.
              </span>
              <Button size="sm" icon={<RefreshCw className="size-3.5" />} onClick={() => setRenewing(true)}>
                Make a new secret
              </Button>
            </span>
          </Banner>
        )}
      </div>

      {renewed && <Made made={renewed} workspace={w} again onDone={() => setRenewed(undefined)} />}

      <SectionLayout.Provider value="part">
        <People account={a} />
        <Sessions account={a} workspace={w} />
        <Section
          title="Secret"
          description="Your program sends it as its bearer token. fctl keeps only a hash of it, and its end, to tell secrets apart."
        >
          <div className="flex flex-col gap-6">
            <SecretLine account={a} onRenew={() => setRenewing(true)} />
            <UsageHow workspace={w} />
          </div>
        </Section>
      </SectionLayout.Provider>
      <DangerZone
        title="Delete this service account"
        description="Its secret works no more, and people's answers to it go. The sessions it started stay their people's."
        action="Delete…"
        onAction={() => setDeleting(true)}
      />

      <RenewDialog
        account={a}
        open={renewing}
        onClose={() => setRenewing(false)}
        onRenewed={(made) => {
          setRenewing(false);
          setRenewed(made);
        }}
      />
      <DeleteDialog account={a} open={deleting} onClose={() => setDeleting(false)} onDeleted={onBack} />
    </div>
  );
}

/** Renaming it, and saying what it is. */
function EditForm({ account: a, onDone }: { account: G.ServiceAccount; onDone: () => void }) {
  const [name, setName] = useState(a.name);
  const [description, setDescription] = useState(a.description ?? "");
  const save = useMutation({
    mutationFn: () => api.updateServiceAccount(a.id, { name: name.trim(), description: description.trim() }),
    onSuccess: (s) => {
      refreshServiceAccounts(s.id);
      toast("Saved: its people see it so from now on");
      onDone();
    },
  });
  const failure = save.error instanceof ApiFailure ? save.error : undefined;
  const fieldError = (f: string) => (failure?.field === f ? failure.message : undefined);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (name.trim()) save.mutate();
      }}
      aria-label={`Change ${a.name}`}
      className="flex animate-rise flex-col gap-5 rounded-2xl border border-line bg-surface/70 p-4 sm:p-5"
    >
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        aria-label="Name"
        maxLength={60}
        autoFocus
        className="w-full border-b border-transparent bg-transparent pb-1 font-display text-[20px] leading-snug font-medium tracking-[-0.01em] text-ink outline-none transition-colors placeholder:text-ink-4 hover:border-line focus:border-line-strong"
      />
      {fieldError("name") && <p className="-mt-4 text-[12.5px] text-danger">{fieldError("name")}</p>}
      <Field
        label="What it is"
        error={fieldError("description")}
        hint="The people it asks read it: say what it is and where they use it."
      >
        {(id) => (
          <Textarea
            id={id}
            rows={2}
            maxLength={280}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        )}
      </Field>
      {save.error && !failure?.field && <Banner tone="error">{save.error.message}</Banner>}
      <div className="flex justify-end gap-2 border-t border-line pt-4">
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" loading={save.isPending} disabled={!name.trim()}>
          Save
        </Button>
      </div>
    </form>
  );
}

type Filter = "all" | "acts" | "asked" | "declined" | "cannot";

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "Everyone" },
  { value: "acts", label: "It acts for" },
  { value: "asked", label: "Asked" },
  { value: "declined", label: "Didn't allow it" },
  { value: "cannot", label: "Can't now" },
];

/** How a person stands for a service account: it acts for them, waits for their answer, or can't. */
function standing(p: G.ServiceAccountPerson): Exclude<Filter, "all"> {
  if (p.acts_for) return "acts";
  if (p.blocked) return "cannot";
  return p.answer === "declined" ? "declined" : "asked";
}

/** Whom it acts for: each of the workspace's people, with their answer, and yours to change. */
function People({ account: a }: { account: G.ServiceAccount }) {
  const { data: me } = useMe();
  const people = useQuery({
    queryKey: keys.serviceAccountPeople(a.id),
    queryFn: () => api.serviceAccountPeople(a.id),
    refetchInterval: 60_000,
  });
  const [filter, setFilter] = useState<Filter>("all");
  const list = people.data?.people ?? [];
  const counts = new Map<Filter, number>();
  for (const p of list) counts.set(standing(p), (counts.get(standing(p)) ?? 0) + 1);
  const shown = filter === "all" ? list : list.filter((p) => standing(p) === filter);
  return (
    <Section
      title="Who it acts for"
      description={
        a.consent
          ? "Each person answers for themselves: fctl asks them on their Agent Floor, and in this workspace's Service accounts. Nobody answers for anyone else."
          : "This instance doesn't ask people: it acts for every member here whose account it can act for."
      }
    >
      {people.isError ? (
        <Banner tone="error">{people.error.message}</Banner>
      ) : !people.data ? (
        <div className="space-y-2">
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-[13px] text-ink-2">{a.people && tallyWords(a.people, a.consent)}</p>
            {list.length > 6 && (
              <fieldset
                aria-label="Show"
                className="inline-flex flex-wrap rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
              >
                {FILTERS.filter((f) => f.value === "all" || counts.get(f.value)).map((f) => (
                  <button
                    key={f.value}
                    type="button"
                    aria-pressed={filter === f.value}
                    onClick={() => setFilter(f.value)}
                    className={cn(
                      "h-7 rounded-full px-3 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                      filter === f.value
                        ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                        : "text-ink-3 hover:text-ink",
                    )}
                  >
                    {f.label}
                    {f.value !== "all" && (
                      <span className="ml-1.5 text-ink-4 tabular-nums">{counts.get(f.value)}</span>
                    )}
                  </button>
                ))}
              </fieldset>
            )}
          </div>
          <ul className="border-t border-line">
            {shown.map((p) => (
              <PersonRow key={p.user.id} person={p} account={a} you={p.user.id === me?.user.id} />
            ))}
          </ul>
        </div>
      )}
    </Section>
  );
}

const DOTS: Record<Exclude<Filter, "all">, string> = {
  acts: "bg-checks-passed",
  asked: "bg-blue",
  declined: "bg-ink-4",
  cannot: "bg-amber",
};

function PersonRow({
  person: p,
  account: a,
  you,
}: {
  person: G.ServiceAccountPerson;
  account: G.ServiceAccount;
  you: boolean;
}) {
  const s = standing(p);
  const says =
    s === "acts"
      ? a.consent
        ? `Allowed it${p.answered_at ? ` ${agoShortWords(p.answered_at)}` : ""}`
        : "It acts for them"
      : s === "asked"
        ? "Asked: hasn't answered yet"
        : s === "declined"
          ? "Didn't allow it"
          : (p.why_not ?? "It can't act for them now");
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-1 py-2.5">
      <Avatar user={p.user} size={28} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13.5px] text-ink">
          {p.user.name}
          {you && <span className="text-ink-4"> (you)</span>}
        </div>
        <div className="truncate text-[12px] text-ink-4">
          {p.email}
          {p.role !== "member" && ` · ${p.role === "owner" ? "primary admin" : "admin"}`}
        </div>
      </div>
      {you && a.consent ? (
        <AnswerControl account={{ ...a, answer: p.answer, answered_at: p.answered_at }} />
      ) : (
        <span
          className={cn(
            "flex max-w-[22rem] items-center gap-1.5 text-[12.5px]",
            s === "asked" ? "text-blue" : s === "cannot" ? "text-amber" : "text-ink-3",
          )}
        >
          <span className={cn("size-1.5 shrink-0 rounded-full", DOTS[s])} aria-hidden="true" />
          <span className="min-w-0 [overflow-wrap:anywhere]">{says}</span>
        </span>
      )}
    </li>
  );
}

/** The sessions it started lately, each its person's. */
function Sessions({ account: a, workspace: w }: { account: G.ServiceAccount; workspace: G.Workspace }) {
  const sessions = useQuery({
    queryKey: keys.serviceAccountSessions(a.id),
    queryFn: () => api.serviceAccountSessions(a.id),
    refetchInterval: 30_000,
  });
  const list = sessions.data?.sessions ?? [];
  return (
    <Section
      title="Sessions it started"
      description={
        a.sessions
          ? `${a.sessions} in all, the newest first. Each is its person's, who follows it.`
          : "Each one it starts is its person's, who follows it as they follow their own."
      }
    >
      {sessions.isError ? (
        <Banner tone="error">{sessions.error.message}</Banner>
      ) : !sessions.data ? (
        <Skeleton className="h-12" />
      ) : list.length === 0 ? (
        <p className="border-y border-line py-5 text-[13.5px] text-ink-4">
          None yet. The sessions it starts show here as it starts them.
        </p>
      ) : (
        <div className="border-t border-line">
          {list.map((s) => (
            <SessionRow key={s.id} s={s} workspace={w} />
          ))}
        </div>
      )}
    </Section>
  );
}

/** Its secret, by its end, who keeps it, and making it a new one. */
function SecretLine({ account: a, onRenew }: { account: G.ServiceAccount; onRenew: () => void }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 rounded-md border border-line-strong bg-sunken py-1.5 pr-1.5 pl-3">
        <KeyRound className="size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
        <code className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-ink-3">
          <span aria-hidden="true">fctl_svc_••••••••••••••••••••••••••••••••••••</span>
          <span className="text-ink-2">{a.secret_hint}</span>
          <span className="sr-only">Ends in {a.secret_hint}</span>
        </code>
        <Button size="sm" variant="ghost" icon={<RefreshCw className="size-3.5" />} onClick={onRenew}>
          New secret…
        </Button>
      </div>
      {a.keeper && (
        <p className="flex flex-wrap items-center gap-x-1.5 text-[12.5px] leading-relaxed text-ink-3">
          <span className="inline-flex items-center gap-1">
            Made by <Avatar user={a.keeper} size={14} /> {a.keeper.name}
          </span>
          {a.secret_made_at && <span title={stamp(a.secret_made_at)}>{agoShortWords(a.secret_made_at)}</span>}
          <span>
            · it works while {a.keeper.name.split(" ")[0]} is an admin of {a.workspace.name}.
          </span>
        </p>
      )}
    </div>
  );
}

/** A new secret: what happens to the old one, said before the click. */
function RenewDialog({
  account: a,
  open,
  onClose,
  onRenewed,
}: {
  account: G.ServiceAccount;
  open: boolean;
  onClose: () => void;
  onRenewed: (made: G.CreatedServiceAccount) => void;
}) {
  const renew = useMutation({
    mutationFn: () => api.newServiceAccountSecret(a.id),
    onSuccess: (made) => {
      refreshServiceAccounts(a.id);
      onRenewed(made);
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Make ${a.name} a new secret?`}
      description={
        a.stopped
          ? "It works again with the new one, which you keep from now on: it stops only if you're no longer an admin here. People's answers stay as they are."
          : "Its current secret stops working at once: your program's requests are refused until it has the new one. You keep the new one from now on. People's answers stay as they are."
      }
    >
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" loading={renew.isPending} onClick={() => renew.mutate()}>
          Make a new secret
        </Button>
      </div>
    </Dialog>
  );
}

function DeleteDialog({
  account: a,
  open,
  onClose,
  onDeleted,
}: {
  account: G.ServiceAccount;
  open: boolean;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const remove = useMutation({
    mutationFn: () => api.deleteServiceAccount(a.id),
    onSuccess: () => {
      refreshServiceAccounts();
      toast(`Deleted ${a.name}: its secret works no more`);
      onDeleted();
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Delete ${a.name}?`}
      description={`Your program's requests are refused from now on, and people's answers to it go: a service account made again asks them again. The ${
        a.sessions === 1 ? "session" : "sessions"
      } it started stay their people's.`}
    >
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="danger" loading={remove.isPending} onClick={() => remove.mutate()}>
          Delete
        </Button>
      </div>
    </Dialog>
  );
}
