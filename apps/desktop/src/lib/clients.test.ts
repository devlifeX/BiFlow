import { describe, expect, it } from "vitest";
import { createClientInstance, profileFileName } from "./clients";

describe("profileFileName", () => {
  it("returns the last path segment on Unix and Windows paths", () => {
    expect(profileFileName("/home/user/office.ovpn")).toBe("office.ovpn");
    expect(profileFileName("C:\\Users\\a\\windscribe.ovpn")).toBe(
      "windscribe.ovpn",
    );
    expect(profileFileName(null)).toBeNull();
    expect(profileFileName("")).toBeNull();
  });
});

describe("createClientInstance", () => {
  it("starts OpenVPN and Windscribe without a profile path", () => {
    expect(createClientInstance("openvpn").config).toMatchObject({
      kind: "owned_side_tunnel",
      profile_path: null,
    });
    expect(createClientInstance("windscribe").config).toMatchObject({
      kind: "owned_side_tunnel",
      profile_path: null,
    });
  });
});
