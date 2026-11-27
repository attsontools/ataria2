import { describe, expect, it, vi } from 'vitest';

import { createPluginHost } from '../src/frontend/pluginHost';

describe('plugin download host', () => {
  it('routes tasks and events through Host API 2 download methods', async () => {
    const call = vi.fn(async <T,>(method: string): Promise<T> => (method === 'host.handshake'
      ? undefined
      : { id: 'dl-1' }) as T);
    const notifications = new Map<string, (value: unknown) => void>();
    const host = createPluginHost(async () => ({
      handshake: async () => ({
        apiVersion: '2.0',
        sessionId: 'session-1',
        correlationId: 'correlation-1',
        capabilities: ['host.theme.read', 'host.download.tasks'],
        metadata: { appId: 'com.attson.aria2', name: 'Aria2 下载', version: '0.1.0-alpha.0' },
        theme: 'dark'
      }),
      call,
      onThemeChange: () => () => undefined,
      onNotification: (method, listener) => {
        notifications.set(method, listener);
        return () => undefined;
      },
      close: vi.fn()
    }));
    await host.initialize(() => undefined);
    const received: unknown[] = [];
    host.subscribe((event) => received.push(event));

    const result = await host.request<{ id: string }>('download.tasks.start', {
      request: { url: 'https://example.com/a.zip', downloadDir: '/tmp' }
    });
    notifications.get('host.download.taskEvent')?.({ id: 'dl-1', status: 'running', progress: 25 });

    expect(result).toEqual({ id: 'dl-1' });
    expect(call).toHaveBeenCalledWith('host.download.tasks', {
      method: 'download.tasks.start',
      params: { request: { url: 'https://example.com/a.zip', downloadDir: '/tmp' } }
    });
    expect(received).toEqual([{ id: 'dl-1', status: 'running', progress: 25 }]);
  });

  it('closes the previous connection before retrying initialization', async () => {
    const closes = [vi.fn(), vi.fn()];
    const removeThemes = [vi.fn(), vi.fn()];
    const removeEvents = [vi.fn(), vi.fn()];
    let index = 0;
    const host = createPluginHost(async () => {
      const current = index++;
      return {
        handshake: async () => ({ theme: 'dark' as const, capabilities: ['host.download.tasks'] }),
        call: async () => ({}),
        onThemeChange: () => removeThemes[current],
        onNotification: () => removeEvents[current],
        close: closes[current]
      };
    });

    await host.initialize(() => undefined);
    await host.initialize(() => undefined);

    expect(removeThemes[0]).toHaveBeenCalledOnce();
    expect(removeEvents[0]).toHaveBeenCalledTimes(2);
    expect(closes[0]).toHaveBeenCalledOnce();
    expect(closes[1]).not.toHaveBeenCalled();
  });

  it('delivers Host app activation notifications to subscribers', async () => {
    const notifications = new Map<string, (value: unknown) => void>();
    const host = createPluginHost(async () => ({
      handshake: async () => ({ theme: 'dark' as const, capabilities: ['host.download.tasks'] }),
      call: async () => ({}),
      onThemeChange: () => () => undefined,
      onNotification: (method, listener) => {
        notifications.set(method, listener);
        return () => notifications.delete(method);
      },
      close: vi.fn()
    }));
    const received: unknown[] = [];
    host.subscribeActivation((activation) => received.push(activation));

    await host.initialize(() => undefined);
    notifications.get('host.app.activated')?.({
      activationId: 'activation-1',
      sourceAppId: 'com.attson.video',
      payload: { version: 1, kind: 'download.urls', urls: ['https://example.com/video.mp4'] }
    });

    expect(received).toEqual([{
      activationId: 'activation-1',
      sourceAppId: 'com.attson.video',
      payload: { version: 1, kind: 'download.urls', urls: ['https://example.com/video.mp4'] }
    }]);
  });
});
