import { describe, expect, it } from "vitest";
import i18n from "../i18n/config";
import type { AppError } from "../api/models";
import { failureReason } from "./failureReason";

const platformError: AppError = {
  code: "INTERNAL",
  message_key: "errors.platform",
  retryable: true,
  remediation: "run_diagnostics",
  technical_details:
    "platform operation failed: Mihomo exited immediately (exit code: 1): wintun.dll was not found",
  correlation_id: "00000000-0000-0000-0000-000000000001",
};

describe("failureReason", () => {
  it("includes the engine cause instead of a bare error label", () => {
    const text = failureReason(platformError, i18n.t.bind(i18n));
    expect(text).toContain("A connection step failed.");
    expect(text).toContain("wintun.dll was not found");
    expect(text).not.toBe("error");
  });
});
