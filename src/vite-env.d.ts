/// <reference types="vite/client" />

interface Window {
  __TAURI__?: {
    window: {
      getCurrentWindow(): {
        close(): Promise<void>;
        minimize(): Promise<void>;
      };
    };
  };
}
