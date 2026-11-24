import { useEffect } from "react";
import { Button } from "./ios/controls";
import { Sheet } from "./ui";
import { isMobile } from "../api";
import { useStore } from "../store";
import {
  cancelAllDownloads,
  cancelDownload,
  downloadProgressPct,
  formatBytes,
  openDownloadDir,
  removeDownload,
  startDownload,
  startSeriesDownload,
  syncDownloads,
  useDownloads,
} from "../downloads";
import { IClose, IDownload } from "../icons";

type Props = {
  open: boolean;
  onClose: () => void;
};

const STATUS_LABEL: Record<string, string> = {
  queued: "排队中",
  downloading: "下载中",
  done: "已完成",
  error: "失败",
  cancelled: "已取消",
};

export default function DownloadSheet({ open, onClose }: Props) {
  const { toast } = useStore();
  const { tasks, dir } = useDownloads();

  useEffect(() => {
    if (open) syncDownloads();
  }, [open]);

  const activeCount = tasks.filter((t) => t.status === "queued" || t.status === "downloading").length;
  const downloadingCount = tasks.filter((t) => t.status === "downloading").length;
  const queuedCount = tasks.filter((t) => t.status === "queued").length;
  const doneCount = tasks.filter((t) => t.status === "done").length;

  const onOpenDir = () => {
    void openDownloadDir().catch((err) => {
      toast(err instanceof Error ? err.message : "打开目录失败", "info");
    });
  };

  return (
    <Sheet open={open} onClose={onClose} label="下载管理">
      <div className="flex flex-col max-h-[76vh]">
        <div className="shrink-0 flex items-center justify-between gap-2 px-4 py-3 border-b border-line">
          <div className="flex items-center gap-2 min-w-0">
            <IDownload size={18} className="text-primary shrink-0" />
            <h2 className="text-[15px] font-bold text-on truncate">下载管理</h2>
          </div>
          <div className="flex items-center gap-1.5">
            {activeCount > 0 && (
              <Button
                variant="secondary"
                onClick={() => void cancelAllDownloads().then((n) => { if (n > 0) toast(`已取消 ${n} 个下载任务`, "info"); })}
                className="h-8 px-2.5 rounded-full text-[12px] font-medium pressable"
              >
                全部取消
              </Button>
            )}
            {!isMobile && dir && (
              <Button
                variant="secondary"
                onClick={onOpenDir}
                className="h-8 px-2.5 rounded-full text-[12px] font-medium pressable"
              >
                打开文件夹
              </Button>
            )}
            <Button
              aria-label="关闭下载管理"
              onClick={onClose}
              className="w-8 h-8 rounded-full flex items-center justify-center text-mut hover:bg-s3 pressable"
            >
              <IClose size={16} />
            </Button>
          </div>
        </div>

        {(dir || tasks.length > 0) && (
          <div className="shrink-0 px-4 py-2 text-[11px] text-mut bg-s2/60 border-b border-line flex items-center justify-between gap-2">
            <span className="truncate" title={dir}>{dir ? `保存位置：${dir}` : ""}</span>
            {tasks.length > 0 && (
              <span className="shrink-0 tabular-nums">
                {downloadingCount > 0 ? `${downloadingCount} 下载中` : ""}
                {queuedCount > 0 ? `${downloadingCount > 0 ? " · " : ""}${queuedCount} 排队` : ""}
                {doneCount > 0 ? `${downloadingCount > 0 || queuedCount > 0 ? " · " : ""}${doneCount} 完成` : ""}
              </span>
            )}
          </div>
        )}

        <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2.5">
          {tasks.length === 0 ? (
            <div className="py-10 text-center text-[13px] text-mut">
              暂无下载任务。在视频播放页点击「下载」即可保存本地。
            </div>
          ) : (
            tasks.map((task) => {
              const pct = downloadProgressPct(task);
              const active = task.status === "queued" || task.status === "downloading";
              return (
                <div key={task.id} className="rounded-[14px] bg-s2 p-3 border border-line/70">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0 flex-1">
                      <div className="text-[13.5px] font-semibold text-on truncate">{task.title}</div>
                      {task.subtitle && (
                        <div className="text-[12px] text-mut truncate mt-0.5">{task.subtitle}</div>
                      )}
                    </div>
                    <span
                      className={`shrink-0 text-[11px] font-semibold px-2 py-0.5 rounded-full ${
                        task.status === "done"
                          ? "bg-primary/15 text-primary"
                          : task.status === "error"
                            ? "bg-danger/15 text-danger"
                            : "bg-s3 text-mut"
                      }`}
                    >
                      {STATUS_LABEL[task.status] ?? task.status}
                    </span>
                  </div>

                  <div className="mt-2.5">
                    <div className="h-1.5 rounded-full bg-s3 overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-[width] duration-200 ${
                          task.status === "error" ? "bg-danger" : "bg-primary"
                        }`}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                    <div className="mt-1.5 flex items-center justify-between gap-2 text-[11.5px] text-mut tabular-nums">
                      <span>
                        {task.totalSegments > 0
                          ? `${task.doneSegments}/${task.totalSegments} 分片 (${pct}%)`
                          : active
                            ? "正在解析播放列表…"
                            : `${pct}%`}
                      </span>
                      <span>{formatBytes(task.bytes)}</span>
                    </div>
                  </div>

                  {task.error && (
                    <p role="alert" className="mt-1.5 text-[11.5px] text-danger break-all">
                      {task.error}
                    </p>
                  )}

                  {task.status === "done" && task.filePath && (
                    <p className="mt-1.5 text-[11px] text-mut truncate" title={task.filePath}>
                      {task.filePath}
                    </p>
                  )}

                  <div className="mt-2.5 flex items-center justify-end gap-2">
                    {active ? (
                      <Button
                        variant="secondary"
                        onClick={() => void cancelDownload(task.id)}
                        className="h-7 px-2.5 rounded-full text-[12px] pressable"
                      >
                        取消
                      </Button>
                    ) : (
                      <>
                        {(task.status === "error" || task.status === "cancelled") && (
                          <Button
                            variant="secondary"
                            onClick={() => {
                              void removeDownload(task.id);
                              const retry = task.videoId && task.episode
                                ? startSeriesDownload({
                                    videoId: task.videoId,
                                    title: task.title,
                                    episodes: [task.episode],
                                    preferHevc: !!task.preferHevc,
                                  })
                                : startDownload({
                                    url: task.url,
                                    title: task.title,
                                    subtitle: task.subtitle || undefined,
                                  });
                              void retry.catch((err) => {
                                toast(err instanceof Error ? err.message : "重试失败", "info");
                              });
                            }}
                            className="h-7 px-2.5 rounded-full text-[12px] pressable"
                          >
                            重试
                          </Button>
                        )}
                        <Button
                          variant="secondary"
                          onClick={() => void removeDownload(task.id)}
                          className="h-7 px-2.5 rounded-full text-[12px] pressable"
                        >
                          移除记录
                        </Button>
                      </>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </Sheet>
  );
}
