import { describe, expect, it } from 'vitest';

import {
  TaskEventBuffer,
  addTaskIfMissing,
  mergeTaskEvent,
  parseDownloadUrls,
  reconcileStartedTask
} from '../src/frontend/downloadState';

describe('download page state', () => {
  it('splits batch URLs and preserves existing progress on zero-progress running events', () => {
    expect(parseDownloadUrls(' https://a.test/a.zip,\nhttps://b.test/b.zip ,, ')).toEqual([
      'https://a.test/a.zip',
      'https://b.test/b.zip'
    ]);
    const task = {
      id: 'dl-1',
      url: 'https://a.test/a.zip',
      downloadDir: '/tmp',
      status: 'running' as const,
      progress: 48,
      createdAt: '2026-11-14 10:00:00'
    };

    expect(mergeTaskEvent(task, { id: 'dl-1', status: 'running', progress: 0, message: 'still running' }))
      .toMatchObject({ progress: 48, message: 'still running' });
  });

  it('applies a terminal event that arrived before the start response created the task row', () => {
    const events = new TaskEventBuffer();
    const completed = {
      id: 'dl-2',
      status: 'completed' as const,
      progress: 100,
      message: '下载完成',
      localPath: '/tmp/file.zip'
    };

    const beforeResponse = events.apply([], completed);
    const tasks = addTaskIfMissing(beforeResponse.tasks, {
      id: 'dl-2',
      url: 'https://a.test/file.zip',
      downloadDir: '/tmp',
      status: 'queued',
      progress: 0,
      createdAt: '2026-11-14 10:00:00'
    });

    expect(beforeResponse.matched).toBe(false);
    expect(events.hydrate(tasks)[0]).toMatchObject({
      id: 'dl-2',
      status: 'completed',
      progress: 100,
      localPath: '/tmp/file.zip'
    });
  });

  it('does not replace an authoritative task with a later queued placeholder', () => {
    const completed = {
      id: 'dl-3',
      url: 'https://a.test/file.zip',
      downloadDir: '/tmp',
      status: 'completed' as const,
      progress: 100,
      createdAt: '2026-11-14 10:00:00'
    };

    expect(addTaskIfMissing([completed], { ...completed, status: 'queued', progress: 0 }))
      .toEqual([completed]);
  });

  it('keeps one authoritative row when a retry event refreshes before the start response', () => {
    const events = new TaskEventBuffer();
    const completed = {
      id: 'dl-4',
      url: 'https://a.test/file.zip',
      downloadDir: '/tmp',
      status: 'completed' as const,
      progress: 100,
      createdAt: '2026-11-14 10:00:00'
    };

    expect(reconcileStartedTask([completed], events, {
      ...completed,
      status: 'queued',
      progress: 0
    })).toEqual([completed]);
  });

  it('never regresses terminal state or running progress with an older event', () => {
    const completed = {
      id: 'dl-4',
      url: 'https://a.test/file.zip',
      downloadDir: '/tmp',
      status: 'completed' as const,
      progress: 100,
      createdAt: '2026-11-14 10:00:00'
    };
    const running = { ...completed, id: 'dl-5', status: 'running' as const, progress: 72 };

    expect(mergeTaskEvent(completed, { id: 'dl-4', status: 'running', progress: 90 }))
      .toEqual(completed);
    expect(mergeTaskEvent(running, { id: 'dl-5', status: 'running', progress: 31 }))
      .toMatchObject({ status: 'running', progress: 72 });
  });
});
