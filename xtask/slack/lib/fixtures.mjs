// What fctl posts and opens in Slack, shaped as fctl_domain::slack::blocks
// builds it (its card, a question, the choice of a project, the forms and
// the Home tab), for the stand-in's tests.

export const mrkdwn = (text) => ({ type: "mrkdwn", text, verbatim: true });
export const plain = (text) => ({ type: "plain_text", text, emoji: true });
export const section = (text) => ({ type: "section", text: mrkdwn(text) });
export const context = (text) => ({ type: "context", elements: [mrkdwn(text)] });
export const button = (label, action_id, more = {}) => ({
  type: "button",
  text: plain(label),
  action_id,
  ...more,
});
export const option = (label, value) => ({ text: plain(label), value });

export const CARD = [
  section("*<https://fctl.acme.dev/s/ses_1|Fix the flaky login test>*\n:gear: Running the tests"),
  context("Web · web#12 · for Ada Lovelace"),
  {
    type: "plan",
    title: "1 of 3 done",
    tasks: [
      { task_id: "t0", title: "Reproduce it", status: "complete" },
      { task_id: "t1", title: "Fix the race", status: "in_progress" },
      { task_id: "t2", title: "Add a test", status: "pending" },
    ],
  },
  section(
    ":arrow_heading_up: <https://github.com/acme/web/pull/42|acme/web#42> Fix the login race · :x: 1 check failed",
  ),
  {
    type: "actions",
    elements: [button("Open in fctl", "fctl_open", { url: "https://fctl.acme.dev/s/ses_1" })],
  },
];

export const QUESTION = [
  section(":raising_hand: *The agent asks*\nShip it to production?"),
  {
    type: "actions",
    block_id: "fctl_q:ses_1:1700",
    elements: [
      button("Yes", "fctl_answer_0", { value: "0" }),
      button("Not yet", "fctl_answer_1", { value: "1" }),
    ],
  },
  context("Click an answer, or reply in this thread."),
];

export const CHOOSE = [
  section(":thinking_face: Which project should work on this?"),
  {
    type: "actions",
    block_id: "fctl_remember",
    elements: [
      {
        type: "checkboxes",
        action_id: "fctl_remember",
        options: [{ text: mrkdwn("Remember for #web-team"), value: "remember" }],
      },
    ],
  },
  {
    type: "actions",
    block_id: "fctl_choose",
    elements: [
      button("Playground", "fctl_project_0", { value: "T0ACME:C0WEB:1.2|prj_1" }),
      button("Web", "fctl_project_1", { value: "T0ACME:C0WEB:1.2|prj_2" }),
      button("Cancel", "fctl_cancel", { value: "T0ACME:C0WEB:1.2" }),
    ],
  },
  context("Only you see this."),
];

export const START_FORM = {
  type: "modal",
  callback_id: "fctl_start",
  title: plain("Start a session"),
  submit: plain("Start"),
  close: plain("Cancel"),
  private_metadata: '{"channel":null}',
  blocks: [
    {
      type: "input",
      block_id: "project",
      label: plain("Project"),
      element: {
        type: "static_select",
        action_id: "project",
        placeholder: plain("Choose a project"),
        options: [option("Playground", "prj_1"), option("Web", "prj_2")],
        initial_option: option("Playground", "prj_1"),
      },
    },
    {
      type: "input",
      block_id: "task",
      label: plain("Task"),
      element: {
        type: "plain_text_input",
        action_id: "task",
        multiline: true,
        placeholder: plain("What should the agent do?"),
      },
      hint: plain("It runs as you, on your Claude subscription. Its card comes to your messages with fctl."),
    },
  ],
};

export const UPDATES = { text: plain("Post its card and replies in this thread"), value: "updates" };
export const SEND_FORM = {
  ...START_FORM,
  callback_id: "fctl_send",
  title: plain("Send to fctl"),
  blocks: [
    context("Message from Grace Hopper"),
    section("> The deploy broke"),
    ...START_FORM.blocks,
    {
      type: "input",
      block_id: "updates",
      optional: true,
      label: plain("Updates"),
      element: { type: "checkboxes", action_id: "updates", options: [UPDATES], initial_options: [UPDATES] },
    },
  ],
};

export const HOME = {
  type: "home",
  blocks: [
    { type: "header", text: plain("fctl") },
    section("Hi Dev. Mention fctl in a channel, or write to it in Messages."),
    {
      type: "actions",
      elements: [
        button("Start a session", "fctl_start", { style: "primary" }),
        button("Open fctl", "fctl_open", { url: "http://localhost:8787/" }),
      ],
    },
    { type: "divider" },
    { type: "header", text: plain("Needs you") },
    context("Nothing waits for you."),
  ],
};
