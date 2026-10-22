// The task being written on the home page, kept in this browser until a session
// starts with it: leaving the page, reloading it or closing the tab loses
// none of it. The words, the project, the model, effort level and GitHub
// access picked and the caret are in localStorage, which the page reads as
// it opens. The files are in IndexedDB, which keeps them as they are and has
// room for them (localStorage holds a few megabytes of text); one that
// doesn't fit there stays while the page is open. Each person who signs in
// here has their own draft, and signing out forgets them all.

import type * as G from "@/gen";
import { GITHUB_ACCESS } from "./github-access";

const TEXT = "fctl:draft:";
const DB = "fctl-drafts";
const FILES = "files";

export interface DraftText {
  prompt: string;
  /** The project's slug. */
  project?: string;
  /** Where the caret was, or what was selected. */
  selection?: [number, number];
  /** The model and effort level picked for it ("" is Claude's default). */
  model?: string;
  effort?: string;
  /** What its agent may do on GitHub, as picked. */
  github?: G.GithubAccess;
}

/** A file attached to the draft. */
export interface DraftFile {
  key: string;
  name: string;
  file: File;
}

export function loadText(user: string): DraftText | undefined {
  try {
    const d: unknown = JSON.parse(localStorage.getItem(TEXT + user) ?? "null");
    if (!d || typeof d !== "object" || !("prompt" in d) || typeof d.prompt !== "string") return undefined;
    const { project, model, effort, github } = d as Record<string, unknown>;
    const text = (v: unknown) => (typeof v === "string" ? v : undefined);
    const s = "selection" in d && Array.isArray(d.selection) ? d.selection : [];
    const selection =
      s.length === 2 && s.every((n) => Number.isInteger(n) && n >= 0) ? (s as [number, number]) : undefined;
    const access = GITHUB_ACCESS.find((l) => l.value === github)?.value;
    return {
      prompt: d.prompt,
      project: text(project),
      selection,
      model: text(model),
      effort: text(effort),
      github: access,
    };
  } catch {
    return undefined;
  }
}

/** Keeps the draft's text, or forgets it (`undefined`). */
export function saveText(user: string, d: DraftText | undefined) {
  try {
    if (d) localStorage.setItem(TEXT + user, JSON.stringify(d));
    else localStorage.removeItem(TEXT + user);
  } catch {
    // Private browsing without storage, or no room: the draft lasts the page.
  }
}

interface Row {
  /** `user/key`. */
  id: string;
  key: string;
  name: string;
  file: File;
  /** Orders the files as they were attached. */
  at: number;
}

let opened: Promise<IDBDatabase> | undefined;

function database(): Promise<IDBDatabase> {
  opened ??= new Promise<IDBDatabase>((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(FILES, { keyPath: "id" });
    req.onsuccess = () => {
      const db = req.result;
      // A newer version of the app wants the database: let it have it.
      db.onversionchange = () => {
        db.close();
        opened = undefined;
      };
      resolve(db);
    };
    req.onerror = () => reject(req.error);
  }).catch((e) => {
    opened = undefined;
    throw e;
  });
  return opened;
}

/** Runs one request in a transaction and answers its result once the transaction is done. */
async function run<T>(mode: IDBTransactionMode, request: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await database();
  return new Promise<T>((resolve, reject) => {
    const tx = db.transaction(FILES, mode);
    const req = request(tx.objectStore(FILES));
    tx.oncomplete = () => resolve(req.result);
    tx.onabort = () => reject(tx.error ?? new Error("The transaction was aborted"));
  });
}

/** A person's rows: ids are `user/key`. */
const rows = (user: string) => IDBKeyRange.bound(`${user}/`, `${user}/￿`);

/** Orders files across page loads: later ones are attached later. */
let seq = Date.now();

/** The draft's files, in the order they were attached; none when IndexedDB has none or fails. */
export async function loadFiles(user: string): Promise<DraftFile[]> {
  try {
    const found = (await run("readonly", (s) => s.getAll(rows(user)))) as Row[];
    return found.sort((a, b) => a.at - b.at).map(({ key, name, file }) => ({ key, name, file }));
  } catch {
    return [];
  }
}

/** Keeps a file of the draft. False when it doesn't fit, or this browser keeps no files. */
export async function saveFile(user: string, f: DraftFile): Promise<boolean> {
  const row: Row = { id: `${user}/${f.key}`, key: f.key, name: f.name, file: f.file, at: seq++ };
  try {
    await run("readwrite", (s) => s.put(row));
    return true;
  } catch {
    return false;
  }
}

export async function deleteFile(user: string, key: string) {
  await run("readwrite", (s) => s.delete(`${user}/${key}`)).catch(() => {});
}

/** The draft's files as this page has them, those that didn't fit in IndexedDB too, by person. */
const onPage = new Map<string, DraftFile[]>();

export const pageFiles = (user: string): DraftFile[] | undefined => onPage.get(user);

export function setPageFiles(user: string, files: DraftFile[]) {
  onPage.set(user, files);
}

/**
 * Forgets a person's draft, once a session started with it; without a
 * person, every draft in this browser (signing out).
 */
export async function forgetDraft(user?: string) {
  try {
    const keys = user
      ? [TEXT + user]
      : Array.from({ length: localStorage.length }, (_, i) => localStorage.key(i)).filter(
          (k): k is string => !!k?.startsWith(TEXT),
        );
    for (const k of keys) localStorage.removeItem(k);
  } catch {}
  if (user) onPage.delete(user);
  else onPage.clear();
  const files = run("readwrite", (s) => (user ? s.delete(rows(user)) : s.clear())).catch(() => {});
  // Signing out doesn't wait long on a database that doesn't answer.
  await Promise.race([files, new Promise((r) => setTimeout(r, 1000))]);
}
