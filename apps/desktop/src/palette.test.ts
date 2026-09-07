import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(path.join(process.cwd(), "src/index.css"), "utf8");

function block(selector: string, nextSelector: string): string {
  const start = css.indexOf(selector);
  const end = css.indexOf(nextSelector, start);
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  return css.slice(start, end);
}

function rgbToken(source: string, name: string): [number, number, number] {
  const match = source.match(
    new RegExp(`--${name}:\\s*(\\d+)\\s+(\\d+)\\s+(\\d+);`),
  );
  expect(match).not.toBeNull();
  return [Number(match?.[1]), Number(match?.[2]), Number(match?.[3])];
}

function luminance(rgb: [number, number, number]): number {
  const linear = (channel: number) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return (
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
  );
}

function contrast(
  foreground: [number, number, number],
  background: [number, number, number],
): number {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

describe("BiFlow palette", () => {
  it("recolors the existing light and dark tokens without a second theme system", () => {
    const light = block(":root {", ".dark {");
    const dark = block(".dark {", "* {");

    expect(rgbToken(light, "brand")).toEqual([0, 104, 230]);
    expect(rgbToken(light, "accent")).toEqual([180, 83, 9]);
    expect(rgbToken(light, "success")).toEqual([0, 124, 131]);
    expect(rgbToken(dark, "canvas")).toEqual([18, 18, 18]);
    expect(rgbToken(dark, "success")).toEqual([0, 210, 209]);
  });

  it("keeps white button text readable on the Nero accent orange", () => {
    const light = block(":root {", ".dark {");
    expect(
      contrast(rgbToken(light, "accent"), [255, 255, 255]),
    ).toBeGreaterThanOrEqual(4.5);
  });
});
