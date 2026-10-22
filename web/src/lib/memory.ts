// A project's memory: notes agents and people keep for the agents of later
// sessions (fctl_domain::memory on the Rust side).
import type * as G from "@/gen";

/** Who may add to a project's memory, change its notes and delete them. */
export const canEditMemory = (role?: G.ProjectRole) =>
  role === "owner" || role === "maintainer" || role === "editor";

/** What a note is about, on one line: its first line with text, without Markdown's marks. */
export function headline(text: string, max = 120): string {
  const line =
    text
      .split("\n")
      .map((l) =>
        l
          .trim()
          .replace(/^#+\s*/, "")
          .replace(/^[-*>]+\s*/, "")
          .trim(),
      )
      .find((l) => l) ?? "";
  const one = line.split(/\s+/).join(" ");
  if ([...one].length <= max) return one;
  return `${[...one]
    .slice(0, max - 1)
    .join("")
    .trimEnd()}…`;
}

/** Whether a note has every word someone looks for, in its text or where it came from. */
export function matches(m: G.Memory, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = [
    m.text,
    m.session ? `#${m.session.number} ${m.session.title}` : "",
    m.created_by?.name ?? "",
    m.stale?.reason ?? "",
    m.id,
  ]
    .join(" ")
    .toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** The notes a session's agent wrote, and those it marked stale. */
export function ofSession(memories: G.Memory[], session: string) {
  return {
    added: memories.filter((m) => m.session?.id === session),
    staled: memories.filter((m) => m.stale?.session?.id === session && m.session?.id !== session),
  };
}

/** The note that took a stale one's place, when the list has it. */
export const replacement = (memories: G.Memory[], m: G.Memory) =>
  m.stale?.replaced_by ? memories.find((x) => x.id === m.stale?.replaced_by) : undefined;

/** The note a duplicate repeats, when the list has it. */
export const original = (memories: G.Memory[], m: G.Memory) =>
  m.stale?.duplicate_of ? memories.find((x) => x.id === m.stale?.duplicate_of) : undefined;

/** The element id of a note in the project's Memory tab, which links to it scroll to. */
export const noteAnchor = (id: string) => `note-${id}`;

/** The longest note, in characters (fctl_domain::memory::MAX_CHARS). */
export const MAX_NOTE_CHARS = 2000;
