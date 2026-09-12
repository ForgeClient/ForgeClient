// Reports user-facing text that has not been routed through the i18n catalogue.
//
// Localisation is being rolled out feature by feature (see ui/src/i18n/). This
// script is how that rollout stays measurable: it prints every literal string
// that still looks like copy, grouped by file, plus a total.
//
// It is a heuristic, deliberately biased towards over-reporting. A false
// positive costs one glance; a missed string ships an untranslatable client.
// Four hiding places matter most and are all covered here, because an earlier
// version that only looked at JSX text nodes under-reported by roughly half:
//
//   {copied ? "Link copied" : "Copy live link"}     ternaries
//   case "VICTORY": return "Victory";               switch returns
//   `${count} players`                              template literals
//   Filters{count > 0 ? ` (${count})` : ""}         text beside an expression
//
// The last two were found by hand rather than by this script, which is what
// added them: a dozen strings were sitting in template literals and in JSX
// text that happened to touch a brace, and the report said three.
//
// Usage:
//   node scripts/i18n-scan.mjs                 whole ui/src, counts per file
//   node scripts/i18n-scan.mjs ui/src/features/maps --list    with the strings
//   node scripts/i18n-scan.mjs --max 0         exit non-zero above a budget

import { readdir, readFile } from "node:fs/promises";
import { extname, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const verbose = args.includes("--list");
const maxIndex = args.indexOf("--max");
const budget = maxIndex === -1 ? null : Number(args[maxIndex + 1]);
const target = args.find((arg) => !arg.startsWith("--") && arg !== String(budget)) ?? "ui/src";

// Files whose capitalised strings are data, not copy. Each carries its reason:
// an unexplained ignore list is how real strings get lost.
const IGNORED_FILES = new Map([
  ["ui/src/store/reducer.ts", "AppEvent kind discriminants, matched against the wire"],
  ["ui/src/store/store.ts", "string-concatenation fragments, not standalone copy"],
  ["ui/src/shared/mapPresentation.ts", "official map names: proper nouns, never translated"],
  ["ui/src/design-system/Icon.tsx", "inline SVG path data"],
  ["ui/src/shared/externalLinks.ts", "developer-facing throw messages, never rendered"],
  ["ui/src/shared/factions.ts", "faction data keyed by wire id; the shown label is factions.random"],
  ["ui/src/features/links/LinksView.tsx", "the names in the thanks list; what each is thanked for is a key"],
  ["ui/src/features/player-card/PlayerOverview.tsx", "the three clients' product names, matched against the user agent"],
  ["ui/src/features/training/RunMap.tsx", "marker type ids read out of the game's own map files"],
  ["ui/src/features/training/recording.ts", "the same marker type ids, on the analyser side"],
  ["ui/src/features/events/eventSubmission.ts", "the body of a GitHub issue, which is English wherever it is written from"],
  ["ui/src/features/maps/MapPreviewZoom.tsx", "throw messages for a copy that falls back on its own; never rendered"],
]);

// Attribute names whose values are machine tokens, never prose.
const TECHNICAL_ATTRS =
  /\b(?:className|key|id|htmlFor|name|type|role|value|href|src|rel|target|autoComplete|inputMode|allow|data-[\w-]+|aria-(?:hidden|current|expanded|haspopup|controls|live|valuetext|labelledby|describedby|selected|checked|disabled|sort))\s*=\s*"[^"]*"/g;

// Object keys carrying machine tokens in this codebase's command shapes.
const COMMAND_KEYS =
  /\b(?:kind|type|command|payload|leaderboard|sortBy|field|constraint|faction|outcome|status|mode|tab|channel|queueName|folderName|technicalName)\s*:\s*"[^"]*"/g;

// KeyboardEvent.key values: compared against, never displayed, and sentence
// cased, so without this list they dominate the report.
const KEYBOARD_KEYS = new Set([
  "Enter", "Escape", "Backspace", "Delete", "Tab", "Home", "End", "PageUp", "PageDown",
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Shift", "Control", "Alt", "Meta",
  // The bare prefix, for the `startsWith("Arrow")` that handles all four.
  "Arrow",
]);

// Template literals that build a CSS value rather than a sentence. A style
// string is the one kind of template whose fixed half reads like prose to the
// rule above, because it is words separated by spaces.
const CSS_VALUE =
  /\b(?:px|fr|vh|vw|rem|em|deg|repeat|minmax|translate[XY]?|rotate|scale|calc|url|color-mix|srgb|linear-gradient|no-repeat|solid|contain|cover|center)\b|%[,)]|^\d/;

// TypeScript builtins that appear as bare words in type positions.
const TYPE_NAMES = new Set([
  "Promise", "Record", "Partial", "Readonly", "Array", "Map", "Set", "ReactNode", "JSX",
]);

const NOT_PROSE = [
  /^[a-z][a-z0-9]*(?:[-_][a-z0-9]+)*$/,          // kebab/snake/lower identifiers
  /^[A-Z][A-Z0-9_]*$/,                            // SCREAMING_CASE
  /^[\w.-]+\.(?:tsx?|css|json|lua|exe|jar|png|svg)$/i,
  /^#[0-9a-f]{3,8}$/i,                            // colours
  /^\d/,                                          // starts with a digit
  /^[^a-zA-Z]*$/,                                 // no letters at all
  // Proper nouns: a generator, a league division and a release kind, all of
  // them matched against or printed verbatim rather than translated.
  /^(?:faf|coop|nomads|fafbeta|fafdevelop|ladder1v1|global|en|de|UEF|Aeon|Cybran|Seraphim|Neroxis|Grandmaster|Hotfix)$/,
  /^[a-z][\w-]*(?:\s+[a-z][\w-]*)+$/,             // a CSS class list
  /(?:\|\||&&|===|!==|=>|\)\.)/,                  // half of a split expression
  /^[A-Z][a-z]+(?:[A-Z][a-z]+)+$/,                // PascalCase type or slice name
  /^[,;:.]/,                                      // half of a concatenation
  /^\)/,                                          // starts mid-expression
  /[<>{}]/,                                       // contains markup or a brace
];

function isProse(value) {
  const text = value.trim();
  if (text.length < 3) return false;
  if (KEYBOARD_KEYS.has(text)) return false;
  if (TYPE_NAMES.has(text)) return false;
  if (NOT_PROSE.some((rule) => rule.test(text))) return false;
  if (!/[a-z]/.test(text)) return false;
  // Either a sentence-cased word or several words: both read as copy.
  return /^[A-Z][a-z]/.test(text) || /\s[a-z]/.test(text);
}

async function sourceFiles(directory) {
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (["node_modules", "dist", "i18n"].includes(entry.name)) continue;
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) found.push(...(await sourceFiles(path)));
    else if ([".ts", ".tsx"].includes(extname(entry.name)) && !entry.name.includes(".test."))
      found.push(path);
  }
  return found;
}

let total = 0;
const perFile = [];

for (const path of (await sourceFiles(resolve(root, target))).sort()) {
  const relativePath = relative(root, path).split("\\").join("/");
  if (IGNORED_FILES.has(relativePath)) continue;

  const source = (await readFile(path, "utf8"))
    .replace(/^\s*import[^;]+;$/gm, "")
    .replace(/^\s*\/\/.*$/gm, "")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\bt\(\s*"[^"]*"/g, "t(")
    .replace(/\bMessageKey\b[^;]*;/g, "")
    .replace(TECHNICAL_ATTRS, "")
    .replace(COMMAND_KEYS, "");

  const hits = new Set();
  for (const [, value] of source.matchAll(/"([^"\\\n]{3,})"/g)) if (isProse(value)) hits.add(value);
  for (const [, value] of source.matchAll(/'([^'\\\n]{3,})'/g)) if (isProse(value)) hits.add(value);
  // JSX text nodes are not string literals, so they need their own pass.
  for (const [, value] of source.matchAll(/>\s*([A-Z][A-Za-z0-9 ,.'\u2019!?()/&%:-]{2,})\s*</g)) {
    if (isProse(value)) hits.add(value);
  }
  // JSX text that touches an expression container on either side. Without
  // this, `Filters{count > 0 ? ... : ""}` and `{count} slots` both read as
  // fragments of an expression rather than as the copy they are.
  //
  // Both sides must touch real markup, and the text may hold no bracket,
  // colon or equals sign: that is what keeps ordinary TypeScript between two
  // braces from being read as a sentence.
  const JSX_WORDS = "[A-Za-z0-9 ,.'’!?/&%-]";
  const JSX_BESIDE_EXPRESSION = [
    new RegExp(`>\\s*([A-Z]${JSX_WORDS}{2,}?)\\s*\\{`, "g"),
    new RegExp(`\\}\\s*([A-Za-z]${JSX_WORDS}{2,}?)\\s*<`, "g"),
  ];
  for (const pattern of JSX_BESIDE_EXPRESSION) {
    for (const [, value] of source.matchAll(pattern)) if (isProse(value)) hits.add(value.trim());
  }
  // Template literals whose fixed halves are prose. A class name is the
  // common false positive and is excluded by `isProse`, which refuses a
  // lower-case identifier list; what is left is copy with a number in it.
  for (const [, value] of source.matchAll(/`([^`\\\n]{3,})`/g)) {
    const fixed = value.replace(/\$\{[^}]*\}/g, " ").trim();
    if (CSS_VALUE.test(fixed)) continue;
    if (fixed.split(/\s+/).filter(Boolean).length >= 2 && isProse(fixed)) hits.add(`\`${value}\``);
  }

  if (hits.size === 0) continue;
  total += hits.size;
  perFile.push([relativePath, [...hits]]);
}

perFile.sort((left, right) => right[1].length - left[1].length);
for (const [file, hits] of perFile) {
  console.log(`${String(hits.length).padStart(4)}  ${file}`);
  if (verbose) for (const hit of hits) console.log(`        ${hit}`);
}

console.log(`\nUntranslated strings: ${total}`);

if (budget !== null && total > budget) {
  console.error(`\nBudget exceeded: ${total} > ${budget}`);
  process.exit(1);
}
