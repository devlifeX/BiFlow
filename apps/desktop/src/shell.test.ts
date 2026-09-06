import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { installContextMenuGuard } from "./installContextMenuGuard";

const css = readFileSync(path.join(process.cwd(), "src/index.css"), "utf8");
const guard = readFileSync(
  path.join(process.cwd(), "src/installContextMenuGuard.ts"),
  "utf8",
);
const main = readFileSync(path.join(process.cwd(), "src/main.tsx"), "utf8");

describe("fixed desktop shell", () => {
  it("locks html, body, and #root to the viewport without outer scroll", () => {
    expect(css).toMatch(/html,\s*body,\s*#root[\s\S]*overflow:\s*hidden/);
    expect(css).toMatch(/height:\s*100%/);
  });

  it("disables selection on chrome while keeping inputs and diagnostics selectable", () => {
    expect(css).toMatch(/user-select:\s*none/);
    expect(css).toMatch(/input,[\s\S]*user-select:\s*text/);
    expect(css).toMatch(/\.diagnostics-selectable/);
  });

  it("blocks the native context menu before React renders", () => {
    expect(guard).toMatch(/addEventListener\(\s*["']contextmenu["']/);
    expect(guard).toMatch(/preventDefault\(\)/);
    expect(main).toMatch(/installContextMenuGuard\(\)/);
    expect(main.indexOf("installContextMenuGuard();")).toBeLessThan(
      main.indexOf('createRoot(document.getElementById("root")'),
    );
  });

  it("highlights an available Connect button with a flat brand ring", () => {
    expect(css).toMatch(/\.connect-button-glow[\s\S]*box-shadow:/);
  });

  it("fills connection progress inside the action button", () => {
    expect(css).toMatch(
      /\.connection-action-fill-clip[\s\S]*overflow:\s*hidden/,
    );
    expect(css).toMatch(/\.connection-action-fill[\s\S]*transition:\s*width/);
    expect(css).toMatch(
      /prefers-reduced-motion: reduce[\s\S]*\.connection-action-fill[\s\S]*transition:\s*none/,
    );
  });

  it("draws a square 3px connection glow that sits on the window edges", () => {
    expect(css).toMatch(
      /\.connection-glow::after[\s\S]*border-radius:\s*0[\s\S]*border:\s*3px solid/,
    );
    expect(css).toMatch(/inset 0 0 18px/);
    expect(css).not.toMatch(
      /\.connection-glow::after[\s\S]*border-radius:\s*0\.5rem/,
    );
  });

  it("prevents contextmenu events at runtime", () => {
    installContextMenuGuard();
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves text and number inputs free for the custom context menu", () => {
    installContextMenuGuard();
    const input = document.createElement("input");
    input.type = "text";
    document.body.append(input);
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    input.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    input.remove();
  });
});
