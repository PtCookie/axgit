// @ts-check
import { resolve } from "node:path";
import { defineConfig, includeIgnoreFile } from "eslint/config";
import js from "@eslint/js";
import globals from "globals";
import tsEslint from "typescript-eslint";
import pluginAstro from "eslint-plugin-astro";
import pluginReact from "@eslint-react/eslint-plugin";
import pluginVitest from "@vitest/eslint-plugin";

export default defineConfig([
  includeIgnoreFile(resolve(import.meta.dirname, ".gitignore")),
  // Generated from openapi.json by `pnpm gen:types`; never edited by hand.
  { ignores: ["src/lib/api/types.ts"] },
  {
    files: ["**/*.{js,mjs,cjs,ts,mts,cts,jsx,tsx}"],
    plugins: { js },
    extends: ["js/recommended"],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  tsEslint.configs.strict,
  tsEslint.configs.stylistic,
  pluginAstro.configs.recommended,
  { files: ["**/*.{jsx,tsx}"], ...pluginReact.configs.strict },
  { files: ["tests/**"], ...pluginVitest.configs.recommended },
]);
