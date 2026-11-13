import { describe, expect, it, vi } from 'vitest';

import { createStandaloneHost } from '../src/frontend/standaloneHost';

describe('standalone download host', () => {
  it('uses the standalone service commands with the same domain methods', async () => {
    const invoke = vi.fn(async <T,>(command: string): Promise<T> => (command === 'download_service_request'
      ? { tasks: [] }
      : undefined) as T);
    let listener: ((event: { payload: unknown }) => void) | undefined;
    const host = createStandaloneHost({
      invoke,
      listen: vi.fn(async (_event, callback) => {
        listener = callback;
        return () => undefined;
      }),
      storage: { getItem: () => 'dark' },
      pickDirectory: async () => '/tmp/downloads'
    });
    const events: unknown[] = [];
    host.subscribe((event) => events.push(event));

    await host.initialize(() => undefined);
    const result = await host.request('download.tasks.list', {});
    listener?.({ payload: { id: 'dl-1', status: 'running', progress: 10 } });

    expect(result).toEqual({ tasks: [] });
    expect(invoke).toHaveBeenNthCalledWith(1, 'connect_download_service');
    expect(invoke).toHaveBeenNthCalledWith(2, 'download_service_request', {
      method: 'download.tasks.list', params: {}
    });
    expect(events).toEqual([{ id: 'dl-1', status: 'running', progress: 10 }]);
  });
});
