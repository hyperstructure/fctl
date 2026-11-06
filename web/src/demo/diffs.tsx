// A session's diff dialog over nothing, for the video of its diffs following
// its agent (xtask/demo/diff-video.mjs, web/demo/diffs.html). The recorder
// gives it every version of the session's diffs before the page loads
// (`window.fctlDemoDiffs`), each as the agent's state lists the files and
// the diffs the page would fetch, and shows one after another with
// `window.fctlDemoStep(n)`. Nothing here talks to a server. It shows the dialog
// once every file's grammar is loaded: the dialog waits for a file's colors
// before it shows the file, and a grammar loaded as its file comes in takes
// as long as the machine takes, not the recorder's clock.

import "@/styles/app.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { DiffDialog } from "@/components/session/diff-dialog";
import type * as G from "@/gen";
import { firstLine, parsePatch } from "@/lib/diff";
import { tokenize } from "@/lib/highlight";
import { languageOf } from "@/lib/languages";
import { keys } from "@/lib/queries";

/** A changed file, and its diff. */
type File = G.FileChange & { digest: string; patch: string };

/** What the recorder gives the page. */
interface Demo {
  /** Each version of the session's changed files, in the order the agent's state lists them. */
  versions: File[][];
  /** How the CLI names the session (`platform#437`). */
  cliRef: string;
  /** The repository on GitHub (`owner/name`) and the session's branch, for the files' links there. */
  repo?: string;
  branch?: string;
  /** The file the dialog opens at. */
  initial: string;
  theme: "dark" | "light";
}

declare global {
  interface Window {
    fctlDemoDiffs?: Demo;
    fctlDemoStep?: (version: number) => void;
  }
}

const demo = window.fctlDemoDiffs;
const SESSION = "ses_demo";
const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
for (const files of demo?.versions ?? []) {
  for (const f of files) {
    const diff: G.FileDiff = { path: f.path, status: f.status, digest: f.digest, patch: f.patch };
    client.setQueryData(keys.fileDiff(SESSION, "", f.path, f.digest), diff);
  }
}
document.documentElement.dataset.theme = demo?.theme ?? "dark";

function Page({ demo }: { demo: Demo }) {
  const [version, setVersion] = useState(0);
  window.fctlDemoStep = setVersion;
  const files = (demo.versions[version] ?? []).map(({ patch: _, ...f }) => f);
  const { repo, branch } = demo;
  const blob =
    repo && branch ? (path: string) => `https://github.com/${repo}/blob/${branch}/${path}` : undefined;
  return (
    <QueryClientProvider client={client}>
      <DiffDialog
        session={SESSION}
        cliRef={demo.cliRef}
        groups={[{ title: repo, files, blob }]}
        initial={{ path: demo.initial }}
        onClose={() => {}}
      />
    </QueryClientProvider>
  );
}

/** Loads the grammar of each file the versions show, as the dialog picks it. */
function grammars(demo: Demo): Promise<unknown> {
  const langs = new Set(
    demo.versions.flat().map((f) => languageOf(f.path, firstLine(parsePatch(f.patch))).id),
  );
  return Promise.all([...langs].map((lang) => tokenize([""], lang)));
}

const root = document.getElementById("root");
if (root && demo) void grammars(demo).then(() => createRoot(root).render(<Page demo={demo} />));
