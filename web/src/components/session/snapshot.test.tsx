import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { SnapshotRow } from "./snapshot";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  document.body.innerHTML = "";
});

function text(s: G.SessionSnapshot): string {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<SnapshotRow snapshot={s} />));
  roots.push(() => act(() => root.unmount()));
  return container.textContent ?? "";
}

const saved = Date.now() - 2 * 3_600_000;

describe("SnapshotRow", () => {
  it("says what the session's snapshot holds, where it was saved, and until when", () => {
    const t = text({
      saved_at: saved,
      saved_on: "aws-7",
      size: 12_400_000,
      conversation: true,
      changes: true,
      expires_at: saved + 30 * 86_400_000,
    });
    expect(t).toContain("Snapshot12 MB · 2 hours ago");
    expect(t).toContain("Its conversation and uncommitted work, saved on aws-7, kept by fctl.");
    expect(t).toContain("A resume on another runner restores it until");
  });

  it("names the bucket one is in, and what it left out", () => {
    const t = text({
      saved_at: saved,
      saved_on: "aws-9",
      size: 2_000,
      conversation: true,
      changes: false,
      left_out: ["images"],
      location: "s3://acme-fctl/sessions/ses_1/snp_2",
    });
    expect(t).toContain("Its conversation, without its images, saved on aws-9, in s3://acme-fctl.");
  });
});
