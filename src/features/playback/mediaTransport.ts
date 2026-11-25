import { LOCAL, playlist } from "../../shared/api/api";

type PlaybackSource = { engine: "hls.js" | "native"; url: string; route: string };

/** Only our own media routes carry a wrapped upstream URL. An upstream ?u= is opaque. */
export function playbackSource(src: string, hlsSupported: boolean): PlaybackSource {
  let route = "";
  let direct = false;
  try {
    const source = new URL(src);
    if (source.href.startsWith(`${LOCAL}/`)) {
      const sub = source.pathname.replace(/^\/hg/, "");
      if (sub === "/hls" || sub === "/seg") {
        route = `${sub}${source.search}${source.hash}`;
        direct = sub === "/seg";
      }
    }
  } catch { /* An unwrapped source is passed intact to the playlist route. */ }
  const url = route ? `${LOCAL}${route}` : playlist(src);
  if (!route) route = url.slice(LOCAL.length);
  return { engine: hlsSupported && !direct ? "hls.js" : "native", url, route };
}

export function connectNativeMedia(video: HTMLVideoElement, source: PlaybackSource, callbacks: {
  isCurrent: () => boolean;
  onReady: () => void;
  onError: (message: string) => void;
}) {
  let disposed = false;
  const current = () => !disposed && callbacks.isCurrent();
  const clear = () => {
    video.pause();
    video.removeAttribute("src");
    video.load();
  };
  clear();
  if (current()) {
    video.src = source.url;
    callbacks.onReady();
  }
  return () => {
    disposed = true;
    clear();
  };
}
