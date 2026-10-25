// ESLint's flat config: Expo's rules, and our own from eslint-rules/.
const expo = require("eslint-config-expo/flat");
const local = require("./eslint-rules");

module.exports = [
  ...expo,
  { ignores: ["dist/*", "ios/*", "android/*"] },
  {
    plugins: { local },
    rules: {
      "local/no-unsafe-stop-log": "error",
    },
  },
];
