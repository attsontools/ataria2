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
  let removeActivationListener: (() => void) | null = null;
  const listeners = new Set<(event: DownloadEventPayload) => void>();
  const activationListeners = new Set<(activation: unknown) => void>();

  return {
    mode: 'plugin',
    async initialize(onTheme) {
      removeThemeListener?.();
      removeEventListener?.();
      removeActivationListener?.();
      connection?.close();
      removeThemeListener = null;
      removeEventListener = null;
      removeActivationListener = null;
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
      removeActivationListener = connection.onNotification('host.app.activated', (value) => {
        for (const listener of activationListeners) listener(value);
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
    subscribeActivation(listener) {
      activationListeners.add(listener);
      return () => activationListeners.delete(listener);
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
      removeActivationListener?.();
      removeThemeListener = null;
      removeEventListener = null;
      removeActivationListener = null;
      listeners.clear();
      activationListeners.clear();
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
