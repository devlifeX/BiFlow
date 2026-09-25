import type { TFunction } from "i18next";
import type { AppError } from "../api/models";

/** User-facing sentence plus the engine's specific cause, when it adds detail. */
export function failureReason(error: AppError, translate: TFunction): string {
  const summary = translate(error.message_key);
  const detail = error.technical_details?.trim();
  if (!detail || detail === summary) return summary;
  return `${summary} ${detail}`;
}
