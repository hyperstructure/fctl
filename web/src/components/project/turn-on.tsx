// The words in a banner that turn one of a project's settings on as they're pressed, for its owners
// and maintainers: Memory's notes and retrospectives say so when they're off.

import { useMutation } from "@tanstack/react-query";
import { toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";

/**
 * For a project's owners and maintainers, the words in a banner that turn a setting on at once: what
 * its Settings → Agents page would, without going there.
 */
export function TurnOn({
  project,
  setting,
  done,
  children,
}: {
  project: G.Project;
  setting: "project_memory" | "retrospectives";
  /** What the toast says once it's on. */
  done: string;
  children: string;
}) {
  const save = useMutation({
    mutationFn: () => api.updateProject(project.id, { settings: { ...project.settings, [setting]: true } }),
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      toast(done);
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <button
      type="button"
      disabled={save.isPending}
      onClick={() => save.mutate()}
      className="font-medium text-ink underline underline-offset-2 transition-opacity disabled:opacity-60"
    >
      {save.isPending ? "Turning it on…" : children}
    </button>
  );
}
