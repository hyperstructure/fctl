import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiFailure, api, clickConfirmed, confirmFirst, goConfirmed, setConfirmer } from "./api";

const asked = {
  code: "verification_required",
  message: "Confirm it's you to delete a project, with your passkey.",
};

/** Answers the requests in turn, and keeps them. */
function answer(...answers: { status: number; body: unknown }[]) {
  const sent: { method: string; url: string; body?: string }[] = [];
  const fetch = vi.fn(async (url: string, init: RequestInit = {}) => {
    sent.push({ method: init.method ?? "GET", url, body: init.body as string | undefined });
    const next = answers.shift() ?? { status: 500, body: { code: "internal", message: "No answer" } };
    return new Response(JSON.stringify(next.body), { status: next.status });
  });
  vi.stubGlobal("fetch", fetch);
  return sent;
}

const unregister: (() => void)[] = [];
afterEach(() => {
  for (const u of unregister.splice(0)) u();
  vi.unstubAllGlobals();
});

function confirmer(answer: boolean) {
  const c = vi.fn(async () => answer);
  unregister.push(setConfirmer(c));
  return c;
}

describe("a sensitive action", () => {
  it("goes again once the person confirmed it's them", async () => {
    const sent = answer({ status: 403, body: asked }, { status: 200, body: { ok: true } });
    const c = confirmer(true);
    await expect(api.deleteProject("old")).resolves.toEqual({ ok: true });
    expect(c).toHaveBeenCalledTimes(1);
    expect(sent.map((r) => `${r.method} ${r.url}`)).toEqual([
      "DELETE /api/v1/projects/old",
      "DELETE /api/v1/projects/old",
    ]);
  });

  it("goes again with the same body", async () => {
    const sent = answer({ status: 403, body: asked }, { status: 201, body: { secret: "fctl_pat_x" } });
    confirmer(true);
    await api.createToken({ name: "CI" });
    expect(sent[1]?.body).toBe(sent[0]?.body);
    expect(JSON.parse(sent[1]?.body ?? "{}")).toEqual({ name: "CI" });
  });

  it("fails as the server said when the person doesn't confirm", async () => {
    const sent = answer({ status: 403, body: asked });
    confirmer(false);
    const failure = await api.deleteProject("old").catch((e) => e);
    expect(failure).toBeInstanceOf(ApiFailure);
    expect(failure.code).toBe("verification_required");
    expect(failure.message).toBe(asked.message);
    expect(sent).toHaveLength(1);
  });

  it("asks only once, and fails when the server still asks", async () => {
    answer({ status: 403, body: asked }, { status: 403, body: asked });
    const c = confirmer(true);
    await expect(api.deleteProject("old")).rejects.toMatchObject({ code: "verification_required" });
    expect(c).toHaveBeenCalledTimes(1);
  });

  it("fails without asking when nothing can ask", async () => {
    answer({ status: 403, body: asked });
    await expect(api.deleteProject("old")).rejects.toMatchObject({ code: "verification_required" });
  });

  it("doesn't ask for other refusals", async () => {
    answer({ status: 403, body: { code: "forbidden", message: "Only the project's owners can delete it" } });
    const c = confirmer(true);
    await expect(api.deleteProject("old")).rejects.toMatchObject({ code: "forbidden" });
    expect(c).not.toHaveBeenCalled();
  });

  it("stops asking once the dialog is gone", async () => {
    answer({ status: 403, body: asked });
    const c = vi.fn(async () => true);
    setConfirmer(c)();
    await expect(api.deleteProject("old")).rejects.toMatchObject({ code: "verification_required" });
    expect(c).not.toHaveBeenCalled();
  });
});

describe("before a download", () => {
  const status = (required: boolean, verified: boolean) => ({
    status: 200,
    body: { required, verified, methods: ["passkey"] },
  });

  it("goes on without asking where the instance doesn't ask, or the browser confirmed already", async () => {
    const c = confirmer(false);
    answer(status(false, false));
    await expect(confirmFirst()).resolves.toBe(true);
    answer(status(true, true));
    await expect(confirmFirst()).resolves.toBe(true);
    expect(c).not.toHaveBeenCalled();
  });

  it("asks when a sensitive action would, and goes on only once confirmed", async () => {
    answer(status(true, false));
    confirmer(true);
    await expect(confirmFirst()).resolves.toBe(true);
    for (const u of unregister.splice(0)) u();
    answer(status(true, false));
    confirmer(false);
    await expect(confirmFirst()).resolves.toBe(false);
  });
});

describe("links that take confirming it's you", () => {
  const status = (required: boolean, verified: boolean) => ({
    status: 200,
    body: { required, verified, methods: ["passkey"] },
  });
  const LINK = "/api/v1/linear/link";
  /** The browser's own navigation, as a spy. */
  function navigation() {
    const assign = vi.fn();
    vi.stubGlobal("location", { assign });
    return assign;
  }
  const click = (keys: Partial<Record<"metaKey" | "ctrlKey" | "shiftKey" | "altKey", boolean>> = {}) => ({
    button: 0,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...keys,
    preventDefault: vi.fn(),
  });

  it("goes once confirmed, and stays when the person cancels", async () => {
    const assign = navigation();
    answer(status(true, false));
    confirmer(true);
    await goConfirmed(LINK);
    expect(assign).toHaveBeenCalledWith(LINK);
    for (const u of unregister.splice(0)) u();
    assign.mockClear();
    answer(status(true, false));
    confirmer(false);
    await goConfirmed(LINK);
    expect(assign).not.toHaveBeenCalled();
  });

  it("takes a plain click, and leaves one that opens a new tab to the browser", async () => {
    const assign = navigation();
    answer(status(false, false));
    const plain = click();
    await clickConfirmed(plain, LINK);
    expect(plain.preventDefault).toHaveBeenCalled();
    expect(assign).toHaveBeenCalledWith(LINK);
    assign.mockClear();
    for (const keys of [{ metaKey: true }, { ctrlKey: true }, { shiftKey: true }]) {
      const e = click(keys);
      expect(clickConfirmed(e, LINK)).toBeUndefined();
      expect(e.preventDefault).not.toHaveBeenCalled();
    }
    expect(assign).not.toHaveBeenCalled();
  });
});
