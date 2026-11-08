import { describe, expect, it } from "vitest";
import { extractHost } from "./host";

describe("extractHost", () => {
  it("reduces a full URL to its host", () => {
    expect(extractHost("https://www.example.ir/")).toBe("www.example.ir");
    expect(extractHost("http://digikala.com/product/1?a=2#top")).toBe(
      "digikala.com",
    );
    expect(extractHost("  HTTPS://Example.IR/path  ")).toBe("example.ir");
  });

  it("keeps a bare host untouched", () => {
    expect(extractHost("digikala.com")).toBe("digikala.com");
    expect(extractHost("5.22.12.1")).toBe("5.22.12.1");
  });

  it("drops ports, credentials, and the FQDN root dot", () => {
    expect(extractHost("example.ir:8443")).toBe("example.ir");
    expect(extractHost("https://user:pass@example.ir:8443/x")).toBe(
      "example.ir",
    );
    expect(extractHost("example.ir.")).toBe("example.ir");
  });

  it("handles IPv6 with and without brackets", () => {
    expect(extractHost("[2001:db8::1]:443")).toBe("2001:db8::1");
    expect(extractHost("2001:db8::1")).toBe("2001:db8::1");
  });

  it("accepts protocol-relative and scheme-less input", () => {
    expect(extractHost("//cdn.example.ir/asset.js")).toBe("cdn.example.ir");
    expect(extractHost("example.ir/path")).toBe("example.ir");
  });

  it("returns an empty string for empty input", () => {
    expect(extractHost("   ")).toBe("");
  });
});
