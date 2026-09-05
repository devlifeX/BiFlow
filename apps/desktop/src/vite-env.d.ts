/// <reference types="vite/client" />

declare module "*.png" {
  const src: string;
  export default src;
}

declare global {
  const __APP_VERSION__: string;
  const __MOCK_HIDDIFY_INSTALLED__: boolean;
  const __MOCK_MIHOMO_INSTALLED__: boolean;
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __BIFLOW_RESET_MOCK?: () => void;
    __BIFLOW_NEXT_PROFILE_PATH__?: string | null;
    __BIFLOW_STAGE_SEEN?: string[];
    __BIFLOW_STAGE_STOP?: () => void;
  }
}

export {};
