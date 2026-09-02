// ui-lint: enforces the banned patterns from docs/ui-spec.md §6.
// Zero dependencies. Run: bun apps/web/scripts/ui-lint.mjs
// Exits 1 when violations are found. Chained into the root `check` script.
import { readdirSync, statSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const WEB_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC_ROOT = join(WEB_ROOT, "src");
// shadcn-svelte generated primitives are vendor code — never edited, never linted.
const IGNORE = [join(SRC_ROOT, "lib", "components", "ui")];

const RULES = [
  {
    id: "raw-gray-scale",
    test: /(?:text|bg|border|divide|decoration)-(?:slate|zinc|gray|neutral)-\d+/,
    hint: "Use semantic tokens: text-foreground / text-muted-foreground(/65) (ui-spec.md §1.2)"
  },
  {
    id: "white-alpha-surface",
    test: /(?:text-white\b|(?:border|divide|bg)-white\/)/,
    hint: "Use border-edge / border-edge-strong / bg-ink-800/* tokens (ui-spec.md §1.3)"
  },
  {
    id: "arbitrary-size",
    test: /(?:text|gap|p[xytblr]?|m[xytblr]?|w|h)-\[\d+(?:px|rem)\]/,
    hint: "Arbitrary values are banned; pick from the token tables (ui-spec.md §1.1, §1.7)"
  },
  {
    id: "icon-size-off-scale",
    test: /size=\{(?:10|11|13|15|17|18|19|20|21|22|24)\}/,
    hint: "Icon size must be 12 | 14 | 16 (ui-spec.md §1.5)"
  },
  {
    id: "card-radius-xl",
    test: /(?:^|\s)rounded-(?:xl|2xl)\b/,
    hint: "Cards/panels use rounded-lg (10px); see radius table (ui-spec.md §1.4)"
  },
  {
    id: "truncated-badge",
    test: /\.status\.slice\(0,\s*3\)/,
    hint: "Never truncate status labels; use StatusBadge compact (ui-spec.md §3.1)"
  }
];

function walk(dir, files = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      if (IGNORE.some((path) => full === path || full.startsWith(path))) continue;
      walk(full, files);
    } else if (entry.endsWith(".svelte")) {
      files.push(full);
    }
  }
  return files;
}

let violations = 0;
for (const file of walk(SRC_ROOT)) {
  const rel = file.slice(WEB_ROOT.length + 1);
  const lines = readFileSync(file, "utf8").split(/\r?\n/);
  lines.forEach((line, index) => {
    for (const rule of RULES) {
      if (rule.test.test(line)) {
        violations += 1;
        console.error(`✗ ${rel}:${index + 1}  [${rule.id}]`);
        console.error(`  ${line.trim().slice(0, 120)}`);
        console.error(`  → ${rule.hint}`);
      }
    }
  });
}

if (violations > 0) {
  console.error(`\nui-lint: ${violations} violation(s) against docs/ui-spec.md`);
  process.exit(1);
}
console.log("ui-lint: 0 violations — tokens are consistent with docs/ui-spec.md");
