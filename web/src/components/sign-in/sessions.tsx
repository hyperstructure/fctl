// Admin → Sign-in's session policies: how long a browser and a CLI stay signed in, the longest an access token
// lasts, and whether members make them (G.SessionPolicy). A change is a draft until saved: what it ends at once
// shows first (POST /api/v1/admin/sign-in/sessions/preview), in the sentences `fctl admin sign-in policies` prints.

import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { Banner, Button, Section, Select, Skeleton, Switch, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient } from "@/lib/queries";
import {
  browserIdleChoices,
  browserMaxChoices,
  cliMaxChoices,
  consequences,
  describe,
  samePolicy,
  tokenMaxChoices,
} from "@/lib/session-policy";

/** A limit's row: what it is and what its value means, its control beside them. */
function Row({ label, hint, children }: { label: string; hint: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-2 text-[13.5px] text-ink-2 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
      <span>
        {label}
        <span className="block text-[12.5px] leading-relaxed text-ink-4">{hint}</span>
      </span>
      {children}
    </div>
  );
}

/** The section: the policy as saved, a draft of a change, and what saving it ends. */
export function SessionPolicies({ policy }: { policy?: G.SessionPolicy }) {
  const [draft, setDraft] = useState<G.SessionPolicy | undefined>(policy);
  // A policy saved here, or in another tab, replaces the draft.
  useEffect(() => setDraft(policy), [policy]);
  const changed = !!policy && !!draft && !samePolicy(policy, draft);
  const preview = useQuery({
    queryKey: [...keys.signIn, "sessions", draft],
    queryFn: () => api.sessionPolicyPreview(draft as G.SessionPolicy),
    enabled: changed,
    placeholderData: keepPreviousData,
    retry: false,
  });
  const save = useMutation({
    mutationFn: (p: G.SessionPolicy) => api.updateInstanceSettings({ ...p }),
    onSuccess: (s) => {
      queryClient.setQueryData(keys.instanceSettings, s);
      void queryClient.invalidateQueries({ queryKey: keys.signIn });
      void queryClient.invalidateQueries({ queryKey: keys.me });
      void queryClient.invalidateQueries({ queryKey: keys.tokens });
      toast("Saved: sign-ins last as it says now");
    },
  });
  const set = (p: Partial<G.SessionPolicy>) => setDraft((d) => (d ? { ...d, ...p } : d));
  return (
    <Section
      title="Staying signed in"
      description="How long a sign-in lasts before its person signs in again. Where a domain requires Google, that goes through Google, which checks they still may: once it closes someone's account, they're out of fctl when their sign-ins here end."
    >
      {!draft ? (
        <Skeleton className="h-48 w-full" />
      ) : (
        <form
          className="flex flex-col gap-6"
          onSubmit={(e) => {
            e.preventDefault();
            if (changed) save.mutate(draft);
          }}
        >
          <Row
            label="Browsers sign in again after"
            hint="However much they're used. Short, a closed account is out of fctl soon after."
          >
            <Select
              size="sm"
              aria-label="Browsers sign in again after"
              className="shrink-0 sm:w-48"
              value={String(draft.browser_max_hours)}
              options={browserMaxChoices(policy?.browser_max_hours)}
              onChange={(v) => set({ browser_max_hours: Number(v) })}
            />
          </Row>
          <Row
            label="Browsers sign out unused for"
            hint="A tab left open on a page that updates itself counts as use."
          >
            <Select
              size="sm"
              aria-label="Browsers sign out unused for"
              className="shrink-0 sm:w-48"
              value={String(draft.browser_idle_minutes)}
              options={browserIdleChoices(policy?.browser_idle_minutes)}
              onChange={(v) => set({ browser_idle_minutes: Number(v) })}
            />
          </Row>
          <Row
            label="CLIs sign in again after"
            hint="With fctl login, in a browser. A CLI unused for 180 days signs out whatever this says."
          >
            <Select
              size="sm"
              aria-label="CLIs sign in again after"
              className="shrink-0 sm:w-48"
              value={String(draft.cli_max_days)}
              options={cliMaxChoices(policy?.cli_max_days)}
              onChange={(v) => set({ cli_max_days: Number(v) })}
            />
          </Row>
          <Row
            label="Access tokens last at most"
            hint="New ones can't be made to last longer, and older ones end once they're that old."
          >
            <Select
              size="sm"
              aria-label="Access tokens last at most"
              className="shrink-0 sm:w-48"
              value={String(draft.access_token_max_days)}
              options={tokenMaxChoices(policy?.access_token_max_days)}
              onChange={(v) => set({ access_token_max_days: Number(v) })}
            />
          </Row>
          <Row
            label="Members make access tokens"
            hint={
              draft.member_access_tokens
                ? "Everyone makes tokens for scripts, CI and bots in Settings."
                : "Only admins make them, and members' stop working: members use the CLI, signed in as themselves."
            }
          >
            <Switch
              checked={draft.member_access_tokens}
              onChange={(v) => set({ member_access_tokens: v })}
              label="Members make access tokens"
            />
          </Row>
          {changed ? (
            <Change
              preview={preview.data}
              loading={preview.isFetching}
              error={preview.error?.message ?? save.error?.message}
              saving={save.isPending}
              onCancel={() => {
                setDraft(policy);
                save.reset();
              }}
            />
          ) : (
            <p className="text-[12.5px] leading-relaxed text-ink-4" aria-live="polite">
              {describe(draft).join(" ")}
            </p>
          )}
        </form>
      )}
    </Section>
  );
}

/** What saving the draft ends at once, and its buttons. */
function Change({
  preview,
  loading,
  error,
  saving,
  onCancel,
}: {
  preview?: G.SessionPolicyPreview;
  loading: boolean;
  error?: string;
  saving: boolean;
  onCancel: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      {preview ? (
        <ul
          className={cn(
            "flex flex-col gap-1.5 rounded-lg border border-line bg-sunken/40 px-4 py-3.5 text-[13px] leading-relaxed text-ink-2 transition-opacity",
            loading && "opacity-60",
          )}
          aria-live="polite"
        >
          {consequences(preview).map((l) => (
            <li key={l}>{l}</li>
          ))}
        </ul>
      ) : (
        loading && <Skeleton className="h-14 w-full" />
      )}
      {error && <Banner tone="error">{error}</Banner>}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" loading={saving} disabled={!preview}>
          Save
        </Button>
      </div>
    </div>
  );
}
