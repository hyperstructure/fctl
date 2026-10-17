// Approving `fctl login` on another device.

import { useMutation, useQuery } from "@tanstack/react-query";
import { useSearch } from "@tanstack/react-router";
import { Laptop, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { Banner, Button, Input } from "@/components/ui";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { useMe } from "@/lib/queries";

export function DevicePage() {
  const search = useSearch({ from: "/app/device" });
  const [code, setCode] = useState(search.code ?? "");
  const [entered, setEntered] = useState(search.code ?? "");
  const { data: me } = useMe();
  const info = useQuery({
    queryKey: ["device", entered],
    queryFn: () => api.device(entered),
    enabled: entered.replace(/[^a-z0-9]/gi, "").length === 8,
    retry: false,
  });
  const decide = useMutation({ mutationFn: (approve: boolean) => api.decideDevice(entered, approve) });
  return (
    <div className="mx-auto flex min-h-[80dvh] max-w-md flex-col justify-center px-6">
      <div className="animate-rise">
        <div className="mb-6 flex size-12 items-center justify-center rounded-xl border border-line-strong bg-raised text-accent">
          <Laptop className="size-5" />
        </div>
        <h1 className="font-display text-[30px] leading-tight font-medium tracking-[-0.02em]">
          Sign in on another device
        </h1>
        {decide.isSuccess ? (
          <Banner tone={decide.variables ? "ok" : "info"} className="mt-6">
            {decide.variables ? (
              <>
                Approved. Go back to your terminal — <b>fctl</b> is signed in as {me?.user.name}.
              </>
            ) : (
              "Denied. That device stays signed out."
            )}
          </Banner>
        ) : (
          <>
            <p className="mt-2 text-[14.5px] text-ink-3">
              Check that the code matches the one in your terminal.
            </p>
            <form
              className="mt-6 flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                setEntered(code);
              }}
            >
              <Input
                value={code}
                onChange={(e) => setCode(e.target.value.toUpperCase())}
                placeholder="XXXX-XXXX"
                className="h-12 text-center font-mono text-[20px] tracking-[0.2em]"
                maxLength={9}
                autoFocus
              />
              {!info.data && (
                <Button type="submit" size="lg" disabled={code.replace(/[^a-z0-9]/gi, "").length !== 8}>
                  Continue
                </Button>
              )}
            </form>
            {info.isError && (
              <Banner tone="error" className="mt-4">
                {info.error.message}
              </Banner>
            )}
            {info.data && (
              <div className="mt-6 animate-rise rounded-xl border border-line-strong bg-surface p-5">
                <div className="flex items-start gap-3">
                  <ShieldCheck className="mt-0.5 size-5 text-accent" />
                  <div className="min-w-0 text-[13.5px]">
                    <div className="font-medium text-ink">{info.data.client_name}</div>
                    <div className="mt-1 text-ink-3">
                      Requested {ago(info.data.created_at)}
                      {info.data.location ? ` from ${info.data.location}` : ""}
                      {info.data.ip ? ` (${info.data.ip})` : ""}
                    </div>
                    <div className="mt-2 text-ink-3">
                      It will act as <b className="text-ink-2">{me?.user.email}</b> until you revoke it in
                      Settings.
                    </div>
                  </div>
                </div>
                <div className="mt-5 flex justify-end gap-2">
                  <Button
                    variant="ghost"
                    onClick={() => decide.mutate(false)}
                    loading={decide.isPending && decide.variables === false}
                  >
                    Deny
                  </Button>
                  <Button
                    variant="primary"
                    onClick={() => decide.mutate(true)}
                    loading={decide.isPending && decide.variables === true}
                  >
                    Approve
                  </Button>
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
