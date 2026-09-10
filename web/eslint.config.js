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
  // Every e2e spec has to go through `e2e/fixtures.ts` (docs/DECISIONS.md
  // #96): importing `test` straight from `@playwright/test` would skip the
  // guard that fails a test whose page requested an API endpoint it never
  // stubbed. Type-only imports are fine — they carry no runtime behaviour.
  {
    files: ["e2e/**/*.ts"],
    ignores: ["e2e/fixtures.ts"],
    rules: {
      "@typescript-eslint/no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@playwright/test",
              allowTypeImports: true,
              message: "Import `test`/`expect` from ./fixtures so the unstubbed-API guard applies.",
            },
          ],
        },
      ],
    },
  },
]);
