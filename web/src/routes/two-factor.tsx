// The signed-in layout's door: an account the instance requires two-factor
// sign-in of sees the page that sets it up, until it has, instead of the app.

import { useEffect } from "react";
import { AppShell } from "@/components/shell/app-shell";
import { AuthenticatorSetup } from "@/components/two-factor/setup";
import { toast } from "@/components/ui";
import type * as G from "@/gen";
import { api, TWO_FACTOR_REQUIRED } from "@/lib/api";
import { forgetPush } from "@/lib/push";
import { keys, queryClient, useInstance, useMe } from "@/lib/queries";
import { RECOVERY_LEFT } from "@/lib/two-factor";
import { AuthFrame } from "@/routes/auth";

export function SignedInLayout() {
  const { data: me } = useMe();
  // A request refused for want of two-factor: ask who this is again, so that
  // the page that sets it up shows at once.
  useEffect(() => {
    const again = () => void queryClient.invalidateQueries({ queryKey: keys.me });
    addEventListener(TWO_FACTOR_REQUIRED, again);
    return () => removeEventListener(TWO_FACTOR_REQUIRED, again);
  }, []);
  useRecoveryNotice();
  if (me?.two_factor.setup_required) return <TwoFactorRequired me={me} />;
  return <AppShell />;
}

/** After a sign-in with a recovery code, say once how many are left. */
function useRecoveryNotice() {
  useEffect(() => {
    const left = sessionStorage.getItem(RECOVERY_LEFT);
    if (left === null) return;
    sessionStorage.removeItem(RECOVERY_LEFT);
    const n = Number(left);
    toast(
      n === 0
        ? "That was your last recovery code. Get new ones in Settings."
        : n === 1
          ? "Signed in with a recovery code. 1 is left: get new ones in Settings."
          : `Signed in with a recovery code. ${n} are left.`,
      "info",
    );
  }, []);
}

function TwoFactorRequired({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const signOut = async () => {
    await forgetPush();
    await api.logout().catch(() => {});
    queryClient.clear();
    window.location.assign("/login");
  };
  return (
    <AuthFrame
      wide
      title="Set up two-factor sign-in."
      subtitle={`${instance?.name ?? "This instance"} asks everyone to confirm their sign-ins with a code from an app on their phone. It takes about a minute.`}
      footer={
        <>
          Signed in as {me.user.email} ·{" "}
          <button
            type="button"
            className="text-ink-2 underline underline-offset-2 hover:text-ink"
            onClick={signOut}
          >
            Sign out
          </button>
        </>
      }
    >
      <AuthenticatorSetup
        me={me}
        onDone={(r) => {
          queryClient.setQueryData(keys.me, r.me);
          toast("Two-factor sign-in is on. Welcome in.");
        }}
      />
    </AuthFrame>
  );
}
