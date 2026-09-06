import { describe, expect, it } from "vitest";
import { isGoogleHost, isHiddenProductionHost } from "./hiddenHosts";

describe("hiddenHosts", () => {
  it("matches google.com and its subdomains", () => {
    expect(isGoogleHost("google.com")).toBe(true);
    expect(isGoogleHost("www.google.com")).toBe(true);
    expect(isGoogleHost("gemini.google.com.")).toBe(true);
    expect(isGoogleHost("notgoogle.com")).toBe(false);
    expect(isGoogleHost("facebook.com")).toBe(false);
  });

  it("hides google hosts only in a production build", () => {
    expect(isHiddenProductionHost("google.com", true)).toBe(true);
    expect(isHiddenProductionHost("www.google.com", true)).toBe(true);
    expect(isHiddenProductionHost("google.com", false)).toBe(false);
    expect(isHiddenProductionHost("iran.ir", true)).toBe(false);
  });
});
