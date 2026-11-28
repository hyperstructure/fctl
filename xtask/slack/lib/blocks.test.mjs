import assert from "node:assert/strict";
import { test } from "node:test";
import * as B from "./blocks.mjs";
import {
  button,
  CARD,
  CHOOSE,
  HOME,
  mrkdwn,
  option,
  plain,
  QUESTION,
  SEND_FORM,
  START_FORM,
  section,
  UPDATES,
} from "./fixtures.mjs";

const problems = (blocks, surface) => B.messages(B.blockProblems(blocks, surface));

test("takes what fctl posts and opens", () => {
  for (const blocks of [CARD, QUESTION, CHOOSE, [{ type: "markdown", text: "Done: see **#42**." }]]) {
    assert.deepEqual(problems(blocks), []);
  }
  for (const view of [START_FORM, SEND_FORM, HOME]) assert.deepEqual(B.messages(B.viewProblems(view)), []);
});

test("refuses what Slack refuses, in its words and JSON pointers", () => {
  const long = button("x".repeat(76), "a");
  assert.deepEqual(problems([{ type: "actions", elements: [long] }]), [
    "[ERROR] must be less than 76 characters [json-pointer:/blocks/0/elements/0/text/text]",
  ]);
  assert.deepEqual(problems([section("x".repeat(3001))]), [
    "[ERROR] must be less than 3001 characters [json-pointer:/blocks/0/text/text]",
  ]);
  assert.deepEqual(problems([{ type: "header", text: mrkdwn("Hi") }]), [
    "[ERROR] must be plain_text [json-pointer:/blocks/0/text/type]",
  ]);
  assert.deepEqual(problems([{ type: "carrot" }]), [
    '[ERROR] "carrot" isn\'t a block Slack has [json-pointer:/blocks/0/type]',
  ]);
  assert.deepEqual(problems([{ type: "markdown", text: "hi" }], "modal"), [
    "[ERROR] markdown blocks are for messages only [json-pointer:/blocks/0/type]",
  ]);
  assert.deepEqual(problems([{ ...CARD[2], tasks: [{ task_id: "t0", title: "A", status: "done" }] }]), [
    "[ERROR] must be one of [pending, in_progress, complete, error] [json-pointer:/blocks/0/tasks/0/status]",
  ]);
  assert.deepEqual(problems([{ type: "actions", elements: [button("A", "same"), button("B", "same")] }]), [
    "[ERROR] must be unique within the block [json-pointer:/blocks/0/elements/1/action_id]",
  ]);
  assert.deepEqual(problems(Array.from({ length: 51 }, () => ({ type: "divider" }))), [
    "[ERROR] must provide no more than 50 items [json-pointer:/blocks]",
  ]);
  assert.deepEqual(
    problems([
      { type: "markdown", text: "x".repeat(7000) },
      { type: "markdown", text: "x".repeat(7000) },
    ]),
    [
      "[ERROR] the markdown blocks of a message hold no more than 12000 characters in all [json-pointer:/blocks]",
    ],
  );
  assert.deepEqual(
    problems([section("a"), { ...section("b"), block_id: "x" }, { ...section("c"), block_id: "x" }]),
    ["[ERROR] must be unique [json-pointer:/blocks/2/block_id]"],
  );
  const view = (more) => B.messages(B.viewProblems({ ...START_FORM, ...more }));
  assert.deepEqual(view({ title: plain("Start a session right now") }), [
    "[ERROR] must be less than 25 characters [json-pointer:/view/title/text]",
  ]);
  assert.deepEqual(view({ submit: undefined }), [
    "[ERROR] must provide submit with input blocks [json-pointer:/view/submit]",
  ]);
  const select = { ...START_FORM.blocks[0].element, initial_option: option("Elsewhere", "prj_9") };
  assert.deepEqual(view({ blocks: [{ ...START_FORM.blocks[0], element: select }] }), [
    "[ERROR] must match one of the options [json-pointer:/view/blocks/0/element/initial_option]",
  ]);
  assert.deepEqual(
    B.messages(B.viewProblems({ type: "home", blocks: [{ type: "plan", title: "x", tasks: [] }] })),
    ["[ERROR] plan blocks are for messages only [json-pointer:/view/blocks/0/type]"],
  );
});

test("fills in the ids Slack gives what has none, and finds what people press", () => {
  const kept = B.withIds(CHOOSE);
  assert.equal(kept[1].block_id, "fctl_remember");
  assert.match(kept[0].block_id, /^[\w+/=]{4}$/);
  assert.equal(CHOOSE[0].block_id, undefined, "the app's own blocks stay as they were");
  assert.equal(B.withIds([{ type: "markdown", text: "x" }])[0].block_id, undefined);
  const controls = B.controls(kept);
  assert.deepEqual(
    controls.map((c) => [c.block_id, c.element.action_id]),
    [
      ["fctl_remember", "fctl_remember"],
      ["fctl_choose", "fctl_project_0"],
      ["fctl_choose", "fctl_project_1"],
      ["fctl_choose", "fctl_cancel"],
    ],
  );
  // What the message's inputs hold: the box, ticked or not.
  assert.deepEqual(B.stateValues(kept), {
    fctl_remember: { fctl_remember: { type: "checkboxes", selected_options: [] } },
  });
  const ticked = B.stateValues(kept, { fctl_remember: { fctl_remember: kept[1].elements[0].options } });
  assert.equal(ticked.fctl_remember.fctl_remember.selected_options[0].value, "remember");
  const form = B.stateValues(SEND_FORM.blocks, { task: { task: "Fix it" } });
  assert.deepEqual(form.project.project.selected_option.value, "prj_1");
  assert.deepEqual(form.task.task, { type: "plain_text_input", value: "Fix it" });
  assert.deepEqual(form.updates.updates.selected_options, [UPDATES]);
});

test("shows blocks as people read them", () => {
  const card = B.shown(CARD);
  assert.match(
    card,
    /^\*<https:\/\/fctl\.acme\.dev\/s\/ses_1\|Fix the flaky login test>\*\n:gear: Running the tests/,
  );
  assert.match(card, /\n1 of 3 done\n\[x\] Reproduce it\n\[>\] Fix the race\n\[ \] Add a test\n/);
  assert.match(card, /\[Open in fctl\]$/);
  assert.match(B.shown(CHOOSE), /\[ \] Remember for #web-team\n\[Playground\] \[Web\] \[Cancel\]/);
  assert.equal(
    B.shown(B.richText("<@U0FCTLBOT> look at <#C0INC|incidents>")),
    "<@U0FCTLBOT> look at <#C0INC>",
  );
  assert.deepEqual(B.richText("hi <@U0GRACE>!")[0].elements[0].elements, [
    { type: "text", text: "hi " },
    { type: "user", user_id: "U0GRACE" },
    { type: "text", text: "!" },
  ]);
});
