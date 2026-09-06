import { describe, expect, it } from "vitest";
import {
  downloadLinksFor,
  downloadUrlFor,
  presetById,
  runtimeBinarySpec,
} from "./presets";

describe("preset downloads", () => {
  it("offers OpenVPN plus the Windscribe config generator", () => {
    const links = downloadLinksFor(presetById("windscribe"));
    expect(links.map((link) => link.labelKey)).toEqual([
      "downloadOpenVpn",
      "downloadWindscribeConfig",
    ]);
    expect(downloadUrlFor(links[0]!.spec, "linux")).toBe(
      "https://openvpn.net/community-downloads/",
    );
    expect(downloadUrlFor(links[1]!.spec, "linux")).toBe(
      "https://windscribe.com/getconfig/openvpn",
    );
  });

  it("treats OpenVPN as the runtime binary for Windscribe", () => {
    expect(runtimeBinarySpec("windscribe")?.id).toBe("openvpn");
    expect(runtimeBinarySpec("openvpn")?.id).toBe("openvpn");
    expect(runtimeBinarySpec("happ")).toBeNull();
  });
});
