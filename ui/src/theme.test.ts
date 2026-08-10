/**
 * Contrast, enforced rather than eyeballed.
 *
 * The palettes were originally chosen by eye, and several pairs quietly missed
 * WCAG AA: light hint text at 3.4:1, dark hint text at 3.58:1, the light primary
 * button's own label at 3.68:1, and control borders at 1.6:1 and 2.15:1 against
 * the 3:1 a component boundary needs. None of them looked broken enough to file
 * a bug against, which is the point — an opinion cannot see a 3.4 that should
 * be a 4.5, and a measurement can.
 *
 * So the palette is parsed out of `theme.css` and every foreground/background
 * pair the UI actually uses is checked against WCAG AA. CSS stays the single
 * source of truth; this only reads it.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));
const css = fs.readFileSync(path.join(here, "theme.css"), "utf8");

/** Custom properties from one CSS block, by its selector. */
function paletteAfter(marker: string): Record<string, string> {
  const start = css.indexOf(marker);
  assert.notEqual(start, -1, `theme.css no longer contains ${marker}`);
  const block = css.slice(start, css.indexOf("}", start));
  const palette: Record<string, string> = {};
  for (const [, name, value] of block.matchAll(
    /(--[\w-]+):\s*(#[0-9a-fA-F]{6});/g,
  )) {
    palette[name!] = value!;
  }
  return palette;
}

/** Relative luminance, per WCAG 2.1. */
function luminance(hex: string): number {
  const channel = (pair: string) => {
    const srgb = parseInt(pair, 16) / 255;
    return srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };
  const r = channel(hex.slice(1, 3));
  const g = channel(hex.slice(3, 5));
  const b = channel(hex.slice(5, 7));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
}

const dark = paletteAfter(":root {");
const light = paletteAfter(':root[data-theme="light"] {');

/**
 * Every pair the UI genuinely renders, with the minimum it must clear.
 *
 * 4.5:1 is AA for body text; 3:1 is AA for large text and for the boundary of a
 * user-interface component, which is what borders are.
 */
const PAIRS: Array<{ fg: string; bg: string; min: number; what: string }> = [
  { fg: "--text", bg: "--bg", min: 4.5, what: "body text on the page" },
  { fg: "--text", bg: "--bg-raised", min: 4.5, what: "body text on a panel" },
  { fg: "--text", bg: "--bg-sunken", min: 4.5, what: "input text" },
  {
    fg: "--text-dim",
    bg: "--bg-raised",
    min: 4.5,
    what: "section headings and field labels",
  },
  { fg: "--text-faint", bg: "--bg-raised", min: 4.5, what: "hint text" },
  {
    fg: "--text-faint",
    bg: "--bg-sunken",
    min: 4.5,
    what: "the version detail in a provider pill",
  },
  {
    fg: "--accent",
    bg: "--bg-raised",
    min: 3,
    what: "the running-status colour",
  },
  { fg: "--on-accent", bg: "--accent", min: 4.5, what: "the primary button" },
  {
    fg: "--critical",
    bg: "--bg-raised",
    min: 4.5,
    what: "a critical severity label",
  },
  { fg: "--high", bg: "--bg-raised", min: 4.5, what: "a high severity badge" },
  {
    fg: "--medium",
    bg: "--bg-raised",
    min: 4.5,
    what: "a medium severity badge",
  },
  { fg: "--low", bg: "--bg-raised", min: 4.5, what: "a low severity badge" },
  { fg: "--good", bg: "--bg-raised", min: 4.5, what: "an available provider" },
  {
    fg: "--warn",
    bg: "--bg-raised",
    min: 4.5,
    what: "the uncovered-lane warning",
  },
  { fg: "--border-strong", bg: "--bg-raised", min: 3, what: "control borders" },
];

for (const [name, palette] of [
  ["dark", dark],
  ["light", light],
] as const) {
  test(`${name} theme meets WCAG AA everywhere the UI uses it`, () => {
    for (const { fg, bg, min, what } of PAIRS) {
      const foreground = palette[fg];
      const background = palette[bg];
      assert.ok(foreground, `${name} theme has no ${fg}`);
      assert.ok(background, `${name} theme has no ${bg}`);
      const ratio = contrast(foreground!, background!);
      assert.ok(
        ratio >= min,
        `${name}: ${what} (${fg} on ${bg}) is ${ratio.toFixed(2)}:1, below the ${min}:1 minimum`,
      );
    }
  });
}

test("both themes define the same set of colours", () => {
  // A property present in one theme and missing from the other renders as
  // whatever the dark default was, which is how a light theme ends up with one
  // stray dark value nobody notices.
  const darkKeys = Object.keys(dark).sort();
  const lightKeys = Object.keys(light).sort();
  // Compared directly, in both directions. A one-way difference passed a dark
  // palette that had lost a property the light one still defines, leaving the
  // dark hover rules referencing an undefined custom property.
  assert.deepEqual(
    darkKeys,
    lightKeys,
    "the dark and light themes define different sets of colours",
  );
});

test("the two themes are actually different", () => {
  assert.notEqual(dark["--bg"], light["--bg"]);
  assert.notEqual(dark["--text"], light["--text"]);
});
