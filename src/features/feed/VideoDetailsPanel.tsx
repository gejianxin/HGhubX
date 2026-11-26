import { fmt } from "../../shared/api/api";
import type { VideoCard } from "../../shared/api/types";
import { IClose, IDownload } from "../../shared/ui/icons";
import { Button, Surface } from "../../shared/ui/ios/controls";
import { Avatar, Img } from "../../shared/ui/ui";
import { ContentTags } from "../../shared/ui/ContentTags";
import { FilteredContent } from "../../shared/ui/FilteredContent";
import { RowCard } from "../../shared/ui/cards";
import "../../styles/video-details.css";
import { tr } from "../../shared/i18n";

type VideoMeta = {
  title: string; cover: string; score: number; like: number; collect: number; comment: number;
  author: string; authorId: number; tags: readonly string[]; desc: string;
};

/** One details body for the mobile sheet and desktop dock. */
export default function VideoDetailsPanel({ meta, videoId, episodeCount, related, following, followPending,
  downloadLabel = tr("下载当前集"), onDownload, onDownloadAll,
  onFollow, onTag, onBlockTag, onAuthor, onRelated, onClose }: {
  meta: VideoMeta; videoId: number; episodeCount: number; related: VideoCard[];
  following: boolean; followPending: boolean;
  downloadLabel?: string; onDownload?: () => void; onDownloadAll?: () => void;
  onFollow: () => void; onTag: (tag: string) => void;
  onBlockTag?: (tag: string) => void;
  onAuthor: () => void; onRelated: (video: VideoCard, visible: VideoCard[]) => void; onClose?: () => void;
}) {
  return <div className="video-details-panel" data-video-details>
    {onClose && <div className="video-details-heading">
      <h2>{tr("视频详情")}</h2>
      <Button className="ios-icon-button" aria-label={tr("关闭视频详情")} onClick={onClose}><IClose size={18} /></Button>
    </div>}
    <div className="video-details-scroll scroll-y">
      <div className="video-details-overview">
        <Surface className="video-details-cover"><Img src={meta.cover} eager loadingIndicator alt={meta.title} /></Surface>
        <div className="video-details-copy">
          <h3>{meta.title}</h3>
          <div className="video-details-stats">
            {!!meta.score && <span className="text-amber font-bold">{tr("{0} 分", meta.score.toFixed(1))}</span>}
            {episodeCount > 0 && <span>{tr("共 {0} 集", episodeCount)}</span>}
          </div>
          <div className="video-details-stats"><span>{tr("点赞 {0}", fmt(meta.like))}</span><span>{tr("收藏 {0}", fmt(meta.collect))}</span><span>{tr("评论 {0}", fmt(meta.comment))}</span></div>
        </div>
      </div>

      {(onDownload || onDownloadAll) && (
        <div className="flex flex-wrap gap-2">
          {onDownload && (
            <Button onClick={onDownload} className="flex-1 min-w-[120px] h-10 px-3 rounded-(--r-sm) bg-s3 text-[13px] font-semibold text-on inline-flex items-center justify-center gap-1.5 pressable">
              <IDownload size={15} className="text-primary" />
              <span>{downloadLabel === tr("下载") ? tr("下载当前集") : downloadLabel}</span>
            </Button>
          )}
          {onDownloadAll && episodeCount > 1 && (
            <Button onClick={onDownloadAll} className="flex-1 min-w-[120px] h-10 px-3 rounded-(--r-sm) bg-primary text-[13px] font-semibold text-on-primary inline-flex items-center justify-center gap-1.5 pressable">
              <IDownload size={15} />
              <span>{tr("下载全集 ({0})", episodeCount)}</span>
            </Button>
          )}
        </div>
      )}

      <section className="video-details-section" aria-label={tr("标签导航")}>
        <div className="video-details-section-heading"><h3>{tr("标签 · 点击查看同类{0}", onBlockTag ? tr(" · 长按屏蔽") : "")}</h3></div>
        <ContentTags tags={meta.tags} onOpen={onTag} onBlock={onBlockTag} />
      </section>

      <Surface className="video-details-author">
        <Button onClick={onAuthor} disabled={meta.authorId <= 0} className="video-details-author-link">
          <Avatar size={36} name={meta.author} hue={220} />
          <span className="min-w-0"><span className="block text-[14px] font-semibold truncate">{meta.author}</span><span className="text-[12px] text-mut">{tr("作者主页")}</span></span>
        </Button>
        {meta.authorId > 0 && <Button onClick={onFollow} disabled={followPending} aria-busy={followPending} aria-pressed={following}
          className={`video-details-follow ${following ? "bg-s3 text-mut" : "bg-primary text-on-primary"}`}>
          {followPending ? tr("处理中…") : following ? tr("已关注") : tr("+ 关注")}
        </Button>}
      </Surface>

      <section className="video-details-section">
        <h3 className="video-details-section-heading">{tr("简介")}</h3>
        <p className="video-details-description">{meta.desc || tr("暂无简介")}</p>
      </section>

      {related.length > 0 && <section className="video-details-section">
        <h3 className="video-details-section-heading">{tr("相关推荐")}</h3>
        <FilteredContent items={related.slice(0, 12)} scopeKey={videoId}>{visible => <div className="scroll-x flex gap-2.5 snap-x">
          {visible.map(video => <RowCard key={video.id} v={video} onOpen={() => onRelated(video, visible)} />)}
        </div>}</FilteredContent>
      </section>}
    </div>
  </div>;
}
