import { describe, expect, it } from "vitest";
import type { AppConfig } from "../api/models";
import {
  canAddPreset,
  createClientInstance,
  isDefaultRouteValid,
  profileFileName,
  sanitizeDefaultRoute,
} from "./clients";

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

describe("primary route switching", () => {
  const hiddifyId = "11111111-1111-1111-1111-111111111111";
  const happId = "22222222-2222-2222-2222-222222222222";
  const hiddify = createClientInstance("hiddify", hiddifyId);
  const happ = createClientInstance("happ", happId);

  it("accepts any enabled client as primary, not only Hiddify", () => {
    const clients = [hiddify, happ];
    expect(
      isDefaultRouteValid({ kind: "client", client_id: happId }, clients),
    ).toBe(true);
    expect(
      isDefaultRouteValid({ kind: "client", client_id: hiddifyId }, clients),
    ).toBe(true);
    expect(isDefaultRouteValid({ kind: "direct" }, [])).toBe(true);
  });

  it("rejects a primary that was removed or disabled", () => {
    // Hiddify removed entirely, Happ promoted.
    expect(
      isDefaultRouteValid({ kind: "client", client_id: hiddifyId }, [happ]),
    ).toBe(false);
    // Happ still present but disabled.
    expect(
      isDefaultRouteValid({ kind: "client", client_id: happId }, [
        { ...happ, enabled: false },
      ]),
    ).toBe(false);
  });

  it("falls back to direct when the primary disappears", () => {
    const config = {
      clients: [happ],
      default_route: { kind: "client", client_id: hiddifyId },
    } as AppConfig;
    expect(sanitizeDefaultRoute(config).default_route).toEqual({
      kind: "direct",
    });
    // A valid Happ primary is left untouched.
    const valid = {
      clients: [happ],
      default_route: { kind: "client", client_id: happId },
    } as AppConfig;
    expect(sanitizeDefaultRoute(valid)).toBe(valid);
  });
});

describe("canAddPreset", () => {
  it("allows a second and third client but never a duplicate preset", () => {
    const hiddify = createClientInstance("hiddify");
    const happ = createClientInstance("happ");
    expect(canAddPreset("happ", [hiddify])).toBe(true);
    expect(canAddPreset("openvpn", [hiddify, happ])).toBe(true);
    expect(canAddPreset("happ", [hiddify, happ])).toBe(false);
    expect(canAddPreset("hiddify", [hiddify])).toBe(false);
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
