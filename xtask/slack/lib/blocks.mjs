// Block Kit, as the stand-in takes it: what Slack refuses of a message's
// blocks or a view (its documented limits, docs.slack.dev/reference/
// block-kit, in Slack's own words and JSON pointers), the ids Slack fills
// in where an app gives none, the buttons and menus people press, and a
// plain rendering of blocks for people reading the stand-in's state.
//
// It checks what Slack documents and nothing it doesn't: a block type it
// doesn't know fails here as it does in Slack, but a field Slack might take
// passes.

import { randomBytes } from "node:crypto";

/** Blocks Slack takes in messages. */
const MESSAGE_BLOCKS = new Set([
  "actions",
  "alert",
  "card",
  "carousel",
  "container",
  "context",
  "context_actions",
  "data_table",
  "data_visualization",
  "divider",
  "file",
  "header",
  "image",
  "input",
  "layout",
  "markdown",
  "plan",
  "rich_text",
  "section",
  "table",
  "task_card",
  "video",
]);

/** Blocks Slack takes in modals and the Home tab: `markdown`, `plan` and `task_card` are for messages only. */
const VIEW_BLOCKS = new Set([
  "actions",
  "context",
  "divider",
  "header",
  "image",
  "input",
  "rich_text",
  "section",
  "video",
]);

/** Elements that people use, and what each holds in `state.values`. */
const ELEMENTS = new Set([
  "button",
  "checkboxes",
  "channels_select",
  "conversations_select",
  "datepicker",
  "datetimepicker",
  "email_text_input",
  "external_select",
  "feedback_buttons",
  "file_input",
  "icon_button",
  "multi_channels_select",
  "multi_conversations_select",
  "multi_external_select",
  "multi_static_select",
  "multi_users_select",
  "number_input",
  "overflow",
  "plain_text_input",
  "radio_buttons",
  "rich_text_input",
  "static_select",
  "timepicker",
  "url_text_input",
  "users_select",
  "workflow_button",
]);

const PLAN_STATUSES = new Set(["pending", "in_progress", "complete", "error"]);

/** Characters as Slack counts them: code points. */
const chars = (s) => [...String(s)].length;

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/** Whether two option objects are the same, whatever the order of their keys. */
function same(a, b) {
  if (Array.isArray(a) || Array.isArray(b)) {
    return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => same(x, b[i]));
  }
  if (isObject(a) && isObject(b)) {
    const ka = Object.keys(a).filter((k) => a[k] !== undefined);
    const kb = Object.keys(b).filter((k) => b[k] !== undefined);
    return ka.length === kb.length && ka.every((k) => same(a[k], b[k]));
  }
  return a === b;
}

/** A checker that collects problems at JSON pointers. */
function checker(base) {
  const problems = [];
  const at = (pointer, message) => problems.push({ pointer: `${base}${pointer}`, message });
  return { problems, at };
}

function text(c, t, p, { plain = false, max = 3000, required = true } = {}) {
  if (t === undefined || t === null) {
    if (required) c.at(p, "must provide an object");
    return;
  }
  if (!isObject(t)) return c.at(p, "must provide an object");
  if (t.type !== "plain_text" && t.type !== "mrkdwn") {
    return c.at(`${p}/type`, "must be one of [plain_text, mrkdwn]");
  }
  if (plain && t.type !== "plain_text") return c.at(`${p}/type`, "must be plain_text");
  if (typeof t.text !== "string") return c.at(`${p}/text`, "must provide a string");
  if (chars(t.text) < 1) return c.at(`${p}/text`, "must be more than 0 characters");
  if (chars(t.text) > max) c.at(`${p}/text`, `must be less than ${max + 1} characters`);
}

function string(c, v, p, max, { required = false } = {}) {
  if (v === undefined || v === null) {
    if (required) c.at(p, "must provide a string");
    return;
  }
  if (typeof v !== "string") return c.at(p, "must provide a string");
  if (required && v.length === 0) return c.at(p, "must be more than 0 characters");
  if (chars(v) > max) c.at(p, `must be less than ${max + 1} characters`);
}

function options(c, opts, p, { min = 1, max, plain, textMax = 75 }) {
  if (!Array.isArray(opts)) return c.at(p, "must provide an array");
  if (opts.length < min) c.at(p, `must provide at least ${min} items`);
  if (opts.length > max) c.at(p, `must provide no more than ${max} items`);
  for (const [i, o] of opts.entries()) {
    if (!isObject(o)) {
      c.at(`${p}/${i}`, "must provide an object");
      continue;
    }
    text(c, o.text, `${p}/${i}/text`, { plain, max: textMax });
    string(c, o.value, `${p}/${i}/value`, 150, { required: true });
    text(c, o.description, `${p}/${i}/description`, { plain, max: 75, required: false });
  }
}

function element(c, e, p) {
  if (!isObject(e)) return c.at(p, "must provide an object");
  if (e.type === "image") return image(c, e, p);
  if (!ELEMENTS.has(e.type)) return c.at(`${p}/type`, `${JSON.stringify(e.type)} isn't an element Slack has`);
  string(c, e.action_id, `${p}/action_id`, 255);
  switch (e.type) {
    case "button":
      text(c, e.text, `${p}/text`, { plain: true, max: 75 });
      string(c, e.url, `${p}/url`, 3000);
      string(c, e.value, `${p}/value`, 2000);
      string(c, e.accessibility_label, `${p}/accessibility_label`, 75);
      if (e.style !== undefined && e.style !== "primary" && e.style !== "danger") {
        c.at(`${p}/style`, "must be one of [primary, danger]");
      }
      break;
    case "static_select":
    case "multi_static_select": {
      text(c, e.placeholder, `${p}/placeholder`, { plain: true, max: 150, required: false });
      if (e.option_groups === undefined) {
        options(c, e.options, `${p}/options`, { max: 100, plain: true });
        const all = Array.isArray(e.options) ? e.options : [];
        if (e.initial_option !== undefined && !all.some((o) => same(o, e.initial_option))) {
          c.at(`${p}/initial_option`, "must match one of the options");
        }
        for (const [i, o] of (e.initial_options ?? []).entries()) {
          if (!all.some((x) => same(x, o)))
            c.at(`${p}/initial_options/${i}`, "must match one of the options");
        }
      }
      break;
    }
    case "checkboxes":
    case "radio_buttons": {
      options(c, e.options, `${p}/options`, { max: 10, plain: false });
      const all = Array.isArray(e.options) ? e.options : [];
      const initial =
        e.type === "checkboxes" ? (e.initial_options ?? []) : [e.initial_option].filter(Boolean);
      for (const [i, o] of initial.entries()) {
        if (!all.some((x) => same(x, o))) {
          c.at(
            `${p}/${e.type === "checkboxes" ? `initial_options/${i}` : "initial_option"}`,
            "must match one of the options",
          );
        }
      }
      break;
    }
    case "overflow":
      options(c, e.options, `${p}/options`, { min: 1, max: 5, plain: true });
      break;
    case "plain_text_input":
      text(c, e.placeholder, `${p}/placeholder`, { plain: true, max: 150, required: false });
      string(c, e.initial_value, `${p}/initial_value`, 3000);
      if (e.max_length !== undefined && !(e.max_length >= 1 && e.max_length <= 3000)) {
        c.at(`${p}/max_length`, "must be between 1 and 3000");
      }
      break;
    default:
      break;
  }
}

function image(c, e, p) {
  if (typeof e.image_url !== "string" && !isObject(e.slack_file)) {
    c.at(p, "must provide image_url or slack_file");
  }
  string(c, e.image_url, `${p}/image_url`, 3000);
  string(c, e.alt_text, `${p}/alt_text`, 2000, { required: true });
}

function actionIds(c, elements, p) {
  const seen = new Set();
  elements.forEach((e, i) => {
    if (!isObject(e) || typeof e.action_id !== "string") return;
    if (seen.has(e.action_id)) c.at(`${p}/${i}/action_id`, "must be unique within the block");
    seen.add(e.action_id);
  });
}

function block(c, b, p, surface, totals) {
  if (!isObject(b)) return c.at(p, "must provide an object");
  const known = surface === "message" ? MESSAGE_BLOCKS : VIEW_BLOCKS;
  if (!known.has(b.type)) {
    if (MESSAGE_BLOCKS.has(b.type)) return c.at(`${p}/type`, `${b.type} blocks are for messages only`);
    return c.at(`${p}/type`, `${JSON.stringify(b.type)} isn't a block Slack has`);
  }
  string(c, b.block_id, `${p}/block_id`, 255);
  switch (b.type) {
    case "section":
      if (b.text === undefined && b.fields === undefined) c.at(p, "must provide text or fields");
      text(c, b.text, `${p}/text`, { max: 3000, required: false });
      if (b.fields !== undefined) {
        if (!Array.isArray(b.fields)) c.at(`${p}/fields`, "must provide an array");
        else {
          if (b.fields.length > 10) c.at(`${p}/fields`, "must provide no more than 10 items");
          b.fields.forEach((f, i) => {
            text(c, f, `${p}/fields/${i}`, { max: 2000 });
          });
        }
      }
      if (b.accessory !== undefined) element(c, b.accessory, `${p}/accessory`);
      break;
    case "context":
      if (!Array.isArray(b.elements) || b.elements.length === 0) {
        c.at(`${p}/elements`, "must provide at least 1 items");
        break;
      }
      if (b.elements.length > 10) c.at(`${p}/elements`, "must provide no more than 10 items");
      b.elements.forEach((e, i) => {
        if (isObject(e) && e.type === "image") image(c, e, `${p}/elements/${i}`);
        else text(c, e, `${p}/elements/${i}`, { max: 3000 });
      });
      break;
    case "actions":
      if (!Array.isArray(b.elements) || b.elements.length === 0) {
        c.at(`${p}/elements`, "must provide at least 1 items");
        break;
      }
      if (b.elements.length > 25) c.at(`${p}/elements`, "must provide no more than 25 items");
      b.elements.forEach((e, i) => {
        element(c, e, `${p}/elements/${i}`);
      });
      actionIds(c, b.elements, `${p}/elements`);
      break;
    case "header":
      text(c, b.text, `${p}/text`, { plain: true, max: 150 });
      break;
    case "markdown":
      if (typeof b.text !== "string") c.at(`${p}/text`, "must provide a string");
      else if (b.text.length === 0) c.at(`${p}/text`, "must be more than 0 characters");
      else totals.markdown += chars(b.text);
      break;
    case "plan": {
      if (typeof b.title !== "string" || b.title.length === 0) c.at(`${p}/title`, "must provide a string");
      if (!Array.isArray(b.tasks)) {
        c.at(`${p}/tasks`, "must provide an array");
        break;
      }
      if (b.tasks.length > 50) c.at(`${p}/tasks`, "must provide no more than 50 items");
      const ids = new Set();
      for (const [i, t] of b.tasks.entries()) {
        const q = `${p}/tasks/${i}`;
        if (!isObject(t)) {
          c.at(q, "must provide an object");
          continue;
        }
        string(c, t.task_id, `${q}/task_id`, 255, { required: true });
        string(c, t.title, `${q}/title`, 3000, { required: true });
        if (!PLAN_STATUSES.has(t.status)) {
          c.at(`${q}/status`, "must be one of [pending, in_progress, complete, error]");
        }
        if (ids.has(t.task_id)) c.at(`${q}/task_id`, "must be unique within the plan");
        ids.add(t.task_id);
      }
      break;
    }
    case "input":
      text(c, b.label, `${p}/label`, { plain: true, max: 2000 });
      text(c, b.hint, `${p}/hint`, { plain: true, max: 2000, required: false });
      if (b.element === undefined) c.at(`${p}/element`, "must provide an object");
      else element(c, b.element, `${p}/element`);
      break;
    case "image":
      image(c, b, p);
      break;
    case "rich_text":
      if (!Array.isArray(b.elements)) c.at(`${p}/elements`, "must provide an array");
      break;
    default:
      break;
  }
}

/**
 * What Slack refuses of `blocks` on a `surface` (`message`, `modal` or
 * `home`), each problem with its JSON pointer under `base`.
 */
export function blockProblems(blocks, surface = "message", base = "/blocks") {
  const c = checker(base);
  if (!Array.isArray(blocks)) {
    c.at("", "must provide an array");
    return c.problems;
  }
  const max = surface === "message" ? 50 : 100;
  if (blocks.length > max) c.at("", `must provide no more than ${max} items`);
  const totals = { markdown: 0 };
  const ids = new Set();
  blocks.forEach((b, i) => {
    block(c, b, `/${i}`, surface, totals);
    if (isObject(b) && typeof b.block_id === "string" && b.type !== "markdown") {
      if (ids.has(b.block_id)) c.at(`/${i}/block_id`, "must be unique");
      ids.add(b.block_id);
    }
  });
  if (totals.markdown > 12_000) {
    c.at("", "the markdown blocks of a message hold no more than 12000 characters in all");
  }
  return c.problems;
}

/** What Slack refuses of a view: a `modal` (views.open) or the Home tab (views.publish). */
export function viewProblems(view, base = "/view") {
  const c = checker(base);
  if (!isObject(view)) {
    c.at("", "must provide an object");
    return c.problems;
  }
  if (view.type !== "modal" && view.type !== "home") {
    c.at("/type", "must be one of [modal, home]");
    return c.problems;
  }
  if (view.type === "modal") {
    text(c, view.title, "/title", { plain: true, max: 24 });
    text(c, view.submit, "/submit", { plain: true, max: 24, required: false });
    text(c, view.close, "/close", { plain: true, max: 24, required: false });
    const inputs = Array.isArray(view.blocks) && view.blocks.some((b) => isObject(b) && b.type === "input");
    if (inputs && view.submit === undefined) c.at("/submit", "must provide submit with input blocks");
  }
  string(c, view.private_metadata, "/private_metadata", 3000);
  string(c, view.callback_id, "/callback_id", 255);
  string(c, view.external_id, "/external_id", 255);
  if (view.blocks === undefined) c.at("/blocks", "must provide an array");
  else c.problems.push(...blockProblems(view.blocks, view.type, `${base}/blocks`));
  return c.problems;
}

/** Slack's words for problems: `["[ERROR] must be … [json-pointer:/blocks/0/text/text]"]`. */
export const messages = (problems) => problems.map((p) => `[ERROR] ${p.message} [json-pointer:${p.pointer}]`);

// ───────────────────────────── what Slack fills in ─────────────────────────────

const ID_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
const randomId = () => [...randomBytes(4)].map((b) => ID_CHARS[b % ID_CHARS.length]).join("");

/** The blocks as Slack keeps them: a copy, with ids where the app gave none. */
export function withIds(blocks) {
  if (!Array.isArray(blocks)) return blocks;
  return structuredClone(blocks).map((b) => {
    if (!isObject(b)) return b;
    if (b.type !== "markdown" && b.block_id === undefined) b.block_id = randomId();
    for (const e of elementsOf(b)) {
      if (isObject(e) && e.type !== "image" && e.action_id === undefined) e.action_id = randomId();
    }
    return b;
  });
}

function elementsOf(b) {
  if (b.type === "actions" && Array.isArray(b.elements)) return b.elements;
  if (b.type === "section" && b.accessory) return [b.accessory];
  if (b.type === "input" && b.element) return [b.element];
  return [];
}

/**
 * The elements people use in `blocks`, in order: each with its block's id
 * (as Slack filled it in), for pressing them.
 */
export function controls(blocks) {
  const out = [];
  for (const b of Array.isArray(blocks) ? blocks : []) {
    if (!isObject(b)) continue;
    for (const e of elementsOf(b)) {
      if (isObject(e) && e.type !== "image") out.push({ block_id: b.block_id, element: e, block: b });
    }
  }
  return out;
}

/** What a stateful element holds, as `state.values` gives it: its initial value, or `chosen`. */
export function stateOf(e, chosen) {
  switch (e.type) {
    case "checkboxes":
    case "multi_static_select":
      return { type: e.type, selected_options: chosen ?? e.initial_options ?? [] };
    case "static_select":
    case "radio_buttons":
      return { type: e.type, selected_option: chosen ?? e.initial_option ?? null };
    case "plain_text_input":
    case "email_text_input":
    case "url_text_input":
    case "number_input":
      return { type: e.type, value: chosen ?? e.initial_value ?? null };
    case "conversations_select":
      return { type: e.type, selected_conversation: chosen ?? e.initial_conversation ?? null };
    case "channels_select":
      return { type: e.type, selected_channel: chosen ?? e.initial_channel ?? null };
    case "users_select":
      return { type: e.type, selected_user: chosen ?? e.initial_user ?? null };
    case "datepicker":
      return { type: e.type, selected_date: chosen ?? e.initial_date ?? null };
    default:
      return null;
  }
}

/** The `state.values` of blocks, with `chosen` values by block id and action id. */
export function stateValues(blocks, chosen = {}) {
  const values = {};
  for (const { block_id, element } of controls(blocks)) {
    const s = stateOf(element, chosen[block_id]?.[element.action_id]);
    if (!s) continue;
    values[block_id] ??= {};
    values[block_id][element.action_id] = s;
  }
  return values;
}

// ───────────────────────────── as people see them ─────────────────────────────

const plain = (t) => (isObject(t) && typeof t.text === "string" ? t.text : "");

function richWords(elements) {
  return (elements ?? [])
    .map((e) => {
      if (Array.isArray(e?.elements)) return richWords(e.elements);
      if (e?.type === "text") return e.text;
      if (e?.type === "user") return `<@${e.user_id}>`;
      if (e?.type === "channel") return `<#${e.channel_id}>`;
      if (e?.type === "link") return e.text ?? e.url;
      if (e?.type === "emoji") return `:${e.name}:`;
      return "";
    })
    .join("");
}

function controlShown(e) {
  switch (e.type) {
    case "button":
      return `[${plain(e.text)}]`;
    case "static_select":
      return `[${plain(e.placeholder) || "Choose"} ▾ ${(e.options ?? []).map((o) => plain(o.text)).join(" | ")}]`;
    case "checkboxes":
      return (e.options ?? [])
        .map((o) => `[${(e.initial_options ?? []).some((x) => same(x, o)) ? "x" : " "}] ${plain(o.text)}`)
        .join(" ");
    case "plain_text_input":
      return `[${e.initial_value ?? plain(e.placeholder) ?? ""}]`;
    default:
      return `[${e.type}]`;
  }
}

/** Blocks as plain lines, roughly as Slack shows them: their words, and buttons in brackets. */
export function shown(blocks) {
  const lines = [];
  for (const b of Array.isArray(blocks) ? blocks : []) {
    if (!isObject(b)) continue;
    switch (b.type) {
      case "section": {
        const parts = [plain(b.text), ...(b.fields ?? []).map(plain)].filter(Boolean);
        if (b.accessory) parts.push(controlShown(b.accessory));
        lines.push(parts.join(" "));
        break;
      }
      case "context":
        lines.push(
          (b.elements ?? []).map((e) => plain(e) || (e?.alt_text ? `(${e.alt_text})` : "")).join(" · "),
        );
        break;
      case "actions":
        lines.push((b.elements ?? []).map(controlShown).join(" "));
        break;
      case "header":
        lines.push(plain(b.text));
        break;
      case "markdown":
        lines.push(b.text ?? "");
        break;
      case "divider":
        lines.push("---");
        break;
      case "plan": {
        const mark = { complete: "x", in_progress: ">", error: "!", pending: " " };
        lines.push(b.title ?? "");
        for (const t of b.tasks ?? []) lines.push(`[${mark[t.status] ?? " "}] ${t.title}`);
        break;
      }
      case "input":
        lines.push(`${plain(b.label)}: ${b.element ? controlShown(b.element) : ""}`);
        break;
      case "rich_text":
        lines.push(richWords(b.elements));
        break;
      default:
        lines.push(`(${b.type})`);
    }
  }
  return lines.filter((l) => l !== "").join("\n");
}

/** The `rich_text` block Slack's clients send with a person's message, from its text. */
export function richText(text) {
  const elements = [];
  let last = 0;
  for (const m of String(text).matchAll(/<([@#!])([^>|]+)(?:\|([^>]*))?>/g)) {
    if (m.index > last) elements.push({ type: "text", text: text.slice(last, m.index) });
    if (m[1] === "@") elements.push({ type: "user", user_id: m[2] });
    else if (m[1] === "#") elements.push({ type: "channel", channel_id: m[2] });
    else elements.push({ type: "broadcast", range: m[2] });
    last = m.index + m[0].length;
  }
  if (last < text.length) elements.push({ type: "text", text: text.slice(last) });
  return [{ type: "rich_text", block_id: randomId(), elements: [{ type: "rich_text_section", elements }] }];
}
