// Creating a workspace: its name, its color (one of the brand's or one of its own), and the address
// you join it as.

import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { useCallback, useId, useRef, useState } from "react";
import { Banner, Button, Field, Input, Select } from "@/components/ui";
import { ColorPopover } from "@/components/ui/color-popover";
import { WORKSPACE_COLORS, WorkspaceAvatar } from "@/components/workspace/avatar";
import { AvatarThemes, Wheel } from "@/components/workspace/color-field";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { colorName, showHex } from "@/lib/color";
import { BRAND_HEX, FIRST_OWN } from "@/lib/project-icons";
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

const PILL =
  "flex items-center gap-2 rounded-lg border px-2.5 py-2 text-[12.5px] transition-all duration-150";
const PILL_ON = "border-accent/50 bg-accent-soft text-ink shadow-[0_0_0_3px_var(--c-accent-soft)]";
const PILL_OFF = "border-line-strong text-ink-3 hover:border-ink-4 hover:text-ink";

export function NewWorkspace() {
  const navigate = useNavigate();
  const { data: me } = useMe();
  const list = useWorkspaces();
  const taken = new Set(list.data?.workspaces.map((w) => w.color));
  const [name, setName] = useState("");
  const address = slugOf(name, new Set(list.data?.workspaces.map((w) => w.slug)));
  const [color, setColor] = useState<G.WorkspaceColor>();
  // A color of its own, kept while a brand color is tried, and whether it's the one chosen.
  const [own, setOwn] = useState<string>();
  const [custom, setCustom] = useState(false);
  const [editing, setEditing] = useState<false | "pointer" | "keyboard">(false);
  const swatch = useRef<HTMLButtonElement>(null);
  const popoverId = useId();
  const [email, setEmail] = useState<string>();
  // A color the person's other workspaces don't have, until they pick one.
  const shown = color ?? (["yellow", "pink", "blue", "ink"] as const).find((c) => !taken.has(c)) ?? "yellow";
  const hex = custom ? own : undefined;
  const closeEditor = useCallback((refocus: boolean) => {
    setEditing(false);
    if (refocus) swatch.current?.focus();
  }, []);
  const full = list.data && limitReached(list.data);
  const addresses = me?.emails ?? [];
  const create = useMutation({
    mutationFn: () => workspaces.create({ name: name.trim(), color: shown, hex, email }),
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
        {/* Level with the input rather than centred on the field, which grows when its hint wraps. */}
        <div className="flex items-start gap-5">
          <WorkspaceAvatar
            workspace={{ name: name.trim() || "New workspace", color: shown, hex }}
            size={72}
            className="mt-2.75 transition-colors duration-300"
          />
          <Field
            label="Name"
            className="flex-1"
            error={field === "name" || field === "slug" ? create.error?.message : undefined}
            hint={
              address.ending ? (
                <>
                  <span className="font-mono text-[11.5px]">
                    {location.host}/w/{beforeEnding(address.slug)}-<span className="text-ink-4">xxxxxx</span>
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
                aria-pressed={!hex && shown === c}
                aria-label={WORKSPACE_COLORS[c].label}
                onClick={() => {
                  setColor(c);
                  setCustom(false);
                }}
                className={cn(PILL, !hex && shown === c ? PILL_ON : PILL_OFF)}
              >
                <WorkspaceAvatar workspace={{ name: name.trim() || "W", color: c }} size={20} title={false} />
                {WORKSPACE_COLORS[c].label}
              </button>
            ))}
            <button
              ref={swatch}
              type="button"
              aria-pressed={!!hex}
              aria-expanded={!!editing}
              aria-haspopup="dialog"
              aria-controls={editing ? popoverId : undefined}
              aria-label={own ? `Custom color: ${colorName(own)}, ${showHex(own)}` : "Custom color"}
              title={own ? `${showHex(own)}: change it` : undefined}
              // Enter and Space click with no detail; a pointer's click counts its presses.
              onClick={(e) => {
                const byKeyboard = e.detail === 0;
                if (editing) return closeEditor(byKeyboard);
                // It starts from the color of its own it had, else from the brand color it has.
                if (!own) setOwn(shown === "ink" ? FIRST_OWN : BRAND_HEX[shown]);
                setCustom(true);
                setEditing(byKeyboard ? "keyboard" : "pointer");
              }}
              onKeyDown={(e) => {
                // With the focus still here, Escape closes the popover.
                if (e.key === "Escape" && editing) {
                  e.preventDefault();
                  closeEditor(true);
                }
              }}
              className={cn(PILL, hex || editing ? PILL_ON : PILL_OFF)}
            >
              {own ? (
                <WorkspaceAvatar
                  workspace={{ name: name.trim() || "W", color: shown, hex: own }}
                  size={20}
                  title={false}
                />
              ) : (
                <Wheel size={20} />
              )}
              Custom
            </button>
            <ColorPopover
              id={popoverId}
              open={!!editing}
              anchor={swatch}
              hex={own ?? FIRST_OWN}
              preview={<AvatarThemes name={name.trim() || "New workspace"} hex={own ?? FIRST_OWN} />}
              used={[]}
              focusHex={editing === "keyboard"}
              onChange={(h) => {
                setOwn(h);
                setCustom(true);
              }}
              onClose={closeEditor}
            />
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
