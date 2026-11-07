// The switch between the classic session page and the new one, while people compare the two: an
// item in each page's menu and `v`, as Settings → Appearance also has it. This browser shows the
// one picked from then on (lib/session-view.ts).

import { Sparkles, Undo2 } from "lucide-react";
import { MenuItem, toast } from "@/components/ui";
import { otherView, type SessionView, useSessionView } from "@/lib/session-view";
import { useShortcut } from "@/lib/shortcuts";

/** Says which view the page shows now, as the switch leaves it. */
function said(view: SessionView) {
  toast(
    view === "new"
      ? "Sessions open in the new view. Switch back from the ⋯ menu."
      : "Sessions open in the classic view again.",
  );
}

/** The menu item that shows the session in the other view. */
export function ViewSwitch({ onClick }: { onClick: () => void }) {
  const [view, setView] = useSessionView();
  return (
    <MenuItem
      icon={view === "new" ? <Undo2 /> : <Sparkles />}
      onClick={() => {
        onClick();
        const next = otherView(view);
        setView(next);
        said(next);
      }}
    >
      {view === "new" ? (
        "Back to the classic view"
      ) : (
        <span className="flex flex-1 items-center justify-between gap-3">
          Try the new view
          <span className="rounded-full bg-accent-soft px-1.5 py-px text-[10.5px] font-medium text-accent">
            New
          </span>
        </span>
      )}
    </MenuItem>
  );
}

/** `v` on a session's page: the other view. */
export function useViewShortcut() {
  const [view, setView] = useSessionView();
  useShortcut("view", () => {
    const next = otherView(view);
    setView(next);
    said(next);
  });
}
