// A project's members (Settings → Members): who works in it and what each may do. Everyone in the
// project sees them; its maintainers add people and change roles, and anyone can leave.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Avatar, Button, Field, Input, Section, Select, type SelectOption, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { manages } from "@/lib/project-page";
import { keys, queryClient, useMe } from "@/lib/queries";

/** Project roles; only owners make owners. */
const projectRoles = (actor?: G.ProjectRole): SelectOption<G.ProjectRole>[] => [
  { value: "viewer", label: "Viewer", description: "Follows sessions" },
  { value: "editor", label: "Editor", description: "Also starts sessions and messages agents" },
  { value: "maintainer", label: "Maintainer", description: "Also manages settings, secrets and members" },
  {
    value: "owner",
    label: "Owner",
    description: "Also deletes the project and changes owners",
    disabled: actor !== "owner",
  },
];

/** Settings → Members: who is in the project and their roles, which its maintainers change. */
export function Members({ project }: { project: G.Project }) {
  const { data: me } = useMe();
  const members = useQuery({ queryKey: keys.members(project.id), queryFn: () => api.members(project.id) });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<G.ProjectRole>("editor");
  const manage = manages(project.role);
  const add = useMutation({
    mutationFn: () => api.addMember(project.id, { email, role }),
    onSuccess: (r) => {
      queryClient.setQueryData(keys.members(project.id), r);
      setEmail("");
    },
  });
  const change = useMutation({
    mutationFn: (v: { user: string; role: G.ProjectRole }) =>
      api.setMemberRole(project.id, v.user, { role: v.role }),
    onSuccess: (r) => queryClient.setQueryData(keys.members(project.id), r),
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (user: string) => api.removeMember(project.id, user),
    onSuccess: () => members.refetch(),
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <Section title="Members" description="Who works in the project, and what each of them may do in it.">
      {manage && (
        <form
          className="mb-6"
          onSubmit={(e) => {
            e.preventDefault();
            add.mutate();
          }}
        >
          <Field label="Add someone by email" error={add.error?.message}>
            {(id) => (
              <div className="flex gap-2">
                <Input
                  id={id}
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="teammate@example.com"
                  className="min-w-0 flex-1"
                />
                <Select
                  aria-label="Role"
                  value={role}
                  onChange={setRole}
                  options={projectRoles(project.role)}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="field"
                  icon={<Plus className="size-3.5" />}
                  loading={add.isPending}
                  disabled={!email}
                >
                  Add
                </Button>
              </div>
            )}
          </Field>
        </form>
      )}
      <ul className="border-t border-line">
        {members.data?.members.map((m) => (
          <li key={m.user.id} className="flex items-center gap-3 border-b border-line py-3">
            <Avatar user={m.user} size={28} />
            <div className="min-w-0 flex-1">
              <div className="text-[14px] text-ink">
                {m.user.name}
                {m.user.id === me?.user.id && <span className="ml-1.5 text-[12px] text-ink-4">you</span>}
              </div>
              <div className="text-[12px] text-ink-4">{m.email}</div>
            </div>
            {manage ? (
              <Select
                size="sm"
                aria-label={`Role of ${m.user.name}`}
                value={m.role}
                onChange={(role) => change.mutate({ user: m.user.id, role })}
                options={projectRoles(project.role)}
              />
            ) : (
              <span className="text-[12.5px] text-ink-3 capitalize">{m.role}</span>
            )}
            {(manage || m.user.id === me?.user.id) && (
              <button
                type="button"
                aria-label={m.user.id === me?.user.id ? `Leave ${project.name}` : `Remove ${m.user.name}`}
                title={m.user.id === me?.user.id ? `Leave ${project.name}` : `Remove ${m.user.name}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => remove.mutate(m.user.id)}
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </li>
        ))}
      </ul>
      <p className="mt-4 text-[12.5px] leading-relaxed text-ink-4">
        Viewers follow sessions. Editors also start sessions and message agents. Maintainers manage settings,
        secrets and members.
      </p>
    </Section>
  );
}
