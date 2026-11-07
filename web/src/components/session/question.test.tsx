import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { QuestionCard } from "./question";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const question: G.SessionQuestion = {
  text: "Round Danish prices to whole kroner, or keep øre?",
  options: ["Round to whole kroner", "Keep øre"],
  asked_at: Date.now() - 4 * 60_000,
};

function setup(props: { canAnswer: boolean; question?: G.SessionQuestion }) {
  const onAnswer = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QuestionCard question={props.question ?? question} canAnswer={props.canAnswer} onAnswer={onAnswer} />,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (name: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent === name);
  return { container, onAnswer, button };
}

describe("the agent's question", () => {
  it("says what it asks, and sends an answer it offers as the reply", async () => {
    const q = setup({ canAnswer: true });
    expect(q.container.textContent).toContain("The agent asks you");
    expect(q.container.textContent).toContain(question.text);
    expect(q.container.textContent).toContain("Or write your own answer below");
    await act(async () => q.button("Keep øre")?.click());
    expect(q.onAnswer).toHaveBeenCalledWith("Keep øre");
  });

  it("offers nothing to click to people who can't message the agent", () => {
    const q = setup({ canAnswer: false });
    expect(q.button("Keep øre")).toBeUndefined();
    expect(q.container.textContent).toContain("The people who can message the agent answer it");
  });

  it("asks for a reply when it offers no answers", () => {
    const q = setup({ canAnswer: true, question: { ...question, options: undefined } });
    expect(q.container.querySelectorAll("button")).toHaveLength(0);
    expect(q.container.textContent).toContain("Your next message answers it");
  });
});
