// A project's secrets (Settings → Secrets): credentials its agents get as environment variables,
// which nobody sees again once saved. Its owners and maintainers manage them.

import { useMutation, useQuery } from "@tanstack/react-query";
import { KeyRound, Trash2 } from "lucide-react";
import { useState } from "react";
import { Banner, Button, Field, Input, SecretInput, Section, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { keys, queryClient } from "@/lib/queries";

/** Settings → Secrets: their names, saving a new one or a new value, and deleting them. */
export function Secrets({ project }: { project: G.Project }) {
  const secrets = useQuery({ queryKey: keys.secrets(project.id), queryFn: () => api.secrets(project.id) });
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const set = useMutation({
    mutationFn: () => api.setSecret(project.id, name.trim().toUpperCase(), value),
    onSuccess: (r) => {
      queryClient.setQueryData(keys.secrets(project.id), r);
      setName("");
      setValue("");
      toast("Saved: sessions that start from now on have it");
    },
  });
  const del = useMutation({
    mutationFn: (n: string) => api.deleteSecret(project.id, n),
    onSuccess: (r, n) => {
      queryClient.setQueryData(keys.secrets(project.id), r);
      toast(`Deleted ${n}: sessions that start from now on don't have it`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  /** What the control plane said of a field, under it; anything else goes under the form. */
  const field = (set.error as { field?: string } | null)?.field;
  const err = (f: string) => (field === f ? set.error?.message : undefined);
  return (
    <Section
      title="Secrets"
      description="Credentials its agents need, such as API keys. fctl encrypts them, hands them to sandboxes as environment variables, and never shows them again, not even to you."
    >
      {/* The fields line up by their labels, so an error under the name moves nothing.
          On a phone, the value goes under the name. */}
      <form
        className="mb-6 grid gap-3 sm:grid-cols-[1fr_1.4fr] sm:items-start sm:gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          set.mutate();
        }}
      >
        <Field
          label="Name"
          hint="Names fctl manages, such as ANTHROPIC_* and GH_TOKEN, are refused."
          error={err("name")}
        >
          {(id) => (
            <Input
              id={id}
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="font-mono uppercase"
              placeholder="STRIPE_KEY"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
            />
          )}
        </Field>
        <Field label="Value" error={err("value")}>
          {(id) => (
            <div className="flex gap-2">
              <SecretInput
                id={id}
                value={value}
                onChange={(e) => setValue(e.target.value)}
                className="min-w-0 flex-1"
              />
              <Button
                type="submit"
                variant="primary"
                size="field"
                icon={<KeyRound className="size-3.5" />}
                loading={set.isPending}
                disabled={!name || !value}
              >
                Save
              </Button>
            </div>
          )}
        </Field>
      </form>
      {set.error && field !== "name" && field !== "value" && (
        <Banner tone="error" className="-mt-2 mb-6">
          {set.error.message}
        </Banner>
      )}
      <ul className="border-t border-line">
        {secrets.data?.secrets.map((x) => (
          <li key={x.name} className="flex items-center gap-3 border-b border-line py-3">
            <code className="min-w-0 flex-1 font-mono text-[13px] text-ink [overflow-wrap:anywhere]">
              {x.name}
            </code>
            <span className="text-[12px] text-ink-4">
              updated {ago(x.updated_at)}
              {x.updated_by ? ` by ${x.updated_by.name}` : ""}
            </span>
            <button
              type="button"
              aria-label={`Delete ${x.name}`}
              className="rounded p-1.5 text-ink-4 hover:text-danger"
              onClick={() => del.mutate(x.name)}
            >
              <Trash2 className="size-3.5" />
            </button>
          </li>
        ))}
        {secrets.data?.secrets.length === 0 && (
          <li className="py-6 text-[13px] text-ink-4">
            No secrets yet. Sessions that start after you save one have it as an environment variable.
          </li>
        )}
      </ul>
    </Section>
  );
}
