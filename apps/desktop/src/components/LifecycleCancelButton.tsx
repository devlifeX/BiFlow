import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AppButton } from "./AppButton";

export function LifecycleCancelButton({
  icon,
  onClick,
}: {
  icon: ReactNode;
  onClick: () => void;
}) {
  const { t } = useTranslation();

  return (
    <AppButton
      icon={icon}
      onClick={onClick}
      className="lifecycle-cancel-action h-14 shrink-0 whitespace-nowrap rounded-2xl border border-ink/15 bg-surface px-5 text-sm font-semibold sm:text-base"
    >
      {t("cancel")}
    </AppButton>
  );
}
