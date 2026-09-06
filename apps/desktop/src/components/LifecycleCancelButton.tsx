import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AppButton } from "./AppButton";
import { CONNECTION_BUTTON_HEIGHT_CLASS } from "./ConnectionActionButton";

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
      className={`lifecycle-cancel-action ${CONNECTION_BUTTON_HEIGHT_CLASS} shrink-0 whitespace-nowrap rounded-[5px] border border-[rgb(var(--border-default))] bg-surface px-2 text-[11px] font-semibold leading-none`}
    >
      {t("cancel")}
    </AppButton>
  );
}
