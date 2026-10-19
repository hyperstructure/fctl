// Signing in with a passkey: the browser offers them in the email field's
// suggestions where it can (conditional mediation), and a button opens its
// passkey sheet. Both answer the same challenge, fetched when the page opens
// and again before it runs out.

import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import {
  cancelled,
  getPasskey,
  type PasskeySupport,
  passkeyError,
  passkeySupport,
  rpId,
  signalUnknown,
} from "@/lib/passkeys";

/** A challenge works for ten minutes; a new one comes before that. */
const REFRESH_MS = 9 * 60_000;

export function usePasskeySignIn(onSignedIn: (r: G.AuthResponse) => void) {
  const [support, setSupport] = useState<PasskeySupport>();
  // A new round, a new challenge: after a failed attempt.
  const [round, setRound] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const suggestions = useRef<AbortController | null>(null);
  const done = useRef(onSignedIn);
  done.current = onSignedIn;

  useEffect(() => {
    void passkeySupport().then(setSupport);
  }, []);

  const query = useQuery({
    queryKey: ["passkey-sign-in", round],
    queryFn: api.passkeySignInOptions,
    enabled: !!support?.supported,
    retry: false,
    refetchInterval: REFRESH_MS,
    refetchOnWindowFocus: false,
    gcTime: 0,
  });
  const options = query.data ?? null;

  const finish = useCallback(async (credential: G.PasskeyAssertion) => {
    setBusy(true);
    setError(null);
    try {
      done.current(await api.passkeySignIn({ credential }));
    } catch (e) {
      // A passkey fctl doesn't know: its provider can stop offering it.
      if (e instanceof ApiFailure && e.status === 404) signalUnknown(rpId(), credential.id);
      setError(e instanceof Error ? e.message : String(e));
      setRound((r) => r + 1);
    } finally {
      setBusy(false);
    }
  }, []);

  // The email field's suggestions, while the page is open.
  useEffect(() => {
    if (!support?.autofill || !options) return;
    const abort = new AbortController();
    suggestions.current = abort;
    getPasskey(options, { conditional: true, signal: abort.signal })
      .then(finish)
      .catch((e) => {
        if (!cancelled(e)) setError(passkeyError(e));
      });
    return () => abort.abort();
  }, [support?.autofill, options, finish]);

  /** The button: the browser's passkey sheet, at once, since the click is what lets it open. */
  const start = useCallback(() => {
    if (!options) return;
    suggestions.current?.abort();
    setError(null);
    getPasskey(options)
      .then(finish)
      .catch((e) => {
        if (!cancelled(e)) setError(passkeyError(e));
        // The suggestions come back, with a new challenge.
        setRound((r) => r + 1);
      });
  }, [options, finish]);

  return { supported: !!support?.supported, ready: !!options, start, busy, error };
}
