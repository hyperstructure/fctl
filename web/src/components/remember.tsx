// Adding a note to a project's memory, from an insight in a retrospective:
// the text to keep, which the person can shorten first.

import { useMutation } from "@tanstack/react-query";
import { BookMarked } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, Dialog, Field, Textarea, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { MAX_NOTE_CHARS } from "@/lib/memory";
import { keys, queryClient } from "@/lib/queries";

export function RememberDialog({
  project,
  open,
  onClose,
  initial,
  from,
}: {
  project: G.Project;
  open: boolean;
  onClose: () => void;
  /** The text it starts with. */
  initial: string;
  /** Where the text comes from, for the dialog's description. */
  from: string;
}) {
  const [text, setText] = useState(initial);
  useEffect(() => {
    if (open) setText(initial.trim());
  }, [open, initial]);
  const add = useMutation({
    mutationFn: () => api.addMemory(project.id, { text }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.memories(project.id) });
      toast(
        project.settings.project_memory
          ? "Remembered: agents start with it"
          : "Added to the project's memory",
      );
      onClose();
    },
  });
  const left = MAX_NOTE_CHARS - [...text].length;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Remember for later agents"
      description={`${from}. Each new session's agent in ${project.name} starts with the project's notes.`}
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) add.mutate();
        }}
      >
        <Field
          label="Note"
          hint={
            left < 200
              ? `${left} characters left`
              : "One fact, in a sentence or two, that holds beyond one task."
          }
          error={add.error?.message}
        >
          {(id) => (
            <Textarea
              id={id}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && text.trim()) {
                  e.preventDefault();
                  add.mutate();
                }
              }}
              minRows={3}
              maxRows={10}
              autoFocus
            />
          )}
        </Field>
        {!project.settings.project_memory && (
          <p className="-mt-2 text-[12.5px] leading-relaxed text-ink-3">
            This project's agents keep no memory now: they start with its notes once its maintainers turn it
            on.
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            icon={<BookMarked className="size-3.5" />}
            loading={add.isPending}
            disabled={!text.trim() || left < 0}
          >
            Remember
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
