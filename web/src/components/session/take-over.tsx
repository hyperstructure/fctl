// Moving a session to your Claude subscription: from the session's menu, or the line above the
// message box while it waits for its subscription's limit to reset. Messages keep a session on the
// subscription it runs on; this moves it.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Dialog, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { patchSession } from "@/lib/queries";
import type { TakeOver } from "@/lib/subscription";

/** Asks before it moves the session to your subscription, and says what happens then. */
export function TakeOverDialog({
  session,
  takeOver,
  open,
  onClose,
}: {
  session: G.Session;
  takeOver: TakeOver;
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const move = useMutation({
    mutationFn: () => api.takeOver(session.id),
    onSuccess: (updated) => {
      patchSession(queryClient, updated);
      toast("It continues on your Claude subscription");
      onClose();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const from = takeOver.whose
    ? `${takeOver.whose}'s Claude subscription`
    : "another of your Claude connections";
  const then =
    session.status === "stopped" || session.status === "stopping"
      ? "It resumes on yours"
      : "It starts again on yours";
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Continue it on your subscription?"
      description={`It runs on ${from}. ${then}, the connection you chose for this workspace in Settings → Claude, and Claude continues the same conversation. Messages from then on run on yours too.`}
    >
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" loading={move.isPending} onClick={() => move.mutate()}>
          Continue on mine
        </Button>
      </div>
    </Dialog>
  );
}
