import solid from "eslint-plugin-solid/configs/typescript";
import globals from "globals";
import astro from "eslint-plugin-astro";
import tseslint from "typescript-eslint";
import path from "node:path";
import { fileURLToPath } from "node:url";
import js from "@eslint/js";
import { FlatCompat } from "@eslint/eslintrc";
import gitignore from "eslint-config-flat-gitignore";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const compat = new FlatCompat({
  baseDirectory: __dirname,
  recommendedConfig: js.configs.recommended,
  allConfig: js.configs.all,
});

export default [
  gitignore({
    files: [".gitignore", "apps/hhai.dev/.gitignore"],
  }),
  {
    ignores: [".prettierrc.js"],
  },
  js.configs.recommended,
  ...compat.extends("plugin:prettier/recommended"),
  ...tseslint.configs.recommended,
  ...astro.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    ignores: ["**/*React.{ts,tsx}"],
    ...solid,

    languageOptions: {
      parser: tseslint.parser,
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
];
