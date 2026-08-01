import type { HighlighterCore, LanguageInput } from "shiki/core";

/**
 * Client-side syntax highlighting (ARCHITECTURE.md, DECISIONS.md #11/#19) —
 * Astro's built-in Shiki is build-time only and can't touch runtime-fetched
 * blob content. Uses `shiki/core` with the **JavaScript RegExp engine**
 * (`shiki/engine/javascript`, `forgiving: true`) rather than the default
 * Oniguruma/WASM engine, so no ~500 KiB `.wasm` asset needs to ship with the
 * build — at the cost of reduced grammar accuracy for a few complex
 * languages (DECISIONS.md #19).
 */

/** Above this size, or this many lines, highlighting is skipped entirely —
 *  the tokenizer's cost isn't worth it for huge files that `BlobView`
 *  barely lets the user scroll through anyway. */
const SIZE_THRESHOLD_BYTES = 512 * 1024;
const LINE_THRESHOLD = 5000;

/** Extension (lowercased, no dot) → the language id passed to Shiki, which
 *  is also the `shiki/langs/*` module basename below. Only languages with a
 *  loader entry are actually highlighted — everything else falls back to
 *  plain text. */
const LANG_BY_EXTENSION: Record<string, string> = {
  rs: "rust",
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  tsx: "tsx",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "jsx",
  py: "python",
  go: "go",
  java: "java",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  json: "json",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  md: "markdown",
  markdown: "markdown",
  html: "html",
  htm: "html",
  css: "css",
  sql: "sql",
  rb: "ruby",
  php: "php",
  xml: "xml",
};

/** Lazily imports one language's grammar module. Each module's default
 *  export is an array (the grammar plus any grammars it embeds, e.g. `tsx`
 *  pulling in `jsx`), so `loadLanguage` is always called with a spread. */
const LANG_LOADERS: Record<string, () => Promise<{ default: LanguageInput[] }>> = {
  rust: () => import("shiki/langs/rust.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  bash: () => import("shiki/langs/bash.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  php: () => import("shiki/langs/php.mjs"),
  docker: () => import("shiki/langs/docker.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
};

/** Maps a repository path to a Shiki language id, or `undefined` if it has
 *  no highlighting support. Exported for testing. */
export function languageForPath(path: string): string | undefined {
  const base = path.slice(path.lastIndexOf("/") + 1);
  if (/^dockerfile$/i.test(base)) return "docker";
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return undefined;
  return LANG_BY_EXTENSION[base.slice(dot + 1).toLowerCase()];
}

const LIGHT_THEME = "github-light";
const DARK_THEME = "github-dark";

let highlighterPromise: Promise<HighlighterCore> | undefined;

/** The one highlighter instance is shared across every `highlightCode`
 *  call — languages are loaded into it incrementally as needed, never
 *  re-created per file. */
function getHighlighter(): Promise<HighlighterCore> {
  highlighterPromise ??= (async () => {
    const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, light, dark] = await Promise.all([
      import("shiki/core"),
      import("shiki/engine/javascript"),
      import("shiki/themes/github-light.mjs").then((m) => m.default),
      import("shiki/themes/github-dark.mjs").then((m) => m.default),
    ]);
    return createHighlighterCore({
      themes: [light, dark],
      langs: [],
      engine: createJavaScriptRegexEngine({ forgiving: true }),
    });
  })();
  return highlighterPromise;
}

/** One highlighted token: its text and an inline style object — `color` for
 *  the light theme plus a `--shiki-dark` custom property, matching Shiki's
 *  documented dual-theme CSS-variables approach (`global.css` supplies the
 *  `.dark` override). Rendered directly as a React `style` prop, never
 *  through `dangerouslySetInnerHTML` — repository content is untrusted
 *  input (same rule `lib/format/linkify.tsx` follows). */
export interface HighlightedToken {
  content: string;
  style: Record<string, string>;
}

export type HighlightedLine = HighlightedToken[];

/** Tokenizes `code` for syntax highlighting, keyed off `path`'s extension.
 *  Returns `null` — meaning "render as plain text" — when the language
 *  isn't supported, or the file is above the size/line threshold. */
export async function highlightCode(code: string, path: string): Promise<HighlightedLine[] | null> {
  if (code.length > SIZE_THRESHOLD_BYTES) return null;

  const lang = languageForPath(path);
  if (!lang) return null;

  const loader = LANG_LOADERS[lang];
  if (!loader) return null;

  // Cheap upper bound before paying for tokenization.
  let lines = 1;
  for (let i = 0; i < code.length; i++) {
    if (code.charCodeAt(i) === 10 /* \n */) lines++;
    if (lines > LINE_THRESHOLD) return null;
  }

  const highlighter = await getHighlighter();
  if (!highlighter.getLoadedLanguages().includes(lang)) {
    const { default: grammars } = await loader();
    await highlighter.loadLanguage(...grammars);
  }

  const { tokens } = highlighter.codeToTokens(code, {
    lang,
    themes: { light: LIGHT_THEME, dark: DARK_THEME },
  });

  return tokens.map((line) => line.map((token) => ({ content: token.content, style: token.htmlStyle ?? {} })));
}
