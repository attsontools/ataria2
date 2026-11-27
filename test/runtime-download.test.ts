// @ts-nocheck
import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';

import { downloadVerified } from '../scripts/runtime-download.mjs';

describe('pinned runtime downloads', () => {
  it('uses the origin first and falls back to the proxy with the same hash check', async () => {
    const bytes = Buffer.from('verified aria2 source');
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    const requests: Array<{ url: string; signal: AbortSignal }> = [];
    const fetchImpl = vi.fn(async (url, options) => {
      requests.push({ url, signal: options.signal });
      if (requests.length === 1) throw new Error('origin unavailable');
      return new Response(bytes, { status: 200 });
    });

    await expect(downloadVerified(
      'https://github.com/aria2/aria2/releases/download/release-1.37.0/source.tar.gz',
      sha256,
      { fetchImpl, timeoutMs: 5000 }
    )).resolves.toEqual(bytes);
    expect(requests.map((request) => request.url)).toEqual([
      'https://github.com/aria2/aria2/releases/download/release-1.37.0/source.tar.gz',
      'https://gh-proxy.com/https://github.com/aria2/aria2/releases/download/release-1.37.0/source.tar.gz'
    ]);
    expect(requests.every((request) => request.signal instanceof AbortSignal)).toBe(true);
  });

  it('rejects content that does not match the fixed hash from either source', async () => {
    const fetchImpl = vi.fn(async () => new Response('tampered', { status: 200 }));

    await expect(downloadVerified('https://github.com/example/archive', 'a'.repeat(64), {
      fetchImpl,
      timeoutMs: 5000
    })).rejects.toThrow(/checksum mismatch/);
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });
});
