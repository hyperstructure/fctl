// Adding a passkey: Settings, the page that requires two-factor and the offer
// after a password sign-in share it.
//
// Browsers (Safari strictly) open the passkey sheet only right after a click,
// so the options are fetched before it, and the click asks the browser at
// once. When the server wants proof first (the password, or a code, unless
// this browser just signed in), a dialog asks for it, then for the click.

import { useQuery } from "@tanstack/react-query";
import { Fingerprint } from "lucide-react";
import { useState } from "react";
import { Banner, Button, CodeInput, Dialog, Field, Input } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { browserName } from "@/lib/format";
import { cancelled, createPasskey, passkeyError } from "@/lib/passkeys";
import { keys, queryClient } from "@/lib/queries";
import { CODE_LENGTH } from "@/lib/two-factor";

type Proof = "password" | "code" | "sign_in";

const PROOFS: Proof[] = ["password", "code", "sign_in"];

/** Options for a new passkey, fetched ahead of the click; or what the server wants first. */
function useOptions(enabled: boolean) {
  return useQuery({
    queryKey: ["passkey-options"],
    queryFn: () => api.passkeyOptions(),
    enabled,
    retry: false,
    // A challenge works for ten minutes.
    staleTime: 8 * 60_000,
    gcTime: 9 * 60_000,
    refetchOnWindowFocus: false,
  });
}

export function useAddPasskey({
  enabled = true,
  onAdded,
}: {
  enabled?: boolean;
  onAdded: (me: G.Me) => void;
}) {
  const options = useOptions(enabled);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The server wants proof: the dialog asks for it.
  const [proof, setProof] = useState<Proof | null>(null);
  const failure = options.error instanceof ApiFailure ? options.error : null;
  const needs = failure?.field && PROOFS.includes(failure.field as Proof) ? (failure.field as Proof) : null;

  const make = async (o: G.PasskeyCreationOptions) => {
    setBusy(true);
    setError(null);
    try {
      const credential = await createPasskey(o);
      const me = await api.addPasskey({ credential, name: browserName(navigator.userAgent) });
      queryClient.setQueryData(keys.me, me);
      setProof(null);
      onAdded(me);
    } catch (e) {
      if (!cancelled(e)) setError(e instanceof ApiFailure ? e.message : passkeyError(e));
    } finally {
      setBusy(false);
      // Each challenge works once.
      void queryClient.invalidateQueries({ queryKey: ["passkey-options"] });
    }
  };

  /** The button's click: the passkey sheet at once, or the dialog that asks for proof. */
  const start = () => {
    setError(null);
    if (options.data) void make(options.data);
    else if (needs) setProof(needs);
    else if (options.error) setError(options.error.message);
  };

  return {
    start,
    make,
    busy: busy || (options.isFetching && !options.data && !needs),
    error,
    proof,
    closeProof: () => setProof(null),
  };
}

/** A button that adds a passkey, with the dialog that asks for proof when the server wants it. */
export function AddPasskey({
  onAdded,
  label = "Add a passkey",
  variant = "secondary",
  size,
  className,
}: {
  onAdded: (me: G.Me) => void;
  label?: string;
  variant?: "primary" | "secondary";
  size?: "sm" | "md" | "lg";
  className?: string;
}) {
  const add = useAddPasskey({ onAdded });
  return (
    <>
      <Button
        variant={variant}
        size={size}
        className={className}
        icon={<Fingerprint className="size-4" />}
        loading={add.busy && !add.proof}
        onClick={add.start}
      >
        {label}
      </Button>
      {add.error && !add.proof && (
        <Banner tone="error" className="mt-3">
          {add.error}
        </Banner>
      )}
      <ProveIt
        proof={add.proof}
        busy={add.busy}
        error={add.error}
        onClose={add.closeProof}
        onReady={(o) => void add.make(o)}
      />
    </>
  );
}

/** Asks for the password (or a code) before a passkey is made, then for the click that makes it. */
export function ProveIt({
  proof,
  busy,
  error,
  onClose,
  onReady,
}: {
  proof: Proof | null;
  busy: boolean;
  error: string | null;
  onClose: () => void;
  onReady: (o: G.PasskeyCreationOptions) => void;
}) {
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [checking, setChecking] = useState(false);
  const [failure, setFailure] = useState<ApiFailure | null>(null);
  const [ready, setReady] = useState<G.PasskeyCreationOptions | null>(null);
  const close = () => {
    setPassword("");
    setCode("");
    setFailure(null);
    setReady(null);
    onClose();
  };
  const check = async () => {
    setChecking(true);
    setFailure(null);
    try {
      setReady(await api.passkeyOptions(proof === "password" ? { password } : { code }));
    } catch (e) {
      setFailure(e instanceof ApiFailure ? e : new ApiFailure(0, { message: String(e) }));
      setCode("");
    } finally {
      setChecking(false);
    }
  };
  return (
    <Dialog
      open={!!proof}
      onClose={close}
      title={ready ? "Now the passkey" : "Confirm it's you"}
      description={
        ready
          ? "Your browser asks for your fingerprint, face or screen lock, or a phone or security key."
          : proof === "sign_in"
            ? "Adding a passkey takes a sign-in from the last 15 minutes."
            : "A passkey signs in without a password or code, so adding one takes proof it's you."
      }
    >
      {ready ? (
        <div className="flex flex-col gap-4">
          {error && <Banner tone="error">{error}</Banner>}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button
              variant="primary"
              icon={<Fingerprint className="size-4" />}
              loading={busy}
              onClick={() => onReady(ready)}
              autoFocus
            >
              Add the passkey
            </Button>
          </div>
        </div>
      ) : proof === "sign_in" ? (
        <div className="flex flex-col gap-4">
          <p className="text-[13.5px] leading-relaxed text-ink-2">
            Sign out, sign in again, and add it within 15 minutes.
          </p>
          <div className="flex justify-end">
            <Button onClick={close}>Close</Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            void check();
          }}
        >
          {proof === "password" ? (
            <Field label="Your password" error={failure?.field === "password" ? failure.message : undefined}>
              {(id) => (
                <Input
                  id={id}
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoFocus
                />
              )}
            </Field>
          ) : (
            <div className="flex flex-col gap-2">
              <span className="text-[12.5px] font-medium text-ink-2">Code from your authenticator app</span>
              <CodeInput
                value={code}
                onChange={setCode}
                onComplete={() => void check()}
                state={checking ? "busy" : failure ? "error" : "idle"}
                label="The six-digit code from your authenticator app"
                autoFocus
              />
              {failure && <p className="text-[12.5px] text-danger">{failure.message}</p>}
            </div>
          )}
          {failure && !failure.field && <Banner tone="error">{failure.message}</Banner>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              loading={checking}
              disabled={proof === "password" ? !password : code.length !== CODE_LENGTH}
            >
              Continue
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** The proof dialog for `useAddPasskey`, where a page draws its own button. */
export function AddPasskeyProof({ add }: { add: ReturnType<typeof useAddPasskey> }) {
  return (
    <ProveIt
      proof={add.proof}
      busy={add.busy}
      error={add.error}
      onClose={add.closeProof}
      onReady={(o) => void add.make(o)}
    />
  );
}
