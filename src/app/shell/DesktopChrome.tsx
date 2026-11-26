import { useState } from "react";
import { Button } from "../../shared/ui/ios/controls";
import { useStore, type Tab } from "../../shared/model/store";
import { APP_VERSION } from "../../shared/api/api";
import { Avatar } from "../../shared/ui/ui";
import DownloadSheet from "../../shared/ui/DownloadSheet";
import { useDownloads } from "../../downloads";
import {
  IDownload, IGrid, IGridF, IHome, IHomeF, IMoon, ISearch, ISettings, ISidebar,
  ISun, ITrophy, IUser, IUserF, Logo,
} from "../../shared/ui/icons";
import { tr } from "../../shared/i18n";

export function DesktopTitleBar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  const { theme, setTheme, setTab, theater, setTheater } = useStore();
  const [downloadOpen, setDownloadOpen] = useState(false);

  return (
    <>
    <header className="win-titlebar">
      <div className="win-wordmark">
        <Logo size={24} />
        <span>HG<span className="win-wordmark-light">hub</span>X</span>
      </div>
      <Button className="win-caption-tool" aria-label={collapsed || theater ? tr("展开侧栏") : tr("收起侧栏")}
        onClick={() => { if (theater) { setTheater(false); if (collapsed) onToggle(); } else onToggle(); }}>
        <ISidebar size={17} strokeWidth={1.7} />
      </Button>
      <div className="win-drag-region" />
      <Button className="win-caption-tool" aria-label={tr("下载管理")} title={tr("下载管理")} onClick={() => setDownloadOpen(true)}>
        <IDownload size={17} strokeWidth={1.7} />
      </Button>
      <Button className="win-caption-tool" aria-label={tr("切换外观")} onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
        {theme === "dark" ? <ISun size={17} strokeWidth={1.7} /> : <IMoon size={17} strokeWidth={1.7} />}
      </Button>
      <Button className="win-caption-tool" aria-label={tr("搜索")} onClick={() => setTab("search")}>
        <ISearch size={17} strokeWidth={1.7} />
      </Button>
    </header>
    <DownloadSheet open={downloadOpen} onClose={() => setDownloadOpen(false)} />
    </>
  );
}

export function DesktopRail({ collapsed }: { collapsed: boolean }) {
  const { tab, setTab, user, setLoginOpen, openRootPage, platform, stack } = useStore();
  const [downloadOpen, setDownloadOpen] = useState(false);
  const { tasks: downloadTasks } = useDownloads();
  const activeDownloads = downloadTasks.filter((t) => t.status === "queued" || t.status === "downloading").length;
  /* 从侧栏进入的栈页（排行榜 / 设置）接管高亮，否则「我的」会一直亮着，看起来不像独立页面 */
  const root = stack[0];
  const railPage: "ranks" | "settings" | null =
    root?.kind === "settings" ? "settings" : root?.kind === "ranks" ? "ranks" : null;
  const onTab = (key: Tab | "search") => !stack.length && tab === key;
  const items: { key: Tab; label: string; Icon: typeof IHome; Filled: typeof IHomeF }[] = [
    { key: "home", label: tr("首页"), Icon: IHome, Filled: IHomeF },
    { key: "category", label: tr("分类"), Icon: IGrid, Filled: IGridF },
    { key: "profile", label: tr("我的"), Icon: IUser, Filled: IUserF },
  ];
  return (
    <>
    <nav className={`win-sidebar ${collapsed ? "is-compact" : ""}`} aria-label={tr("主导航")}>
      <div className="win-sidebar-inner">
        <Button className={`win-search-launch ${onTab("search") ? "is-selected" : ""}`}
          onClick={() => setTab("search")} aria-label={tr("搜索")}>
          <span className="win-nav-symbol"><ISearch size={19} strokeWidth={1.7} /></span>
          <span className="win-nav-copy">{tr("搜索")}</span>
          <kbd className="win-nav-key">/</kbd>
        </Button>
        <div className="win-nav-items">
          {items.map(({ key, label, Icon, Filled }) => (
            <Button key={key} aria-label={label} aria-current={onTab(key) ? "page" : undefined}
              className={`win-nav-item ${onTab(key) ? "is-selected" : ""}`}
              onClick={() => setTab(key)}>
              <span className="win-nav-symbol">{onTab(key) ? <Filled size={21} /> : <Icon size={21} strokeWidth={1.7} />}</span>
              <span className="win-nav-copy">{label}</span>
            </Button>
          ))}
          <Button aria-label={tr("排行榜")} className={`win-nav-item ${railPage === "ranks" ? "is-selected" : ""}`}
            aria-current={railPage === "ranks" ? "page" : undefined}
            onClick={() => { if (railPage !== "ranks" || stack.length > 1) openRootPage({ kind: "ranks" }); }}>
            <span className="win-nav-symbol"><ITrophy size={21} strokeWidth={1.7} /></span>
            <span className="win-nav-copy">{tr("排行榜")}</span>
          </Button>
          <Button aria-label={tr("下载管理")} className="win-nav-item" onClick={() => setDownloadOpen(true)}>
            <span className="win-nav-symbol"><IDownload size={20} strokeWidth={1.7} /></span>
            <span className="win-nav-copy">{activeDownloads > 0 ? `${tr("下载")} (${activeDownloads})` : tr("下载")}</span>
          </Button>
          <Button aria-label={tr("设置")} className={`win-nav-item ${railPage === "settings" ? "is-selected" : ""}`}
            aria-current={railPage === "settings" ? "page" : undefined}
            onClick={() => { if (railPage !== "settings" || stack.length > 1) openRootPage({ kind: "settings" }); }}>
            <span className="win-nav-symbol"><ISettings size={21} strokeWidth={1.7} /></span>
            <span className="win-nav-copy">{tr("设置")}</span>
          </Button>
        </div>
        <div className="win-sidebar-bottom">
          <Button aria-label={tr("账号与个人中心")} className="win-account" onClick={() => { setTab("profile"); if (!user) setLoginOpen(true); }}>
            <Avatar size={34} hue={220} name={user?.nickname} icon={user ? undefined : <IUser size={17} />} />
            <span className="win-account-copy">
              <strong>{user?.nickname || tr("登录 HGhubX")}</strong>
            </span>
          </Button>
          <span className="win-sidebar-version">HGhubX for {platform === "macos" ? "macOS" : platform === "linux" ? "Linux" : "Windows"} {APP_VERSION}</span>
        </div>
      </div>
    </nav>
    <DownloadSheet open={downloadOpen} onClose={() => setDownloadOpen(false)} />
    </>
  );
}
