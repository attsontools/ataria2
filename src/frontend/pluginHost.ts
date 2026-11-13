import type { DownloadEventPayload, DownloadHost, ThemeName } from './host';

const APP_ID = 'com.attson.aria2';
const REQUIRED_CAPABILITY = 'host.download.tasks';

interface PluginHandshake {
  theme: ThemeName;
  capabilities: string[];
}

interface PluginConnection {
  handshake(): Promise<PluginHandshake>;
  call(method: string, params?: unknown): Promise<unknown>;
  onThemeChange(listener: (theme: ThemeName) => void): () => void;
  onNotification(method: string, listener: (params: unknown) => void): () => void;
  close(): void;
}

type ConnectPlugin = (appId: string) => Promise<PluginConnection>;

export function createPluginHost(connect: ConnectPlugin): DownloadHost {
  let connection: PluginConnection | null = null;
  let removeThemeListener: (() => void) | null = null;
  let removeEventListener: (() => void) | null = null;
  const listeners = new Set<(event: DownloadEventPayload) => void>();

  return {
    mode: 'plugin',
    async initialize(onTheme) {
      removeThemeListener?.();
      removeEventListener?.();
      connection?.close();
      removeThemeListener = null;
      removeEventListener = null;
      connection = null;
      connection = await connect(APP_ID);
      const handshake = await connection.handshake();
      if (!handshake.capabilities.includes(REQUIRED_CAPABILITY)) {
        connection.close();
        connection = null;
        throw new Error(`HOST_PERMISSION_DENIED: ${REQUIRED_CAPABILITY}`);
      }
      removeThemeListener = connection.onThemeChange(onTheme);
      removeEventListener = connection.onNotification('host.download.taskEvent', (value) => {
        if (!isDownloadEvent(value)) return;
        for (const listener of listeners) listener(value);
      });
      onTheme(handshake.theme);
      return handshake.theme;
    },
    async request<T>(method: string, params: unknown = {}): Promise<T> {
      if (!connection) throw new Error('DOWNLOAD_SERVICE_NOT_CONNECTED');
      return await connection.call('host.download.tasks', { method, params }) as T;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async pickDirectory(defaultPath) {
      if (!connection) throw new Error('DOWNLOAD_SERVICE_NOT_CONNECTED');
      const result = await connection.call('host.download.pickDirectory', { defaultPath });
      if (typeof result !== 'object' || result === null) {
        throw new Error('HOST_DIRECTORY_PICK_INVALID');
      }
      const selection = result as { cancelled?: unknown; path?: unknown };
      if (selection.cancelled === true) return null;
      if (typeof selection.path !== 'string' || !selection.path) {
        throw new Error('HOST_DIRECTORY_PICK_INVALID');
      }
      return selection.path;
    },
    dispose() {
      removeThemeListener?.();
      removeEventListener?.();
      removeThemeListener = null;
      removeEventListener = null;
      listeners.clear();
      connection?.close();
      connection = null;
    }
  };
}

function isDownloadEvent(value: unknown): value is DownloadEventPayload {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const event = value as Partial<DownloadEventPayload>;
  return typeof event.id === 'string'
    && ['queued', 'running', 'completed', 'failed', 'cancelled'].includes(String(event.status))
    && typeof event.progress === 'number'
    && Number.isFinite(event.progress);
}
