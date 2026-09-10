import type { HighlighterCore, LanguageInput, ThemeRegistration } from "shiki/core";

/**
 * Client-side syntax highlighting (web/README.md, DECISIONS.md #11/#19) —
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

/** Canonical Shiki ids (`LANG_LOADERS`' keys) accepted as-is, so a fence
 *  written ```rust or ```typescript works the same as an alias below. */
const CANONICAL_LANG_IDS = new Set(Object.keys(LANG_LOADERS));

/** Maps a markdown fence's info string (e.g. `rust`, `ts`, `js title=x`) to
 *  a Shiki language id, or `undefined` if unsupported — reuses the same
 *  extension-alias table `languageForPath` draws from, so `README.md` fences
 *  and repository file extensions recognize the same set of languages.
 *  Exported for testing. */
export function languageForFence(info: string): string | undefined {
  const token = info.trim().split(/\s+/, 1)[0]?.toLowerCase();
  if (!token) return undefined;
  if (CANONICAL_LANG_IDS.has(token)) return token;
  return LANG_BY_EXTENSION[token];
}

/** The themes an operator can pick from, via `AXGIT_SYNTAX_THEME_LIGHT` /
 *  `AXGIT_SYNTAX_THEME_DARK` (docs/DECISIONS.md #95). A curated subset of
 *  Shiki's 66 bundled themes, for the same reason `LANG_LOADERS` curates
 *  languages: every entry here becomes its own chunk in `web/dist`, which
 *  the default build bakes into the binary. The light/dark grouping below
 *  is only how the themes were designed — either mode accepts any id.
 *
 *  Each specifier must stay a *literal*: a template literal would make Vite
 *  bundle all 66. Adding one means adding it to the table in `README.md`
 *  and the `[syntax]` section of `axgit.toml` too. */
const THEME_LOADERS = {
  "github-light": () => import("shiki/themes/github-light.mjs"),
  "github-light-default": () => import("shiki/themes/github-light-default.mjs"),
  "one-light": () => import("shiki/themes/one-light.mjs"),
  "catppuccin-latte": () => import("shiki/themes/catppuccin-latte.mjs"),
  "solarized-light": () => import("shiki/themes/solarized-light.mjs"),
  "vitesse-light": () => import("shiki/themes/vitesse-light.mjs"),
  "min-light": () => import("shiki/themes/min-light.mjs"),
  "github-dark": () => import("shiki/themes/github-dark.mjs"),
  "github-dark-dimmed": () => import("shiki/themes/github-dark-dimmed.mjs"),
  "one-dark-pro": () => import("shiki/themes/one-dark-pro.mjs"),
  nord: () => import("shiki/themes/nord.mjs"),
  dracula: () => import("shiki/themes/dracula.mjs"),
  "catppuccin-mocha": () => import("shiki/themes/catppuccin-mocha.mjs"),
  "solarized-dark": () => import("shiki/themes/solarized-dark.mjs"),
  "vitesse-dark": () => import("shiki/themes/vitesse-dark.mjs"),
  "tokyo-night": () => import("shiki/themes/tokyo-night.mjs"),
} satisfies Record<string, () => Promise<{ default: ThemeRegistration }>>;

type ThemeId = keyof typeof THEME_LOADERS;

/** Used when the deployment configures nothing, and when what it configures
 *  can't be loaded. */
const DEFAULT_THEMES: Record<ColorMode, ThemeId> = {
  light: "github-light",
  dark: "github-dark",
};

type ColorMode = "light" | "dark";

function isThemeId(id: string): id is ThemeId {
  return id in THEME_LOADERS;
}

/** The theme id configured for `mode`, read from the `<meta>` `shell.rs`
 *  injects into every served shell (`api/src/shell.rs::site_head_meta`).
 *  Read from the document rather than fetched, so it is already in hand by
 *  the time the highlighter is built — and unset in `astro dev`, which
 *  serves the shells itself and injects nothing.
 *
 *  An id with no loader falls back rather than failing: the API deliberately
 *  passes the value through without validating it, since the list of ids
 *  lives here (docs/DECISIONS.md #95), so this warning is the only place a
 *  typo surfaces. */
function configuredTheme(mode: ColorMode): ThemeId {
  const fallback = DEFAULT_THEMES[mode];
  const configured = document.querySelector<HTMLMetaElement>(`meta[name="axgit:syntax-theme-${mode}"]`)?.content.trim();

  if (!configured) return fallback;
  if (isThemeId(configured)) return configured;

  console.warn(
    `[axgit] unknown ${mode} syntax theme "${configured}", falling back to "${fallback}". ` +
      `Available: ${Object.keys(THEME_LOADERS).join(", ")}`,
  );
  return fallback;
}

/** The core plus the ids its two themes actually registered under, which
 *  `codeToTokens` needs by name on every call. */
interface Highlighter {
  core: HighlighterCore;
  themes: Record<ColorMode, string>;
}

let highlighterPromise: Promise<Highlighter> | undefined;

/** The one highlighter instance is shared across every `highlightCode`
 *  call — languages are loaded into it incrementally as needed, never
 *  re-created per file. The configured themes are read once here, not per
 *  call: they are deployment-wide, and the module outlives an
 *  `astro:after-swap` (only `<body>` is swapped). */
function getHighlighter(): Promise<Highlighter> {
  highlighterPromise ??= (async () => {
    const lightId = configuredTheme("light");
    const darkId = configuredTheme("dark");
    const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, light, dark] = await Promise.all([
      import("shiki/core"),
      import("shiki/engine/javascript"),
      THEME_LOADERS[lightId]().then((m) => m.default),
      THEME_LOADERS[darkId]().then((m) => m.default),
    ]);
    const core = await createHighlighterCore({
      themes: [light, dark],
      langs: [],
      engine: createJavaScriptRegexEngine({ forgiving: true }),
    });
    // `createHighlighterCore` registers each theme under its own `name`,
    // which is what `codeToTokens` then has to be given. Every bundled theme
    // carries one and it matches its module's basename, but the type makes
    // it optional, so the id we asked for stands in.
    return {
      core,
      themes: { light: light.name ?? lightId, dark: dark.name ?? darkId },
    };
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

/** Tokenizes `code` in the given Shiki language id. Returns `null` — meaning
 *  "render as plain text" — when `lang` is `undefined`/unsupported, or the
 *  content is above the size/line threshold. Shared by `highlightCode`
 *  (path-derived language) and `highlightFence` (markdown fence info
 *  string). */
async function tokenize(code: string, lang: string | undefined): Promise<HighlightedLine[] | null> {
  if (code.length > SIZE_THRESHOLD_BYTES) return null;
  if (!lang) return null;

  const loader = LANG_LOADERS[lang];
  if (!loader) return null;

  // Cheap upper bound before paying for tokenization.
  let lines = 1;
  for (let i = 0; i < code.length; i++) {
    if (code.charCodeAt(i) === 10 /* \n */) lines++;
    if (lines > LINE_THRESHOLD) return null;
  }

  const { core, themes } = await getHighlighter();
  if (!core.getLoadedLanguages().includes(lang)) {
    const { default: grammars } = await loader();
    await core.loadLanguage(...grammars);
  }

  const { tokens } = core.codeToTokens(code, { lang, themes });

  return tokens.map((line) => line.map((token) => ({ content: token.content, style: token.htmlStyle ?? {} })));
}

/** Tokenizes `code` for syntax highlighting, keyed off `path`'s extension.
 *  Returns `null` — meaning "render as plain text" — when the language
 *  isn't supported, or the file is above the size/line threshold. */
export function highlightCode(code: string, path: string): Promise<HighlightedLine[] | null> {
  return tokenize(code, languageForPath(path));
}

/** Tokenizes `code` for a markdown fence, keyed off its info string (e.g.
 *  ` ```rust `). Same fallback rules as `highlightCode`. */
export function highlightFence(code: string, info: string): Promise<HighlightedLine[] | null> {
  return tokenize(code, languageForFence(info));
}
