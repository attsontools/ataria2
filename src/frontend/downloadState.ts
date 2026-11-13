import type { DownloadEventPayload, DownloadTask } from './host';

export interface TaskEventMergeResult {
  tasks: DownloadTask[];
  matched: boolean;
}

export class TaskEventBuffer {
  private readonly pending = new Map<string, DownloadEventPayload>();

  apply(tasks: DownloadTask[], event: DownloadEventPayload): TaskEventMergeResult {
    let matched = false;
    const merged = tasks.map((task) => {
      if (task.id !== event.id) return task;
      matched = true;
      return mergeTaskEvent(task, event);
    });
    if (matched) this.pending.delete(event.id);
    else this.pending.set(event.id, event);
    return { tasks: merged, matched };
  }

  hydrate(tasks: DownloadTask[]): DownloadTask[] {
    return tasks.map((task) => {
      const event = this.pending.get(task.id);
      if (!event) return task;
      this.pending.delete(task.id);
      return mergeTaskEvent(task, event);
    });
  }
}

export function addTaskIfMissing(tasks: DownloadTask[], task: DownloadTask): DownloadTask[] {
  return tasks.some((current) => current.id === task.id) ? tasks : [task, ...tasks];
}

export function reconcileStartedTask(
  tasks: DownloadTask[],
  events: TaskEventBuffer,
  task: DownloadTask
): DownloadTask[] {
  return events.hydrate(addTaskIfMissing(tasks, task));
}

export function parseDownloadUrls(value: string): string[] {
  return value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function mergeTaskEvent(
  task: DownloadTask,
  event: DownloadEventPayload
): DownloadTask {
  if (task.id !== event.id) return task;
  if (isTerminal(task.status)) return task;
  if (task.status === 'running' && event.status === 'queued') return task;
  return {
    ...task,
    ...event,
    progress: event.status === 'completed'
      ? event.progress
      : Math.max(task.progress, event.progress)
  };
}

function isTerminal(status: DownloadTask['status']): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled';
}
