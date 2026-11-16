<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import {
  darkTheme,
  NAlert,
  NButton,
  NConfigProvider,
  NInput,
  NInputGroup,
  NInputNumber,
  NSelect
} from 'naive-ui';

import Panel from './components/Panel.vue';
import TaskRow from './components/TaskRow.vue';
import { VideoActivationInbox } from './frontend/appActivation';
import {
  TaskEventBuffer,
  parseDownloadUrls,
  reconcileStartedTask
} from './frontend/downloadState';
import type {
  DownloadHost,
  DownloadTask,
  StartDownloadRequest,
  ThemeName
} from './frontend/host';

const props = defineProps<{ host: DownloadHost }>();

const minSplitOptions = [
  { label: '1M', value: '1M' },
  { label: '4M', value: '4M' },
  { label: '8M', value: '8M' },
  { label: '16M', value: '16M' }
];
const theme = ref<ThemeName>('dark');
const url = ref('');
const downloadDir = ref('');
const fileName = ref('');
const connections = ref(16);
const split = ref(16);
const minSplitSize = ref('1M');
const tasks = ref<DownloadTask[]>([]);
const submitting = ref(false);
const choosingDir = ref(false);
const connecting = ref(true);
const notice = ref('');
const noticeType = ref<'error' | 'success'>('error');
const taskSearch = ref('');
let unsubscribe: (() => void) | undefined;
let unsubscribeActivation: (() => void) | undefined;
const taskEvents = new TaskEventBuffer();
const activationInbox = new VideoActivationInbox();
let refreshPromise: Promise<void> | null = null;
let refreshAgain = false;

const activeCount = computed(
  () => tasks.value.filter((task) => task.status === 'queued' || task.status === 'running').length
);
const completedCount = computed(() => tasks.value.filter((task) => task.status === 'completed').length);
const hasCompleted = computed(() => completedCount.value > 0);
const filteredTasks = computed(() => {
  const query = taskSearch.value.trim().toLocaleLowerCase();
  if (!query) return tasks.value;
  return tasks.value.filter((task) =>
    [task.url, task.fileName ?? '', task.message ?? '', task.localPath ?? '']
      .join('\n')
      .toLocaleLowerCase()
      .includes(query)
  );
});

onMounted(() => void connect());
onUnmounted(() => {
  unsubscribe?.();
  unsubscribeActivation?.();
  props.host.dispose();
});

async function connect() {
  connecting.value = true;
  notice.value = '';
  unsubscribe?.();
  unsubscribeActivation?.();
  unsubscribeActivation = props.host.subscribeActivation((activation) => {
    const result = activationInbox.accept(url.value, activation);
    if (result.kind === 'duplicate') return;
    url.value = result.input;
    if (result.kind === 'accepted') {
      noticeType.value = 'success';
      notice.value = result.added > 0
        ? `已接收 ${result.added} 个视频下载链接。`
        : '视频下载链接已存在，无需重复添加。';
    } else {
      noticeType.value = 'error';
      notice.value = `${result.code}: ${result.message}`;
    }
  });
  try {
    await props.host.initialize(applyTheme);
    unsubscribe = props.host.subscribe((event) => {
      const merged = taskEvents.apply(tasks.value, event);
      tasks.value = merged.tasks;
      if (!merged.matched) void refreshTasks().catch((error) => {
        noticeType.value = 'error';
        notice.value = formatError(error);
      });
    });
    const [directory, listed] = await Promise.all([
      props.host.request<{ path: string }>('download.defaultDirectory'),
      props.host.request<{ tasks: DownloadTask[] }>('download.tasks.list')
    ]);
    downloadDir.value = directory.path;
    tasks.value = taskEvents.hydrate(listed.tasks);
  } catch (error) {
    noticeType.value = 'error';
    notice.value = formatError(error);
  } finally {
    connecting.value = false;
  }
}

function applyTheme(value: ThemeName) {
  theme.value = value;
  document.documentElement.dataset.theme = value;
}

async function chooseDownloadDir() {
  choosingDir.value = true;
  notice.value = '';
  try {
    const path = await props.host.pickDirectory(downloadDir.value || undefined);
    if (path) downloadDir.value = path;
  } catch (error) {
    noticeType.value = 'error';
    notice.value = formatError(error);
  } finally {
    choosingDir.value = false;
  }
}

async function startDownload() {
  const urls = parseDownloadUrls(url.value);
  const directory = downloadDir.value.trim();
  if (urls.length === 0 || !directory) {
    noticeType.value = 'error';
    notice.value = '请填写下载链接和保存目录。';
    return;
  }
  notice.value = '';
  submitting.value = true;
  try {
    const singleFileName = urls.length === 1 ? fileName.value.trim() || undefined : undefined;
    for (const itemUrl of urls) {
      const request: StartDownloadRequest = {
        url: itemUrl,
        downloadDir: directory,
        fileName: singleFileName,
        connections: connections.value,
        split: split.value,
        minSplitSize: minSplitSize.value
      };
      const response = await props.host.request<{ id: string }>('download.tasks.start', { request });
      tasks.value = reconcileStartedTask(
        tasks.value,
        taskEvents,
        pendingTask(response.id, request, '任务已提交给 aria2')
      );
    }
    url.value = '';
    fileName.value = '';
  } catch (error) {
    noticeType.value = 'error';
    notice.value = formatError(error);
  } finally {
    submitting.value = false;
  }
}

function refreshTasks(): Promise<void> {
  if (refreshPromise) {
    refreshAgain = true;
    return refreshPromise;
  }
  refreshPromise = (async () => {
    do {
      refreshAgain = false;
      const listed = await props.host.request<{ tasks: DownloadTask[] }>('download.tasks.list');
      tasks.value = taskEvents.hydrate(listed.tasks);
    } while (refreshAgain);
  })()
    .finally(() => {
      refreshPromise = null;
    });
  return refreshPromise;
}

async function cancelTask(id: string) {
  await runAction(() => props.host.request('download.tasks.cancel', { id }));
}

async function openTaskFolder(id: string) {
  await runAction(() => props.host.request('download.tasks.openFolder', { id }));
}

async function retryTask(task: DownloadTask) {
  submitting.value = true;
  notice.value = '';
  try {
    const request: StartDownloadRequest = {
      url: task.url,
      downloadDir: task.downloadDir,
      fileName: task.fileName ?? undefined,
      connections: connections.value,
      split: split.value,
      minSplitSize: minSplitSize.value
    };
    const response = await props.host.request<{ id: string }>('download.tasks.start', { request });
    tasks.value = reconcileStartedTask(
      tasks.value,
      taskEvents,
      pendingTask(response.id, request, '任务已重新提交给 aria2')
    );
  } catch (error) {
    noticeType.value = 'error';
    notice.value = formatError(error);
  } finally {
    submitting.value = false;
  }
}

async function deleteTask(id: string) {
  await runAction(async () => {
    await props.host.request('download.tasks.delete', { id });
    tasks.value = tasks.value.filter((task) => task.id !== id);
  });
}

async function clearCompleted() {
  await runAction(async () => {
    const result = await props.host.request<{ count: number }>('download.tasks.clearCompleted');
    if (result.count > 0) tasks.value = tasks.value.filter((task) => task.status !== 'completed');
  });
}

async function runAction(action: () => Promise<unknown>) {
  notice.value = '';
  try {
    await action();
  } catch (error) {
    noticeType.value = 'error';
    notice.value = formatError(error);
  }
}

function pendingTask(id: string, request: StartDownloadRequest, message: string): DownloadTask {
  return {
    id,
    url: request.url,
    downloadDir: request.downloadDir,
    fileName: request.fileName,
    status: 'queued',
    progress: 0,
    speed: null,
    eta: null,
    message,
    createdAt: new Date().toLocaleTimeString(),
    startedAt: null,
    finishedAt: null,
    localPath: null
  };
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null) {
    const value = error as { code?: unknown; message?: unknown };
    if (typeof value.message === 'string') {
      return typeof value.code === 'string' ? `${value.code}: ${value.message}` : value.message;
    }
  }
  return String(error);
}
</script>

<template>
  <n-config-provider :theme="theme === 'dark' ? darkTheme : null">
    <main class="page">
      <header class="page-header">
        <div>
          <h1>多线程下载工作台</h1>
          <p>本机 aria2c 引擎，支持断点续传、分片、多连接和实时进度回传。</p>
        </div>
        <div class="stats">
          <span class="stat active">进行中 <b>{{ activeCount }}</b></span>
          <span class="stat">已完成 <b>{{ completedCount }}</b></span>
        </div>
      </header>

      <n-alert v-if="notice" :type="noticeType" :bordered="false" class="notice">
        <div class="notice-content"><span>{{ notice }}</span><n-button v-if="noticeType === 'error' && (connecting || tasks.length === 0)" size="tiny" secondary @click="connect">重试连接</n-button></div>
      </n-alert>

      <div class="workbench" :class="{ muted: connecting }">
        <Panel title="新建下载">
          <template #right><span>支持批量</span></template>
          <form class="form" @submit.prevent="startDownload">
            <label class="field">
              <span>资源链接（每行一个，或用逗号分隔）</span>
              <n-input v-model:value="url" type="textarea" placeholder="https://example.com/file-a.zip&#10;https://example.com/file-b.zip" :autosize="{ minRows: 5, maxRows: 10 }" />
            </label>
            <label class="field">
              <span>保存目录</span>
              <n-input-group>
                <n-input v-model:value="downloadDir" placeholder="/Users/you/Downloads" />
                <n-button secondary :loading="choosingDir" @click="chooseDownloadDir">选择文件夹</n-button>
              </n-input-group>
            </label>
            <label class="field">
              <span>文件名（仅单个链接时生效）</span>
              <n-input v-model:value="fileName" placeholder="archive.zip" />
            </label>
            <div class="row3">
              <label class="field"><span>单服务器连接数</span><n-input-number v-model:value="connections" :min="1" :max="16" /></label>
              <label class="field"><span>分片数</span><n-input-number v-model:value="split" :min="1" :max="64" /></label>
              <label class="field"><span>最小分片大小</span><n-select v-model:value="minSplitSize" :options="minSplitOptions" /></label>
            </div>
            <n-button type="primary" block attr-type="submit" :loading="submitting" :disabled="connecting">
              {{ submitting ? '正在创建...' : '开始下载' }}
            </n-button>
          </form>
        </Panel>

        <Panel title="任务队列">
          <template #right>
            <div class="queue-actions">
              <n-button v-if="hasCompleted" size="tiny" secondary @click="clearCompleted">清理已完成</n-button>
              <span class="mono">实时</span>
            </div>
          </template>
          <n-input v-if="tasks.length" v-model:value="taskSearch" size="small" clearable placeholder="搜索 URL / 文件名 / 路径 / 备注" class="search" />
          <div v-if="connecting" class="empty">正在连接下载服务</div>
          <div v-else-if="tasks.length === 0" class="empty">还没有下载任务</div>
          <div v-else-if="filteredTasks.length === 0" class="empty">没有匹配的任务</div>
          <div v-else class="tasks">
            <TaskRow v-for="task in filteredTasks" :key="task.id" :task="task" @cancel="cancelTask" @open-folder="openTaskFolder" @retry="retryTask" @delete="deleteTask" />
          </div>
        </Panel>
      </div>
    </main>
  </n-config-provider>
</template>

<style scoped>
.page { min-height: 100vh; padding: 18px; background: var(--bg-base); color: var(--text); }
.page-header { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
.page-header h1 { margin: 0; font-size: var(--fs-xl); font-weight: 600; letter-spacing: 0; }
.page-header p { margin: 3px 0 0; color: var(--text-muted); font-size: var(--fs-xs); }
.stats { display: flex; gap: 8px; flex-wrap: wrap; }
.stat { border: 1px solid var(--line); border-radius: var(--radius-pill); padding: 4px 10px; color: var(--text-muted); font-size: var(--fs-xs); }
.stat.active { color: var(--accent); background: var(--accent-soft); }
.notice { margin-bottom: 12px; }
.notice-content { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.workbench { display: grid; grid-template-columns: minmax(360px, 1.1fr) minmax(340px, 1fr); gap: 16px; }
.workbench.muted { opacity: 0.72; }
.form { display: grid; gap: 12px; }
.field { display: grid; gap: 5px; min-width: 0; }
.field > span { color: var(--text-muted); font-size: var(--fs-xxs); }
.field :deep(.n-input-number) { width: 100%; }
.row3 { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
.queue-actions { display: flex; align-items: center; gap: 10px; }
.search { margin-bottom: 8px; }
.tasks { display: grid; gap: 6px; }
.empty { padding: 60px 20px; text-align: center; color: var(--text-muted); border: 1px dashed var(--line-strong); border-radius: var(--radius-md); font-size: var(--fs-sm); }
@media (max-width: 900px) { .workbench { grid-template-columns: 1fr; } }
@media (max-width: 560px) { .page { padding: 12px; } .page-header { display: grid; } .row3 { grid-template-columns: 1fr; } }
</style>
