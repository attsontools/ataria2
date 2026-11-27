import { parseDownloadUrls } from './downloadState';

export type VideoActivationResult = {
  kind: 'accepted';
  input: string;
  added: number;
} | {
  kind: 'rejected';
  input: string;
  code: string;
  message: string;
} | {
  kind: 'duplicate';
  input: string;
};

export class VideoActivationInbox {
  private readonly seen = new Set<string>();
  private readonly order: string[] = [];

  constructor(private readonly maxSeen = 128) {}

  accept(currentInput: string, value: unknown): VideoActivationResult {
    const notification = asRecord(value);
    if (notification?.sourceAppId !== 'com.attson.video') {
      return {
        kind: 'rejected',
        input: currentInput,
        code: 'VIDEO_ACTIVATION_SOURCE_DENIED',
        message: '仅接受来自视频链接抽取应用的下载链接。'
      };
    }
    const urls = parsePayload(notification.payload);
    if (!urls) return invalidPayload(currentInput);
    const activationId = notification.activationId;
    if (typeof activationId !== 'string' || activationId.length === 0 || activationId.length > 255) {
      return invalidPayload(currentInput);
    }
    if (this.seen.has(activationId)) return { kind: 'duplicate', input: currentInput };
    const existing = parseDownloadUrls(currentInput);
    const seen = new Set(existing.map(urlKey));
    const added = urls.filter((url) => {
      const key = urlKey(url);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    this.remember(activationId);
    return {
      kind: 'accepted',
      input: [...existing, ...added].join('\n'),
      added: added.length
    };
  }

  private remember(activationId: string) {
    this.seen.add(activationId);
    this.order.push(activationId);
    while (this.order.length > this.maxSeen) {
      const oldest = this.order.shift();
      if (oldest) this.seen.delete(oldest);
    }
  }
}

function parsePayload(value: unknown): string[] | null {
  const payload = asRecord(value);
  if (!payload) return null;
  if (Object.keys(payload).sort().join(',') !== 'kind,urls,version') return null;
  if (payload.version !== 1 || payload.kind !== 'download.urls') return null;
  if (!Array.isArray(payload.urls) || payload.urls.length < 1 || payload.urls.length > 100) {
    return null;
  }
  const urls: string[] = [];
  const seen = new Set<string>();
  for (const value of payload.urls) {
    if (typeof value !== 'string' || value.length === 0 || value !== value.trim() || value.length > 8192) {
      return null;
    }
    let parsed: URL;
    try {
      parsed = new URL(value);
    } catch {
      return null;
    }
    if (
      !['http:', 'https:'].includes(parsed.protocol)
      || parsed.username.length > 0
      || parsed.password.length > 0
    ) {
      return null;
    }
    const key = parsed.href;
    if (seen.has(key)) return null;
    seen.add(key);
    urls.push(value);
  }
  return urls;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function urlKey(value: string): string {
  try {
    return new URL(value).href;
  } catch {
    return value;
  }
}

function invalidPayload(input: string): VideoActivationResult {
  return {
    kind: 'rejected',
    input,
    code: 'VIDEO_ACTIVATION_INVALID_PAYLOAD',
    message: '视频应用发送的下载链接格式无效。'
  };
}
