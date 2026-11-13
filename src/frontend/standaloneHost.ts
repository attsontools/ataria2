import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

import type { DownloadEventPayload, DownloadHost, ThemeName } from './host';

const STORAGE_KEY = 'ataria2.theme';

export interface StandaloneDependencies {
  invoke(command: string, args?: unknown): Promise<unknown>;
  listen<T>(event: string, listener: (event: { payload: T }) => void): Promise<() => void>;
  storage: Pick<Storage, 'getItem'>;
  pickDirectory(defaultPath?: string): Promise<string | null>;
}

export function createStandaloneHost(
  dependencies: StandaloneDependencies = browserDependencies()
): DownloadHost {
  const listeners = new Set<(event: DownloadEventPayload) => void>();
  let removeEventListener: (() => void) | null = null;
  let disposed = false;

  return {
    mode: 'standalone',
    async initialize(onTheme) {
      await dependencies.invoke('connect_download_service');
      const theme: ThemeName = dependencies.storage.getItem(STORAGE_KEY) === 'light' ? 'light' : 'dark';
      onTheme(theme);
      return theme;
    },
    async request<T>(method: string, params: unknown = {}): Promise<T> {
      return await dependencies.invoke('download_service_request', { method, params }) as T;
    },
    subscribe(listener) {
      listeners.add(listener);
      if (!removeEventListener) {
        void dependencies.listen<DownloadEventPayload>('download-service-event', (event) => {
          if (!disposed) {
            for (const active of listeners) active(event.payload);
          }
        }).then((remove) => {
          if (disposed) remove();
          else removeEventListener = remove;
        });
      }
      return () => listeners.delete(listener);
    },
    pickDirectory: dependencies.pickDirectory,
    dispose() {
      disposed = true;
      listeners.clear();
      removeEventListener?.();
      removeEventListener = null;
    }
  };
}

function browserDependencies(): StandaloneDependencies {
  return {
    invoke,
    listen,
    storage: localStorage,
    async pickDirectory(defaultPath) {
      const path = await open({ directory: true, multiple: false, defaultPath });
      return typeof path === 'string' ? path : null;
    }
  };
}
