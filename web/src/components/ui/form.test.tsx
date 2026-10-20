import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SecretInput } from "./form";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("SecretInput", () => {
  it("masks a text field, which browsers don't fill saved passwords into or spellcheck", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    act(() => root.render(<SecretInput id="value" className="font-mono" />));
    const input = container.querySelector("input");
    expect(input?.type).toBe("text");
    expect(input?.className).toContain("[-webkit-text-security:disc]");
    expect(input?.className).toContain("font-mono");
    expect(input?.getAttribute("autocomplete")).toBe("off");
    expect(input?.getAttribute("autocapitalize")).toBe("off");
    expect(input?.getAttribute("autocorrect")).toBe("off");
    expect(input?.getAttribute("spellcheck")).toBe("false");
    act(() => root.unmount());
    container.remove();
  });
});
