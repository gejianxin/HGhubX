//! 视频下载层：支持 HLS (`.m3u8`，含主清单选流、AES-128 解密、fMP4 INIT 段) 与直链媒体下载。
//!
//! 复用 [`crate::net::fetch_bytes`]，自动继承应用代理、备用 DNS 与严格网络策略。

use crate::app_context::AppContext;
use crate::net::fetch_bytes;
use aes::cipher::{BlockDecrypt, KeyInit};
use aes::Aes128;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DOWNLOAD_EVENT: &str = "download:progress";
const MAX_CONCURRENT_TASKS: usize = 2;
const MAX_CONCURRENT_SEGMENTS: usize = 3;
const SEGMENT_MAX_RETRIES: usize = 3;
const MAX_TASKS_HISTORY: usize = 200;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub title: String,
    pub subtitle: String,
    pub status: String, // "queued" | "downloading" | "done" | "error" | "cancelled"
    pub done_segments: usize,
    pub total_segments: usize,
    pub bytes: u64,
    pub file_path: String,
    pub error: Option<String>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_hevc: Option<bool>,
}

struct TaskEntry {
    task: DownloadTask,
    cancel: Arc<AtomicBool>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static TASKS: Lazy<Mutex<Vec<TaskEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));
static TASK_SEMAPHORE: Lazy<Arc<tokio::sync::Semaphore>> =
    Lazy::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TASKS)));

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn next_task_id() -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("dl-{}-{n}", now_secs())
}

fn resolve_download_dir(ctx: &AppContext) -> Result<PathBuf, String> {
    let base = ctx.download_dir.clone();
    fs::create_dir_all(&base).map_err(|e| format!("创建下载目录失败: {e}"))?;
    Ok(base)
}

fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c| c == '.' || c == ' ' || c == '_');
    let capped: String = trimmed.chars().take(80).collect();
    let capped = capped.trim_end_matches(|c| c == '.' || c == ' ').to_string();
    if capped.is_empty() {
        "video".to_string()
    } else {
        capped
    }
}

fn build_file_stem(title: &str, subtitle: &str) -> String {
    let base = sanitize_filename(title);
    let sub = sanitize_filename(subtitle);
    if subtitle.trim().is_empty() || sub == "video" {
        base
    } else {
        format!("{base} - {sub}")
    }
}

fn unique_target_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    for index in 1..=9999 {
        let candidate = dir.join(format!("{stem} ({index}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.{ext}", now_secs()))
}

/* ───────────────────────── HLS 解析与解密 ───────────────────────── */

fn absolutize(base: &str, target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_string();
    }
    if let Ok(b) = url::Url::parse(base) {
        if let Ok(j) = b.join(target) {
            return j.to_string();
        }
    }
    target.to_string()
}

fn hls_attributes(line: &str) -> Option<(&str, Vec<&str>)> {
    let (tag, rest) = line.split_once(':')?;
    let mut attributes = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, ch) in rest.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        }
        if ch == ',' && !quoted {
            attributes.push(&rest[start..i]);
            start = i + 1;
        }
    }
    attributes.push(&rest[start..]);
    Some((tag, attributes))
}

fn parse_attr_map(line: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some((_, attrs)) = hls_attributes(line) else {
        return map;
    };
    for attr in attrs {
        if let Some((k, v)) = attr.split_once('=') {
            let key = k.trim().to_ascii_uppercase();
            let val = v.trim();
            let unquoted = val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(val);
            map.insert(key, unquoted.to_string());
        }
    }
    map
}

fn parse_hex_iv(raw: &str) -> Option<[u8; 16]> {
    let hex = raw.trim().strip_prefix("0x").or_else(|| raw.trim().strip_prefix("0X"))?;
    if hex.len() > 32 || hex.is_empty() {
        return None;
    }
    let padded = format!("{hex:0>32}");
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&padded[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn sequence_iv(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&seq.to_be_bytes());
    iv
}

fn aes128_cbc_decrypt(data: &[u8], key: &[u8], iv: [u8; 16]) -> Result<Vec<u8>, String> {
    if key.len() != 16 {
        return Err(format!("AES-128 密钥长度异常: {} 字节", key.len()));
    }
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(format!("加密分片长度异常: {} 字节", data.len()));
    }
    let cipher = Aes128::new_from_slice(key).map_err(|_| "初始化 AES-128 解密器失败".to_string())?;
    let mut prev = iv;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let mut ct = [0u8; 16];
        ct.copy_from_slice(chunk);
        let mut block = ct;
        cipher.decrypt_block((&mut block).into());
        for i in 0..16 {
            out.push(block[i] ^ prev[i]);
        }
        prev = ct;
    }
    let pad = *out.last().unwrap_or(&0) as usize;
    if pad > 0 && pad <= 16 && out.len() >= pad && out[out.len() - pad..].iter().all(|b| *b as usize == pad) {
        out.truncate(out.len() - pad);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeySpec {
    uri: String,
    iv: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SegmentSpec {
    url: String,
    key: Option<KeySpec>,
    seq: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum ParsedPlaylist {
    Master { variant_url: String },
    Media {
        init_segment: Option<SegmentSpec>,
        segments: Vec<SegmentSpec>,
        ext: &'static str,
    },
}

fn parse_playlist(base_url: &str, text: &str) -> Result<ParsedPlaylist, String> {
    let body = text.trim_start_matches('\u{feff}');
    if !body.starts_with("#EXTM3U") {
        return Err("不是有效的 HLS 播放列表".into());
    }

    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

    // 主清单：挑选 BANDWIDTH 最高的子流
    let mut best_variant: Option<(u64, String)> = None;
    let mut pending_bandwidth: Option<u64> = None;
    for line in &lines {
        if line.starts_with("#EXT-X-STREAM-INF:") {
            let attrs = parse_attr_map(line);
            let bw = attrs
                .get("BANDWIDTH")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            pending_bandwidth = Some(bw);
        } else if let Some(bw) = pending_bandwidth.take() {
            if !line.starts_with('#') {
                let uri = absolutize(base_url, line);
                if best_variant.as_ref().is_none_or(|(best, _)| bw >= *best) {
                    best_variant = Some((bw, uri));
                }
            }
        }
    }
    if let Some((_, variant_url)) = best_variant {
        return Ok(ParsedPlaylist::Master { variant_url });
    }

    // 媒体清单
    let mut seq: u64 = 0;
    let mut current_key: Option<KeySpec> = None;
    let mut init_segment: Option<SegmentSpec> = None;
    let mut segments = Vec::new();
    let mut saw_mp4 = false;

    for line in &lines {
        if line.starts_with("#EXT-X-MEDIA-SEQUENCE:") {
            if let Some(val) = line.split_once(':').map(|(_, v)| v.trim()) {
                if let Ok(n) = val.parse::<u64>() {
                    seq = n;
                }
            }
        } else if line.starts_with("#EXT-X-KEY:") {
            let attrs = parse_attr_map(line);
            let method = attrs
                .get("METHOD")
                .map(|s| s.to_ascii_uppercase())
                .unwrap_or_else(|| "NONE".to_string());
            match method.as_str() {
                "NONE" => current_key = None,
                "AES-128" => {
                    let uri = attrs
                        .get("URI")
                        .filter(|u| !u.is_empty())
                        .ok_or_else(|| "HLS 加密密钥缺少 URI".to_string())?;
                    let iv = attrs.get("IV").and_then(|v| parse_hex_iv(v));
                    current_key = Some(KeySpec {
                        uri: absolutize(base_url, uri),
                        iv,
                    });
                }
                other => return Err(format!("暂不支持的 HLS 加密方式: {other}")),
            }
        } else if line.starts_with("#EXT-X-MAP:") {
            let attrs = parse_attr_map(line);
            if let Some(uri) = attrs.get("URI").filter(|u| !u.is_empty()) {
                saw_mp4 = true;
                init_segment = Some(SegmentSpec {
                    url: absolutize(base_url, uri),
                    key: current_key.clone(),
                    seq,
                });
            }
        } else if !line.starts_with('#') {
            let seg_url = absolutize(base_url, line);
            let path_lower = seg_url.split('?').next().unwrap_or(&seg_url).to_ascii_lowercase();
            if path_lower.ends_with(".m4s") || path_lower.ends_with(".mp4") {
                saw_mp4 = true;
            }
            segments.push(SegmentSpec {
                url: seg_url,
                key: current_key.clone(),
                seq,
            });
            seq = seq.saturating_add(1);
        }
    }

    if segments.is_empty() {
        return Err("播放列表中没有可下载的视频分片".into());
    }

    Ok(ParsedPlaylist::Media {
        init_segment,
        segments,
        ext: if saw_mp4 { "mp4" } else { "ts" },
    })
}

/* ───────────────────────── 任务状态与执行 ───────────────────────── */

fn emit_task(ctx: &AppContext, task: &DownloadTask) {
    ctx.emit(DOWNLOAD_EVENT, task);
}

fn update_task(ctx: &AppContext, id: &str, mutate: impl FnOnce(&mut DownloadTask)) {
    let snapshot = {
        let mut tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = tasks.iter_mut().find(|e| e.task.id == id) else {
            return;
        };
        mutate(&mut entry.task);
        entry.task.clone()
    };
    emit_task(ctx, &snapshot);
}

async fn fetch_with_retry(url: &str, cancel: &AtomicBool) -> Result<(Option<String>, Vec<u8>), String> {
    let mut last_err = String::new();
    for attempt in 0..SEGMENT_MAX_RETRIES {
        if cancel.load(Ordering::Acquire) {
            return Err("下载已取消".into());
        }
        match fetch_bytes(url).await {
            Ok(res) => return Ok(res),
            Err(err) => {
                last_err = err;
                if attempt + 1 < SEGMENT_MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(350 * (attempt as u64 + 1))).await;
                }
            }
        }
    }
    Err(last_err)
}

async fn fetch_and_decrypt(
    spec: SegmentSpec,
    key_cache: Arc<tokio::sync::Mutex<HashMap<String, Vec<u8>>>>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    let (_, raw) = fetch_with_retry(&spec.url, &cancel).await?;
    let Some(key_spec) = spec.key else {
        return Ok(raw);
    };
    let key_bytes = {
        let mut cache = key_cache.lock().await;
        if let Some(hit) = cache.get(&key_spec.uri) {
            hit.clone()
        } else {
            let (_, bytes) = fetch_with_retry(&key_spec.uri, &cancel).await?;
            cache.insert(key_spec.uri.clone(), bytes.clone());
            bytes
        }
    };
    let iv = key_spec.iv.unwrap_or_else(|| sequence_iv(spec.seq));
    aes128_cbc_decrypt(&raw, &key_bytes, iv)
}

enum DownloadTarget {
    DirectUrl { url: String },
    SeriesEpisode { video_id: i64, ep: u32, prefer_hevc: bool },
}

fn pick_stream_url(info: &serde_json::Value, prefer_hevc: bool) -> Option<String> {
    let main = info
        .get("streams")
        .and_then(|s| s.get("main"))
        .and_then(|v| v.as_str())
        .or_else(|| info.get("video_url").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let hevc = info
        .get("streams")
        .and_then(|s| s.get("hevc"))
        .and_then(|v| v.as_str())
        .or_else(|| info.get("video_url_hevc").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let chosen = if prefer_hevc { hevc.or(main) } else { main.or(hevc) };
    chosen.map(str::to_string)
}

fn format_episode_label(ep: u32, total_hint: usize) -> String {
    let width = if total_hint >= 100 { 3 } else { 2 };
    format!("第 {ep:0width$} 集")
}

async fn run_download(
    app: Arc<AppContext>,
    id: String,
    target: DownloadTarget,
    title: String,
    subtitle: String,
    folder: Option<String>,
    cancel: Arc<AtomicBool>,
) {
    // 限制同时执行的下载任务数，防止全集下载时瞬间并发打满上游与磁盘
    let _permit = match TASK_SEMAPHORE.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return,
    };
    if cancel.load(Ordering::Acquire) {
        update_task(&app, &id, |t| {
            t.status = "cancelled".into();
            t.error = None;
        });
        return;
    }

    let result = run_download_inner(&app, &id, target, &title, &subtitle, folder.as_deref(), &cancel).await;
    match result {
        Ok((file_path, bytes, total)) => {
            update_task(&app, &id, |t| {
                t.status = "done".into();
                t.file_path = file_path;
                t.bytes = bytes;
                t.done_segments = total;
                t.total_segments = total;
                t.error = None;
            });
        }
        Err(err) => {
            let cancelled = cancel.load(Ordering::Acquire);
            update_task(&app, &id, |t| {
                t.status = if cancelled { "cancelled".into() } else { "error".into() };
                t.error = if cancelled { None } else { Some(err) };
            });
        }
    }
}

async fn run_download_inner(
    app: &AppContext,
    id: &str,
    target: DownloadTarget,
    title: &str,
    subtitle: &str,
    folder: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<(String, u64, usize), String> {
    let mut dir = resolve_download_dir(app)?;
    if let Some(sub) = folder.filter(|s| !s.trim().is_empty()) {
        dir = dir.join(sanitize_filename(sub));
        fs::create_dir_all(&dir).map_err(|e| format!("创建剧集目录失败: {e}"))?;
    }
    let stem = if folder.is_some() && !subtitle.trim().is_empty() {
        sanitize_filename(subtitle)
    } else {
        build_file_stem(title, subtitle)
    };

    update_task(app, id, |t| {
        t.status = "downloading".into();
        t.error = None;
    });

    let url = match target {
        DownloadTarget::DirectUrl { url } => url,
        DownloadTarget::SeriesEpisode {
            video_id,
            ep,
            prefer_hevc,
        } => {
            if cancel.load(Ordering::Acquire) {
                return Err("下载已取消".into());
            }
            let info = crate::api::video::resolve_play_info(&app.api, video_id, ep).await?;
            let resolved = pick_stream_url(&info, prefer_hevc)
                .ok_or_else(|| format!("第 {ep} 集暂无可用播放地址"))?;
            crate::network_policy::parse_target(&resolved)?;
            update_task(app, id, |t| {
                t.url = resolved.clone();
            });
            resolved
        }
    };

    // 1. 拉取入口 URL：可能是主清单、媒体清单或直链 MP4
    let mut playlist_url = url;
    let (_, first_bytes) = fetch_with_retry(&playlist_url, cancel).await?;

    let is_m3u8 = first_bytes.starts_with(b"#EXTM3U") || first_bytes.starts_with(b"\xef\xbb\xbf#EXTM3U");
    if !is_m3u8 {
        // 直链媒体文件（如 MP4）
        let target = unique_target_path(&dir, &stem, "mp4");
        let part = target.with_extension("mp4.part");
        fs::write(&part, &first_bytes).map_err(|e| format!("写入文件失败: {e}"))?;
        if cancel.load(Ordering::Acquire) {
            let _ = fs::remove_file(&part);
            return Err("下载已取消".into());
        }
        fs::rename(&part, &target).map_err(|e| format!("保存文件失败: {e}"))?;
        return Ok((target.to_string_lossy().into_owned(), first_bytes.len() as u64, 1));
    }

    let mut text = std::str::from_utf8(&first_bytes)
        .map_err(|_| "播放列表编码无效".to_string())?
        .to_string();

    // 最多展开 2 层主清单，避免循环引用
    let (init_segment, segments, ext) = {
        let mut depth = 0;
        loop {
            match parse_playlist(&playlist_url, &text)? {
                ParsedPlaylist::Master { variant_url } => {
                    depth += 1;
                    if depth > 2 {
                        return Err("播放列表嵌套层级过多".into());
                    }
                    playlist_url = variant_url;
                    let (_, bytes) = fetch_with_retry(&playlist_url, cancel).await?;
                    text = std::str::from_utf8(&bytes)
                        .map_err(|_| "子播放列表编码无效".to_string())?
                        .to_string();
                }
                ParsedPlaylist::Media {
                    init_segment,
                    segments,
                    ext,
                } => break (init_segment, segments, ext),
            }
        }
    };

    let target = unique_target_path(&dir, &stem, ext);
    let part = dir.join(format!(
        "{}.{ext}.part",
        target.file_stem().and_then(|s| s.to_str()).unwrap_or(&stem)
    ));
    let total = segments.len();

    update_task(app, id, |t| {
        t.total_segments = total;
        t.file_path = target.to_string_lossy().into_owned();
    });

    let file = File::create(&part).map_err(|e| format!("创建临时文件失败: {e}"))?;
    let mut writer = BufWriter::new(file);
    let key_cache = Arc::new(tokio::sync::Mutex::new(HashMap::<String, Vec<u8>>::new()));
    let mut written_bytes: u64 = 0;

    // 如果存在 fMP4 初始化段 (#EXT-X-MAP)，先写入
    if let Some(init) = init_segment {
        match fetch_and_decrypt(init, key_cache.clone(), cancel.clone()).await {
            Ok(bytes) => {
                writer.write_all(&bytes).map_err(|e| format!("写入初始化段失败: {e}"))?;
                written_bytes += bytes.len() as u64;
            }
            Err(e) => {
                drop(writer);
                let _ = fs::remove_file(&part);
                return Err(e);
            }
        }
    }

    // 按批次并发拉取（默认 3 并发），按严格顺序写入文件
    let mut done = 0usize;
    for chunk in segments.chunks(MAX_CONCURRENT_SEGMENTS) {
        if cancel.load(Ordering::Acquire) {
            drop(writer);
            let _ = fs::remove_file(&part);
            return Err("下载已取消".into());
        }
        let mut handles = Vec::with_capacity(chunk.len());
        for spec in chunk {
            let spec = spec.clone();
            let cache = key_cache.clone();
            let flag = cancel.clone();
            handles.push(tokio::spawn(async move {
                fetch_and_decrypt(spec, cache, flag).await
            }));
        }
        for handle in handles {
            let seg_bytes = match handle.await {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => {
                    drop(writer);
                    let _ = fs::remove_file(&part);
                    return Err(e);
                }
                Err(e) => {
                    drop(writer);
                    let _ = fs::remove_file(&part);
                    return Err(format!("分片任务异常: {e}"));
                }
            };
            if let Err(e) = writer.write_all(&seg_bytes) {
                drop(writer);
                let _ = fs::remove_file(&part);
                return Err(format!("写入视频分片失败: {e}"));
            }
            written_bytes += seg_bytes.len() as u64;
            done += 1;
            update_task(app, id, |t| {
                t.done_segments = done;
                t.bytes = written_bytes;
            });
        }
    }

    if let Err(e) = writer.flush() {
        drop(writer);
        let _ = fs::remove_file(&part);
        return Err(format!("刷新文件缓冲失败: {e}"));
    }
    drop(writer);

    if cancel.load(Ordering::Acquire) {
        let _ = fs::remove_file(&part);
        return Err("下载已取消".into());
    }

    fs::rename(&part, &target).map_err(|e| {
        let _ = fs::remove_file(&part);
        format!("保存文件失败: {e}")
    })?;

    Ok((target.to_string_lossy().into_owned(), written_bytes, total))
}

/* ───────────────────────── Tauri 命令 ───────────────────────── */

fn push_task_entry(task: DownloadTask, cancel: Arc<AtomicBool>) {
    let mut tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
    tasks.insert(0, TaskEntry { task, cancel });
    while tasks.len() > MAX_TASKS_HISTORY {
        // 保留进行中的任务，裁剪最旧的已完成/失败记录
        if let Some(pos) = tasks
            .iter()
            .rposition(|e| !matches!(e.task.status.as_str(), "queued" | "downloading"))
        {
            tasks.remove(pos);
        } else {
            break;
        }
    }
}

pub async fn download_start(
    app: Arc<AppContext>,
    url: String,
    title: String,
    subtitle: Option<String>,
) -> Result<DownloadTask, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("视频地址为空，无法下载".into());
    }
    // 校验 URL 合法性与安全策略（拒绝非 http/https 或内网探测）
    crate::network_policy::parse_target(&url)?;

    let subtitle = subtitle.unwrap_or_default().trim().to_string();

    // 同一 URL 正在下载时直接返回现有任务，避免重复并发抓取
    {
        let tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = tasks
            .iter()
            .find(|e| e.task.url == url && matches!(e.task.status.as_str(), "queued" | "downloading"))
        {
            return Ok(existing.task.clone());
        }
    }

    let id = next_task_id();
    let task = DownloadTask {
        id: id.clone(),
        url: url.clone(),
        title: if title.trim().is_empty() { "未命名视频".into() } else { title.trim().to_string() },
        subtitle: subtitle.clone(),
        status: "queued".into(),
        done_segments: 0,
        total_segments: 0,
        bytes: 0,
        file_path: String::new(),
        error: None,
        created_at: now_secs(),
        video_id: None,
        episode: None,
        prefer_hevc: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    push_task_entry(task.clone(), cancel.clone());
    emit_task(&app, &task);

    let app_clone = app.clone();
    let task_id = id.clone();
    let title_clone = task.title.clone();
    tokio::spawn(async move {
        run_download(
            app_clone,
            task_id,
            DownloadTarget::DirectUrl { url },
            title_clone,
            subtitle,
            None,
            cancel,
        )
        .await;
    });

    Ok(task)
}

/// 全套剧集下载：将所选全部集数按顺序入队，保存到 `下载目录/HGhubX/<剧名>/第 XX 集.ts`，
/// 由后端信号量限流（最多同时下载 2 集）并在开跑前才按需解析该集播放地址，避免直链过期或打满上游。
pub async fn download_series(
    app: Arc<AppContext>,
    video_id: i64,
    title: String,
    episodes: Vec<u32>,
    prefer_hevc: Option<bool>,
) -> Result<Vec<DownloadTask>, String> {
    if video_id <= 0 {
        return Err("无效的剧集 ID".into());
    }
    let mut unique_eps = Vec::with_capacity(episodes.len());
    for ep in episodes {
        if ep > 0 && !unique_eps.contains(&ep) {
            unique_eps.push(ep);
        }
    }
    if unique_eps.is_empty() {
        return Err("没有可下载的剧集".into());
    }

    let prefer_hevc = prefer_hevc.unwrap_or(false);
    let series_title = if title.trim().is_empty() {
        format!("视频 {video_id}")
    } else {
        title.trim().to_string()
    };
    let total_hint = unique_eps.iter().copied().max().unwrap_or(1) as usize;
    let mut created = Vec::with_capacity(unique_eps.len());

    // 先按正序创建任务并入队（倒序插入列表头部或保持直观顺序）
    for ep in unique_eps {
        // 如果同一剧集的同一集已经在排队/下载中，直接复用，不重复入队
        let existing = {
            let tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
            tasks
                .iter()
                .find(|e| {
                    e.task.video_id == Some(video_id)
                        && e.task.episode == Some(ep)
                        && matches!(e.task.status.as_str(), "queued" | "downloading")
                })
                .map(|e| e.task.clone())
        };
        if let Some(task) = existing {
            created.push(task);
            continue;
        }

        let id = next_task_id();
        let ep_label = format_episode_label(ep, total_hint);
        let subtitle = if prefer_hevc {
            format!("{ep_label} HD")
        } else {
            ep_label
        };
        let task = DownloadTask {
            id: id.clone(),
            url: String::new(),
            title: series_title.clone(),
            subtitle: subtitle.clone(),
            status: "queued".into(),
            done_segments: 0,
            total_segments: 0,
            bytes: 0,
            file_path: String::new(),
            error: None,
            created_at: now_secs(),
            video_id: Some(video_id),
            episode: Some(ep),
            prefer_hevc: Some(prefer_hevc),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        push_task_entry(task.clone(), cancel.clone());
        emit_task(&app, &task);

        let app_clone = app.clone();
        let task_id = id.clone();
        let title_clone = series_title.clone();
        let folder = Some(series_title.clone());
        tokio::spawn(async move {
            run_download(
                app_clone,
                task_id,
                DownloadTarget::SeriesEpisode {
                    video_id,
                    ep,
                    prefer_hevc,
                },
                title_clone,
                subtitle,
                folder,
                cancel,
            )
            .await;
        });

        created.push(task);
    }

    Ok(created)
}

pub async fn download_cancel(app: &AppContext, id: String) -> Result<bool, String> {
    let snapshot = {
        let mut tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = tasks.iter_mut().find(|e| e.task.id == id) else {
            return Ok(false);
        };
        if !matches!(entry.task.status.as_str(), "queued" | "downloading") {
            return Ok(false);
        }
        entry.cancel.store(true, Ordering::Release);
        entry.task.status = "cancelled".into();
        entry.task.error = None;
        entry.task.clone()
    };
    emit_task(app, &snapshot);
    Ok(true)
}

pub async fn download_cancel_all(app: &AppContext) -> Result<usize, String> {
    let cancelled: Vec<DownloadTask> = {
        let mut tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for entry in tasks.iter_mut() {
            if matches!(entry.task.status.as_str(), "queued" | "downloading") {
                entry.cancel.store(true, Ordering::Release);
                entry.task.status = "cancelled".into();
                entry.task.error = None;
                out.push(entry.task.clone());
            }
        }
        out
    };
    for task in &cancelled {
        emit_task(app, task);
    }
    Ok(cancelled.len())
}

pub async fn download_list() -> Result<Vec<DownloadTask>, String> {
    let tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
    Ok(tasks.iter().map(|e| e.task.clone()).collect())
}

pub async fn download_remove(id: String) -> Result<bool, String> {
    let mut tasks = TASKS.lock().unwrap_or_else(|e| e.into_inner());
    let before = tasks.len();
    tasks.retain(|e| {
        if e.task.id == id {
            e.cancel.store(true, Ordering::Release);
            false
        } else {
            true
        }
    });
    Ok(tasks.len() < before)
}

pub async fn download_dir(app: &AppContext) -> Result<String, String> {
    let dir = resolve_download_dir(app)?;
    Ok(dir.to_string_lossy().into_owned())
}

pub async fn download_open_dir(app: &AppContext) -> Result<(), String> {
    let dir = resolve_download_dir(app)?;
    open::that_detached(&dir).map_err(|e| format!("打开下载目录失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;

    #[test]
    fn sanitize_and_stem() {
        assert_eq!(sanitize_filename("  测试:视频/第1集*?  "), "测试_视频_第1集");
        assert_eq!(sanitize_filename("...   "), "video");
        assert_eq!(build_file_stem("剧名", "第 2 集"), "剧名 - 第 2 集");
        assert_eq!(build_file_stem("剧名", ""), "剧名");
    }

    #[test]
    fn parse_master_picks_highest_bandwidth() {
        let playlist = "\
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
low/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1920x1080
high/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1200000,RESOLUTION=1280x720
mid/index.m3u8
";
        let parsed = parse_playlist("https://cdn.example.com/v/master.m3u8", playlist).unwrap();
        assert_eq!(
            parsed,
            ParsedPlaylist::Master {
                variant_url: "https://cdn.example.com/v/high/index.m3u8".into()
            }
        );
    }

    #[test]
    fn parse_media_playlist_with_aes128_and_map() {
        let playlist = "\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA-SEQUENCE:10
#EXT-X-KEY:METHOD=AES-128,URI=\"enc.key\",IV=0x0000000000000000000000000000000a
#EXT-X-MAP:URI=\"init.mp4\"
#EXTINF:4.0,
seg0.m4s
#EXT-X-KEY:METHOD=NONE
#EXTINF:4.0,
https://other.example.com/seg1.m4s
";
        let parsed = parse_playlist("https://cdn.example.com/hls/index.m3u8", playlist).unwrap();
        match parsed {
            ParsedPlaylist::Media {
                init_segment,
                segments,
                ext,
            } => {
                assert_eq!(ext, "mp4");
                assert_eq!(
                    init_segment.as_ref().map(|s| s.url.as_str()),
                    Some("https://cdn.example.com/hls/init.mp4")
                );
                assert_eq!(segments.len(), 2);
                assert_eq!(segments[0].url, "https://cdn.example.com/hls/seg0.m4s");
                assert_eq!(segments[0].seq, 10);
                assert_eq!(
                    segments[0].key.as_ref().map(|k| k.uri.as_str()),
                    Some("https://cdn.example.com/hls/enc.key")
                );
                assert_eq!(segments[0].key.as_ref().and_then(|k| k.iv), Some(sequence_iv(10)));
                assert_eq!(segments[1].url, "https://other.example.com/seg1.m4s");
                assert_eq!(segments[1].seq, 11);
                assert!(segments[1].key.is_none());
            }
            _ => panic!("expected media playlist"),
        }
    }

    #[test]
    fn aes128_cbc_roundtrip() {
        let key = *b"0123456789abcdef";
        let iv = sequence_iv(42);
        let plaintext = b"hello hls segment payload!";
        // PKCS#7 pad
        let pad_len = 16 - (plaintext.len() % 16);
        let mut padded = plaintext.to_vec();
        padded.extend(std::iter::repeat(pad_len as u8).take(pad_len));

        let cipher = Aes128::new_from_slice(&key).unwrap();
        let mut prev = iv;
        let mut ciphertext = Vec::with_capacity(padded.len());
        for chunk in padded.chunks(16) {
            let mut block = [0u8; 16];
            for i in 0..16 {
                block[i] = chunk[i] ^ prev[i];
            }
            cipher.encrypt_block((&mut block).into());
            ciphertext.extend_from_slice(&block);
            prev = block;
        }

        let decrypted = aes128_cbc_decrypt(&ciphertext, &key, iv).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn stream_selection_and_episode_label() {
        assert_eq!(format_episode_label(1, 20), "第 01 集");
        assert_eq!(format_episode_label(12, 120), "第 012 集");

        let info = serde_json::json!({
            "streams": {
                "main": "https://cdn.example.com/main.m3u8",
                "hevc": "https://cdn.example.com/hevc.m3u8"
            }
        });
        assert_eq!(
            pick_stream_url(&info, false).as_deref(),
            Some("https://cdn.example.com/main.m3u8")
        );
        assert_eq!(
            pick_stream_url(&info, true).as_deref(),
            Some("https://cdn.example.com/hevc.m3u8")
        );

        let fallback = serde_json::json!({ "video_url": "https://cdn.example.com/legacy.m3u8" });
        assert_eq!(
            pick_stream_url(&fallback, true).as_deref(),
            Some("https://cdn.example.com/legacy.m3u8")
        );
    }
}
