export type ThemeName = 'dark' | 'light';
export type DownloadStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface DownloadEventPayload {
  id: string;
  status: DownloadStatus;
  progress: number;
  speed?: string | null;
  eta?: string | null;
  message?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  localPath?: string | null;
}

export interface DownloadTask extends DownloadEventPayload {
  url: string;
  downloadDir: string;
  fileName?: string | null;
  createdAt: string;
}

export interface StartDownloadRequest {
  url: string;
  downloadDir: string;
  fileName?: string;
  connections: number;
  split: number;
  minSplitSize: string;
}

export interface DownloadHost {
  readonly mode: 'standalone' | 'plugin';
  initialize(onTheme: (theme: ThemeName) => void): Promise<ThemeName>;
  request<T>(method: string, params?: unknown): Promise<T>;
  subscribe(listener: (event: DownloadEventPayload) => void): () => void;
  subscribeActivation(listener: (activation: unknown) => void): () => void;
  pickDirectory(defaultPath?: string): Promise<string | null>;
  dispose(): void;
}
