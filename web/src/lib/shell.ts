// Commands people copy into a shell, with words that come from elsewhere:
// a repository's file names can hold anything, and must stay one word.

/** `s` as one word of a POSIX shell's command: as it is when every character is plain, else in single quotes. */
export function shellWord(s: string): string {
  return /^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replaceAll("'", `'\\''`)}'`;
}

/**
 * `fctl diff`'s command for one file of a session (`ref`, as `project#12`),
 * and its repository (`owner/name`) when another than the primary one. A
 * path that starts with `-` comes after `--`, where it can't be an option.
 */
export function diffCommand(ref: string, path: string, repo?: string): string {
  const words = ["fctl", "diff", shellWord(ref)];
  if (repo) words.push("--repo", shellWord(repo));
  if (path.startsWith("-")) words.push("--");
  words.push(shellWord(path));
  return words.join(" ");
}
