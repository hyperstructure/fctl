// Admin → Sign-in's networks (InstanceSettings.networks): the IP ranges people sign in and send requests from,
// an office's or a VPN's. The admin's own address shows beside them, and the control plane refuses to save ranges
// that would shut them out, unless they are the primary instance owner, who signs in from anywhere.

import { useMutation } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Banner, Button, Field, Section, Skeleton, Textarea, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";
import { domainsWords } from "@/lib/sign-in";

/** Ranges as typed: by lines, commas or spaces. */
export const parseNetworks = (text: string): string[] =>
  text
    .split(/[\s,]+/)
    .map((n) => n.trim())
    .filter(Boolean);

/** What the section says of the networks saved. */
export function networksLead(networks: readonly string[]): string {
  return networks.length === 0
    ? "People sign in and send requests from anywhere."
    : `People sign in and send requests only from ${domainsWords(networks)}. The primary instance owner signs in from anywhere, in a browser or the CLI.`;
}

export function Networks({ overview }: { overview?: G.SignInOverview }) {
  const saved = overview?.networks;
  const [text, setText] = useState("");
  // What's saved, here or in another tab, replaces what's typed.
  useEffect(() => setText((saved ?? []).join("\n")), [saved]);
  const typed = parseNetworks(text);
  const changed = !!saved && typed.join(",") !== saved.join(",");
  const save = useMutation({
    mutationFn: (networks: string[]) => api.updateInstanceSettings({ networks }),
    onSuccess: (s) => {
      queryClient.setQueryData(keys.instanceSettings, s);
      void queryClient.invalidateQueries({ queryKey: keys.signIn });
      void queryClient.invalidateQueries({ queryKey: keys.instance });
      toast(
        s.networks.length
          ? "Saved: sign-ins come only from these networks"
          : "Saved: people sign in from anywhere",
      );
    },
  });
  const address = overview?.address;
  const field = (save.error as { field?: string } | null)?.field;
  return (
    <Section
      title="Networks"
      description="Where people sign in and send requests from: your office's addresses, or your VPN's. Browsers, CLIs, access tokens, admin tokens and service accounts elsewhere are refused; runners, webhooks and published links connect from anywhere."
    >
      {!overview ? (
        <Skeleton className="h-28 w-full" />
      ) : (
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (changed) save.mutate(typed);
          }}
        >
          <p className="text-[13px] leading-relaxed text-ink-2" aria-live="polite">
            {networksLead(overview.networks ?? [])}
          </p>
          <Field
            label="Networks"
            hint="One range a line, as CIDR: 203.0.113.0/24, 2001:db8::/32, or an address alone. None: anywhere."
            error={field === "networks" ? save.error?.message : undefined}
          >
            {(id) => (
              <Textarea
                id={id}
                rows={3}
                value={text}
                placeholder={"203.0.113.0/24\n2001:db8::/32"}
                onChange={(e) => setText(e.target.value)}
                spellCheck={false}
                autoCapitalize="off"
                className="font-mono text-[12.5px]"
              />
            )}
          </Field>
          {address && (
            <p className="-mt-1 flex flex-wrap items-center gap-x-2 text-[12.5px] text-ink-3">
              <span>
                Your address: <code className="font-mono text-ink-2">{address}</code>
              </span>
              {!typed.includes(address) && (
                <button
                  type="button"
                  onClick={() => setText((t) => (t.trim() ? `${t.trim()}\n${address}` : address))}
                  className="text-ink-3 underline underline-offset-2 hover:text-ink"
                >
                  Add it
                </button>
              )}
            </p>
          )}
          {save.error && field !== "networks" && <Banner tone="error">{save.error.message}</Banner>}
          {changed && (
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="ghost"
                onClick={() => {
                  setText((saved ?? []).join("\n"));
                  save.reset();
                }}
              >
                Cancel
              </Button>
              <Button type="submit" variant="primary" loading={save.isPending}>
                Save
              </Button>
            </div>
          )}
        </form>
      )}
    </Section>
  );
}
