// A Stop carries the customer's name, address and door code, none of which
// may reach a log or Sentry: log stop.toLogSafe() instead.
const loggers = new Set(["log", "console", "Sentry"]);

module.exports = {
  meta: {
    type: "problem",
    messages: {
      unsafe: "Don't log a Stop: it carries the door code. Log {{name}}.toLogSafe() instead",
    },
    schema: [],
  },
  create(context) {
    return {
      CallExpression(node) {
        const callee = node.callee;
        if (callee.type !== "MemberExpression" || !loggers.has(callee.object.name)) return;
        for (const arg of node.arguments) {
          const values = arg.type === "ObjectExpression" ? arg.properties.map((p) => p.value) : [arg];
          for (const value of values) {
            if (value?.type === "Identifier" && /^stop$|Stop$/.test(value.name)) {
              context.report({ node: value, messageId: "unsafe", data: { name: value.name } });
            }
          }
        }
      },
    };
  },
};
