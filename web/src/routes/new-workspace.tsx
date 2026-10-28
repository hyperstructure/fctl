// Creating a workspace: its name, its color, and the address you join it as.

import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { useState } from "react";
import { Banner, Button, Field, Input, Select } from "@/components/ui";
import { WORKSPACE_COLORS, WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient, useMe, useWorkspaces } from "@/lib/queries";
import { limitReached, workspaceCount } from "@/lib/workspaces";

/** Addresses the app's own paths use (fctl_domain::validate::RESERVED_SLUGS). */
const RESERVED = new Set([
  "api",
  "admin",
  "new",
  "settings",
  "login",
  "logout",
  "signup",
  "setup",
  "device",
  "invite",
  "s",
  "p",
  "projects",
  "sessions",
  "runners",
  "assets",
  "static",
  "app",
  "help",
  "docs",
]);

/**
 * What the server makes of the name, for the preview (fctl_domain::workspace::slug_from_name):
 * its slug, and why that one gets a random ending, where the page knows: fctl reserves it, or
 * `taken` has it. The server settles the rest, the addresses of other people's workspaces and those
 * admins reserve.
 */
export function slugOf(
  name: string,
  taken: ReadonlySet<string> = new Set(),
): { slug: string; ending?: "reserved" | "taken" } {
  let s = name
    .replace(/['’]/g, "")
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/, "");
  if (s.length < 2) s = s ? `workspace-${s}` : "workspace";
  if (RESERVED.has(s)) return { slug: s, ending: "reserved" };
  if (taken.has(s)) return { slug: s, ending: "taken" };
  return { slug: s };
}

/** A slug cut to leave room for its random ending of a dash and six hex digits (fctl_domain::slugs). */
export function beforeEnding(slug: string): string {
  return slug.slice(0, 41).replace(/-+$/, "");
}

export function NewWorkspace() {
  const navigate = useNavigate();
  const { data: me } = useMe();
  const list = useWorkspaces();
  const taken = new Set(list.data?.workspaces.map((w) => w.color));
  const [name, setName] = useState("");
  const address = slugOf(name, new Set(list.data?.workspaces.map((w) => w.slug)));
  const [color, setColor] = useState<G.WorkspaceColor>();
  const [email, setEmail] = useState<string>();
  // A color the person's other workspaces don't have, until they pick one.
  const shown = color ?? (["yellow", "pink", "blue", "ink"] as const).find((c) => !taken.has(c)) ?? "yellow";
  const full = list.data && limitReached(list.data);
  const addresses = me?.emails ?? [];
  const create = useMutation({
    mutationFn: () => workspaces.create({ name: name.trim(), color: shown, email }),
    onSuccess: (w) => {
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void navigate({ to: "/w/$slug", params: { slug: w.slug }, search: {} });
    },
  });
  const field = (create.error as { field?: string } | null)?.field;
  return (
    <div className="mx-auto max-w-[640px] px-6 py-14">
      <h1 className="font-display text-[34px] leading-tight font-medium tracking-[-0.02em]">
        A new workspace
      </h1>
      <p className="mt-2 text-[15px] leading-relaxed text-ink-3">
        The people you work with, your projects, the machines that run their agents and the GitHub accounts
        they work in. You are its primary admin.
      </p>
      {full && (
        <Banner tone="warn" className="mt-6">
          {full}
        </Banner>
      )}
      <form
        className="mt-10 flex flex-col gap-7"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) create.mutate();
        }}
      >
        <div className="flex items-center gap-5">
          <WorkspaceAvatar
            workspace={{ name: name.trim() || "New workspace", color: shown }}
            size={72}
            className="transition-colors duration-300"
          />
          <Field
            label="Name"
            className="flex-1"
            error={field === "name" || field === "slug" ? create.error?.message : undefined}
            hint={
              address.ending ? (
                <>
                  <span className="font-mono text-[11.5px]">
                    {location.host}/w/{beforeEnding(address.slug)}-<span className="text-ink-4">······</span>
                  </span>{" "}
                  · {address.slug} is {address.ending}, so it gets a random ending
                </>
              ) : (
                <span className="font-mono text-[11.5px]">
                  {location.host}/w/{address.slug}
                </span>
              )
            }
          >
            {(id) => (
              <Input
                id={id}
                autoFocus
                value={name}
                maxLength={80}
                onChange={(e) => setName(e.target.value)}
                placeholder="Side project"
                className="h-11 text-[17px]"
              />
            )}
          </Field>
        </div>
        <fieldset>
          <legend className="text-[13px] font-medium text-ink-2">Color</legend>
          <p className="mt-0.5 text-[12.5px] text-ink-4">
            Its avatar until an admin uploads an image, so you tell your workspaces apart at a glance.
          </p>
          <div className="mt-3 flex flex-wrap gap-2.5">
            {(Object.keys(WORKSPACE_COLORS) as G.WorkspaceColor[]).map((c) => (
              <button
                key={c}
                type="button"
                aria-pressed={shown === c}
                aria-label={WORKSPACE_COLORS[c].label}
                onClick={() => setColor(c)}
                className={cn(
                  "flex items-center gap-2 rounded-lg border px-2.5 py-2 text-[12.5px] transition-all duration-150",
                  shown === c
                    ? "border-accent/50 bg-accent-soft text-ink shadow-[0_0_0_3px_var(--c-accent-soft)]"
                    : "border-line-strong text-ink-3 hover:border-ink-4 hover:text-ink",
                )}
              >
                <WorkspaceAvatar workspace={{ name: name.trim() || "W", color: c }} size={20} title={false} />
                {WORKSPACE_COLORS[c].label}
              </button>
            ))}
          </div>
        </fieldset>
        {addresses.length > 1 && (
          <Field
            label="You join it as"
            hint="Its people see this address, and your commits in its projects credit its commit email."
            error={field === "email" ? create.error?.message : undefined}
          >
            {(id) => (
              <Select
                id={id}
                value={email ?? me?.user.email}
                onChange={setEmail}
                options={addresses.map((a) => ({
                  value: a.email,
                  label: a.email,
                  description: a.primary ? "Your primary address" : undefined,
                }))}
              />
            )}
          </Field>
        )}
        {create.error && !field && <Banner tone="error">{create.error.message}</Banner>}
        <div className="flex items-center justify-between gap-4">
          <span className="text-[12.5px] text-ink-4">{list.data && workspaceCount(list.data)}</span>
          <Button
            type="submit"
            variant="primary"
            size="lg"
            loading={create.isPending}
            disabled={!name.trim() || !!full}
            trailing={<ArrowRight className="size-4" />}
          >
            Create workspace
          </Button>
        </div>
      </form>
    </div>
  );
}
