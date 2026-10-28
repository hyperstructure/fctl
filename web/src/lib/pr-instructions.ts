// What a project's agents that open pull requests are told about handling them and the issues they
// work on (`pull_request_instructions` in its settings): fctl's defaults until the project writes its
// own, and nothing when it empties them.

import { DEFAULT_PULL_REQUEST_INSTRUCTIONS } from "@/gen/defaults";

export { DEFAULT_PULL_REQUEST_INSTRUCTIONS };

/** What the settings show of what a project stores: fctl's defaults where it stores none. */
export const shownInstructions = (stored: string | null | undefined) =>
  stored ?? DEFAULT_PULL_REQUEST_INSTRUCTIONS;

/** Whether a text says what fctl's defaults say, which the control plane then stores as none. */
export const areDefaults = (text: string | null | undefined) =>
  text == null || text.replace(/\r\n/g, "\n").trim() === DEFAULT_PULL_REQUEST_INSTRUCTIONS;
