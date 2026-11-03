import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Reads the real design tokens so a color change that breaks readability fails here.
const css = readFileSync(new URL("../style.css", import.meta.url), "utf8");

const tokens = {};
for (const match of css.matchAll(/--([\w-]+):\s*light-dark\((#[0-9a-f]{3,8}),\s*(#[0-9a-f]{3,8})\)/gi))
  tokens[match[1]] = { light: match[2], dark: match[3] };

const parse = (hex) => {
  const value = hex.slice(1);
  const full = value.length <= 4 ? [...value].map((c) => c + c).join("") : value;
  const n = (i) => parseInt(full.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: full.length === 8 ? n(6) / 255 : 1 };
};
/** Composites a possibly translucent color over an opaque one. */
const over = (top, bottom) => ({
  r: top.r * top.a + bottom.r * (1 - top.a),
  g: top.g * top.a + bottom.g * (1 - top.a),
  b: top.b * top.a + bottom.b * (1 - top.a),
  a: 1,
});
const luminance = ({ r, g, b }) => {
  const channel = (v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
};
const ratio = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** Resolves a token or literal for one theme; layers are composited bottom-up. */
const color = (spec, theme) => {
  const layers = spec.split(" over ").reverse();
  let result = null;
  for (const layer of layers) {
    const value = layer.startsWith("#") ? parse(layer) : parse(tokens[layer]?.[theme] ?? "#f0f");
    result = result ? over(value, result) : value;
  }
  return result;
};

// [text, background] pairs; "a over b" means a translucent layer drawn on b.
const pairs = [
  ["on-accent", "accent"],
  ["on-accent", "accent-hover"],
  ["disabled-text", "disabled-bg"],
  ["text", "control"],
  ["text", "control-hover"],
  ["danger", "control"],
  ["danger", "control-hover"],
  ["accent", "card"],
  ["accent", "subtle-hover over card"],
  ["disabled-text", "card"],
  ["muted", "card"],
  ["text-2", "layer"],
  ["text-2", "card"],
  ["accent", "accent-soft over card"],
];

describe("button and text contrast", () => {
  it("found the tokens it checks", () => {
    for (const name of ["accent", "on-accent", "danger", "danger-hover", "disabled-text", "disabled-bg"])
      expect(tokens[name], name).toBeTruthy();
  });

  for (const theme of ["light", "dark"])
    for (const [text, background] of pairs)
      it(`${text} on ${background} is readable in ${theme}`, () => {
        expect(ratio(color(text, theme), color(background, theme))).toBeGreaterThanOrEqual(4.5);
      });

  it("filled danger buttons are readable at rest and on hover", () => {
    const text = { light: "#ffffff", dark: "#000000" };
    for (const theme of ["light", "dark"])
      for (const background of ["danger", "danger-hover"])
        expect(ratio(parse(text[theme]), color(background, theme))).toBeGreaterThanOrEqual(4.5);
  });

  it("card header buttons cannot recolor filled buttons", () => {
    expect(css).not.toMatch(/^\.card-actions button\s*\{/m);
    expect(css).toMatch(/:where\(\.card-actions\) button:where\(/);
    expect(css).toMatch(/^button\.accent,\s*\nbutton\.danger-fill \{/m);
  });

  it("disabled states never fade with opacity", () => {
    for (const block of css.matchAll(/([^{}]*:disabled[^{}]*)\{([^}]*)\}/g))
      expect(block[2], block[1].trim()).not.toMatch(/opacity:\s*0/);
  });
});
