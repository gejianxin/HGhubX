import { useSyncExternalStore } from "react";
import { listen } from "./bridge";
import { api, type DownloadTask } from "./shared/api/api";

type Snapshot = {
  tasks: DownloadTask[];
  dir: string;
};

let state: Snapshot = { tasks: [], dir: "" };
const listeners = new Set<() => void>();
let initialized = false;

function emit() {
  for (const fn of listeners) fn();
}

function upsertTask(task: DownloadTask) {
  const index = state.tasks.findIndex((t) => t.id === task.id);
  const next =
    index >= 0
      ? state.tasks.map((t, i) => (i === index ? task : t))
      : [task, ...state.tasks];
  state = { ...state, tasks: next };
  emit();
}

export function syncDownloads() {
  if (!initialized) {
    initialized = true;
    void listen<DownloadTask>("download:progress", (event) => {
      if (event.payload?.id) upsertTask(event.payload);
    }).catch(() => {});
  }
  void api
    .downloadList()
    .then((tasks) => {
      state = { ...state, tasks };
      emit();
    })
    .catch(() => {});
  void api
    .downloadDir()
    .then((dir) => {
      state = { ...state, dir };
      emit();
    })
    .catch(() => {});
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function getSnapshot() {
  return state;
}

export function useDownloads() {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export async function startDownload(params: { url: string; title: string; subtitle?: string }) {
  syncDownloads();
  const task = await api.downloadStart(params.url, params.title, params.subtitle);
  upsertTask(task);
  return task;
}

export async function startSeriesDownload(params: {
  videoId: number;
  title: string;
  episodes: number[];
  preferHevc?: boolean;
}) {
  syncDownloads();
  const tasks = await api.downloadSeries(
    params.videoId,
    params.title,
    params.episodes,
    !!params.preferHevc,
  );
  for (const task of tasks) upsertTask(task);
  return tasks;
}

export async function cancelDownload(id: string) {
  const ok = await api.downloadCancel(id);
  if (ok) {
    state = {
      ...state,
      tasks: state.tasks.map((t) => (t.id === id ? { ...t, status: "cancelled", error: null } : t)),
    };
    emit();
  }
  return ok;
}

export async function cancelAllDownloads() {
  const count = await api.downloadCancelAll();
  if (count > 0) {
    state = {
      ...state,
      tasks: state.tasks.map((t) =>
        t.status === "queued" || t.status === "downloading"
          ? { ...t, status: "cancelled", error: null }
          : t,
      ),
    };
    emit();
  }
  return count;
}

export async function removeDownload(id: string) {
  const ok = await api.downloadRemove(id);
  if (ok) {
    state = { ...state, tasks: state.tasks.filter((t) => t.id !== id) };
    emit();
  }
  return ok;
}

export async function openDownloadDir() {
  await api.downloadOpenDir();
}

export function findTaskByUrl(tasks: DownloadTask[], url: string): DownloadTask | undefined {
  if (!url) return undefined;
  return (
    tasks.find((t) => t.url === url && (t.status === "downloading" || t.status === "queued")) ??
    tasks.find((t) => t.url === url && t.status === "done") ??
    tasks.find((t) => t.url === url)
  );
}

export function downloadProgressPct(task: DownloadTask): number {
  if (task.status === "done") return 100;
  if (task.totalSegments <= 0) return 0;
  return Math.min(99, Math.round((task.doneSegments / task.totalSegments) * 100));
}

export function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
