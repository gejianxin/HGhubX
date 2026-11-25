/**
 * Pure-Rust HTTP/SSE bridge replacing @tauri-apps/api and plugins.
 * Communicates with the same-origin local Axum server on 127.0.0.1.
 */

export function isTauri(): boolean {
  return false;
}

export async function invoke<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const endpoint = cmd === "plugin:app|exit" ? "app_exit" : cmd;
  const response = await fetch(`/api/invoke/${encodeURIComponent(endpoint)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });
  const text = await response.text();
  let parsed: unknown = null;
  if (text) {
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = text;
    }
  }
  if (!response.ok) {
    const errMsg =
      parsed &&
      typeof parsed === "object" &&
      "error" in parsed &&
      typeof (parsed as { error: unknown }).error === "string"
        ? (parsed as { error: string }).error
        : typeof parsed === "string" && parsed
          ? parsed
          : `HTTP ${response.status}`;
    throw errMsg;
  }
  return parsed as T;
}

let sharedSource: EventSource | null = null;
const eventListeners = new Map<string, Set<(payload: unknown) => void>>();

function ensureEventSource(): EventSource {
  if (!sharedSource) {
    sharedSource = new EventSource("/api/events");
  }
  return sharedSource;
}

export async function listen<T = unknown>(
  event: string,
  handler: (e: { payload: T }) => void,
): Promise<() => void> {
  if (typeof EventSource === "undefined") return () => {};
  const es = ensureEventSource();
  let listeners = eventListeners.get(event);
  if (!listeners) {
    listeners = new Set();
    eventListeners.set(event, listeners);
    es.addEventListener(event, (msg: MessageEvent) => {
      let payload: unknown = msg.data;
      try {
        payload = JSON.parse(msg.data);
      } catch {
        // keep raw string
      }
      const set = eventListeners.get(event);
      if (set) {
        for (const fn of set) fn(payload);
      }
    });
  }
  const wrapped = (payload: unknown) => handler({ payload: payload as T });
  listeners.add(wrapped);
  return () => {
    listeners?.delete(wrapped);
  };
}

export async function addPluginListener(
  _plugin: string,
  _event: string,
  _cb: (payload: unknown) => void,
): Promise<{ unregister: () => Promise<void> }> {
  return { unregister: async () => {} };
}

export class Channel<T = unknown> {
  onmessage: (response: T) => void = () => {};
  private unlisten: (() => void) | null = null;

  constructor(onmessage?: (response: T) => void) {
    if (onmessage) this.onmessage = onmessage;
    void listen<T>("tv-update:progress", (e) => {
      this.onmessage(e.payload);
    }).then((fn) => {
      this.unlisten = fn;
    });
  }

  toJSON() {
    return null;
  }

  dispose() {
    this.unlisten?.();
    this.unlisten = null;
  }
}

export function getCurrentWindow() {
  return {
    isMaximized: async () => false,
    isFullscreen: async () => Boolean(typeof document !== "undefined" && document.fullscreenElement),
    setFullscreen: async (_fs: boolean) => {},
    toggleMaximize: async () => {},
    minimize: async () => {},
    close: async () => {
      window.close();
    },
    onResized: async (_cb: () => void) => () => {},
    startResizeDragging: async (_dir: string) => {},
  };
}

export async function writeText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const el = document.createElement("textarea");
  el.value = text;
  el.style.position = "fixed";
  el.style.opacity = "0";
  document.body.appendChild(el);
  el.select();
  try {
    if (!document.execCommand("copy")) {
      throw new Error("当前环境不支持复制，请手动复制");
    }
  } finally {
    document.body.removeChild(el);
  }
}
