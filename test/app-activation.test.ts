import { describe, expect, it } from 'vitest';

import { VideoActivationInbox } from '../src/frontend/appActivation';

describe('video download activation inbox', () => {
  it('appends new video URLs without replacing the existing download input', () => {
    const inbox = new VideoActivationInbox();

    expect(inbox.accept('https://example.com/existing.zip', {
      activationId: 'activation-1',
      sourceAppId: 'com.attson.video',
      payload: {
        version: 1,
        kind: 'download.urls',
        urls: [
          'https://example.com/video.mp4',
          'https://example.com/existing.zip'
        ]
      }
    })).toEqual({
      kind: 'accepted',
      input: 'https://example.com/existing.zip\nhttps://example.com/video.mp4',
      added: 1
    });
  });

  it('rejects an activation from any app other than AT Video', () => {
    const inbox = new VideoActivationInbox();

    expect(inbox.accept('https://example.com/existing.zip', {
      activationId: 'activation-2',
      sourceAppId: 'com.example.untrusted',
      payload: {
        version: 1,
        kind: 'download.urls',
        urls: ['https://example.com/video.mp4']
      }
    })).toEqual({
      kind: 'rejected',
      input: 'https://example.com/existing.zip',
      code: 'VIDEO_ACTIVATION_SOURCE_DENIED',
      message: '仅接受来自视频链接抽取应用的下载链接。'
    });
  });

  it.each([
    ['missing payload', undefined],
    ['wrong version', { version: 2, kind: 'download.urls', urls: ['https://example.com/a'] }],
    ['wrong kind', { version: 1, kind: 'download.files', urls: ['https://example.com/a'] }],
    ['extra field', { version: 1, kind: 'download.urls', urls: ['https://example.com/a'], sourceAppId: 'com.attson.video' }],
    ['empty URLs', { version: 1, kind: 'download.urls', urls: [] }],
    ['too many URLs', { version: 1, kind: 'download.urls', urls: Array.from({ length: 101 }, (_, index) => `https://example.com/${index}`) }],
    ['non-string URL', { version: 1, kind: 'download.urls', urls: [42] }],
    ['blank URL', { version: 1, kind: 'download.urls', urls: ['   '] }],
    ['unsupported protocol', { version: 1, kind: 'download.urls', urls: ['ftp://example.com/file'] }],
    ['URL credentials', { version: 1, kind: 'download.urls', urls: ['https://user:secret@example.com/file'] }],
    ['overlong URL', { version: 1, kind: 'download.urls', urls: [`https://example.com/${'a'.repeat(8193)}`] }],
    ['duplicate URL', { version: 1, kind: 'download.urls', urls: ['https://example.com/a', 'https://example.com/a'] }]
  ])('rejects an invalid payload atomically: %s', (_name, payload) => {
    const inbox = new VideoActivationInbox();

    expect(inbox.accept('https://example.com/existing.zip', {
      activationId: 'activation-invalid',
      sourceAppId: 'com.attson.video',
      payload
    })).toEqual({
      kind: 'rejected',
      input: 'https://example.com/existing.zip',
      code: 'VIDEO_ACTIVATION_INVALID_PAYLOAD',
      message: '视频应用发送的下载链接格式无效。'
    });
  });

  it('processes the same activation ID only once', () => {
    const inbox = new VideoActivationInbox();
    const notification = {
      activationId: 'activation-once',
      sourceAppId: 'com.attson.video',
      payload: {
        version: 1,
        kind: 'download.urls',
        urls: ['https://example.com/video.mp4']
      }
    };
    const accepted = inbox.accept('', notification);

    expect(inbox.accept(accepted.input, notification)).toEqual({
      kind: 'duplicate',
      input: 'https://example.com/video.mp4'
    });
  });

  it('bounds remembered activation IDs and evicts the oldest first', () => {
    const inbox = new VideoActivationInbox(2);
    const activation = (activationId: string, url: string) => ({
      activationId,
      sourceAppId: 'com.attson.video',
      payload: {
        version: 1,
        kind: 'download.urls',
        urls: [url]
      }
    });
    let input = '';
    for (const [activationId, url] of [
      ['activation-1', 'https://example.com/1'],
      ['activation-2', 'https://example.com/2'],
      ['activation-3', 'https://example.com/3']
    ]) {
      const result = inbox.accept(input, activation(activationId, url));
      expect(result.kind).toBe('accepted');
      input = result.input;
    }

    expect(inbox.accept(input, activation('activation-1', 'https://example.com/4'))).toEqual({
      kind: 'accepted',
      input: [
        'https://example.com/1',
        'https://example.com/2',
        'https://example.com/3',
        'https://example.com/4'
      ].join('\n'),
      added: 1
    });
    expect(inbox.accept(input, activation('activation-3', 'https://example.com/5'))).toEqual({
      kind: 'duplicate',
      input
    });
  });
});
