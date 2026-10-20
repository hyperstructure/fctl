// Settings → Passkeys: the passkeys someone signs in with.

import { useMutation } from "@tanstack/react-query";
import { Cloud, Fingerprint, KeyRound, Pencil, Smartphone, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Banner, Button, Dialog, Input, Section, Skeleton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import {
  type PasskeySupport,
  passkeySupport,
  rpId,
  signalAccepted,
  signalDetails,
  signalUnknown,
  userHandle,
} from "@/lib/passkeys";
import { keys, queryClient } from "@/lib/queries";
import { AddPasskey } from "./add";

/** Whether this browser takes passkeys; undefined until it has said. */
export function usePasskeySupport(): PasskeySupport | undefined {
  const [support, setSupport] = useState<PasskeySupport>();
  useEffect(() => {
    void passkeySupport().then(setSupport);
  }, []);
  return support;
}

export function PasskeySettings({ me }: { me: G.Me }) {
  const support = usePasskeySupport();
  const [added, setAdded] = useState<string | null>(null);
  const [removing, setRemoving] = useState<G.Passkey | null>(null);
  // Keep the person's passkey provider in step: the passkeys this account
  // has, and, when it changed, the name it lists them by.
  const accepted = me.passkeys.map((p) => p.credential_id).join(",");
  useEffect(() => {
    const handle = userHandle(me.user.id);
    signalAccepted(rpId(), handle, accepted ? accepted.split(",") : []);
    if (accepted) signalDetails(rpId(), handle, me.user.email, me.user.name);
  }, [accepted, me.user.id, me.user.email, me.user.name]);
  const required = me.two_factor.required;
  return (
    <Section
      title="Passkeys"
      description={
        <>
          Sign in with your fingerprint, face or screen lock: no password, no code, and nothing a fake site
          could catch. Your device or password manager keeps them.
          {required && " Here they also count as your second factor."}
        </>
      }
    >
      <div className="flex flex-col gap-5">
        {me.passkeys.length > 0 && (
          <ul className="divide-y divide-line border-y border-line">
            {me.passkeys.map((p) => (
              <PasskeyRow key={p.id} passkey={p} fresh={p.id === added} onRemove={() => setRemoving(p)} />
            ))}
          </ul>
        )}
        {support === undefined ? (
          <Skeleton className="h-8.5 w-40" />
        ) : support.supported ? (
          <div className="flex flex-col items-start gap-2">
            <AddPasskey
              variant={me.passkeys.length ? "secondary" : "primary"}
              label={me.passkeys.length ? "Add another passkey" : "Add a passkey"}
              onAdded={(m) => {
                const newest = m.passkeys.at(-1);
                setAdded(newest?.id ?? null);
                toast(
                  newest
                    ? `${newest.name} is in. Next time, sign in with your fingerprint, face or screen lock.`
                    : "Passkey added",
                );
              }}
            />
            <p className="text-[12.5px] leading-relaxed text-ink-4">
              {support.platform
                ? "This device can keep one, or use your phone or a security key."
                : "Use your phone or a security key, or a password manager that keeps passkeys."}
              {me.passkeys.length === 1 &&
                " A second one, on another device, keeps you in if you lose this one."}
            </p>
          </div>
        ) : (
          <Banner tone="info">This browser can't use passkeys. Most current browsers can.</Banner>
        )}
      </div>
      <RemovePasskey passkey={removing} onClose={() => setRemoving(null)} />
    </Section>
  );
}

function PasskeyRow({
  passkey: p,
  fresh,
  onRemove,
}: {
  passkey: G.Passkey;
  fresh: boolean;
  onRemove: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(p.name);
  const input = useRef<HTMLInputElement>(null);
  const rename = useMutation({
    mutationFn: (n: string) => api.renamePasskey(p.id, n),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      setEditing(false);
    },
    onError: (e) => toast(e.message, "error"),
  });
  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);
  const save = () => {
    const next = name.trim();
    if (!next || next === p.name) {
      setName(p.name);
      setEditing(false);
    } else rename.mutate(next);
  };
  return (
    <li
      className={cn(
        "flex items-center gap-3 py-3 transition-colors duration-700",
        fresh && "animate-rise rounded-md bg-accent-soft/50",
      )}
    >
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md border border-accent/30 bg-accent-soft text-accent">
        {p.synced ? <Fingerprint className="size-4" /> : <KeyRound className="size-4" />}
      </span>
      <div className="min-w-0 flex-1">
        {editing ? (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              save();
            }}
          >
            <Input
              ref={input}
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={save}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setName(p.name);
                  setEditing(false);
                }
              }}
              maxLength={60}
              aria-label="The passkey's name"
              className="h-8 max-w-xs"
              disabled={rename.isPending}
            />
          </form>
        ) : (
          <button
            type="button"
            className="group flex max-w-full items-center gap-1.5 text-left text-[13.5px] text-ink"
            onClick={() => setEditing(true)}
            title="Rename it"
          >
            <span className="truncate">{p.name}</span>
            <Pencil className="size-3 shrink-0 text-ink-4 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
          </button>
        )}
        <div className="flex flex-wrap items-center gap-x-1.5 text-[12px] text-ink-4">
          <span className="inline-flex items-center gap-1">
            {p.synced ? <Cloud className="size-3" /> : <Smartphone className="size-3" />}
            {p.synced ? "Synced" : "This device only"}
          </span>
          <span>· added {ago(p.created_at)}</span>
          <span>· {p.last_used_at ? `last used ${ago(p.last_used_at)}` : "not used yet"}</span>
        </div>
      </div>
      <button
        type="button"
        aria-label={`Remove ${p.name}`}
        className="rounded p-1.5 text-ink-4 hover:bg-hover hover:text-danger"
        onClick={onRemove}
      >
        <Trash2 className="size-3.5" />
      </button>
    </li>
  );
}

function RemovePasskey({ passkey, onClose }: { passkey: G.Passkey | null; onClose: () => void }) {
  const remove = useMutation({
    mutationFn: (p: G.Passkey) => api.removePasskey(p.id),
    onSuccess: (m, p) => {
      queryClient.setQueryData(keys.me, m);
      // Its provider can stop offering it.
      signalUnknown(rpId(), p.credential_id);
      toast(`${p.name} no longer signs you in`);
      onClose();
    },
  });
  return (
    <Dialog
      open={!!passkey}
      onClose={() => {
        remove.reset();
        onClose();
      }}
      title={`Remove ${passkey?.name ?? "this passkey"}?`}
      description={`It stops signing you in here.${
        passkey?.provider ? ` Remove it from ${passkey.provider} too, so that it stops offering it.` : ""
      }`}
    >
      <div className="flex flex-col gap-4">
        {remove.error && <Banner tone="error">{remove.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={remove.isPending}
            onClick={() => passkey && remove.mutate(passkey)}
          >
            Remove
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
