mod api;
mod app_context;
mod apple_proxy;
mod cache;
mod channel_page;
mod download;
mod edge_crypto;
mod env;
mod lines;
mod logging;
mod media;
mod media_dns;
mod net;
mod net_diag;
mod network_policy;
mod playback_log;
mod proxy;
mod publish_parser;
mod search_hot;
mod session;
mod updater;

use api::Api;
use app_context::{default_data_dir, default_download_dir, init_tls, AppContext};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::Embed;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

const DEFAULT_PORT: u16 = 17650;

#[derive(Embed)]
#[folder = "../dist"]
struct FrontendAssets;

#[derive(Clone)]
struct ServerState {
    ctx: Arc<AppContext>,
    media_origin: String,
}

fn arg<T: DeserializeOwned>(body: &Value, camel: &str, snake: &str) -> Result<T, String> {
    let val = body
        .get(camel)
        .or_else(|| body.get(snake))
        .cloned()
        .ok_or_else(|| format!("缺少参数: {camel}"))?;
    serde_json::from_value(val).map_err(|e| format!("参数 {camel} 无效: {e}"))
}

fn opt_arg<T: DeserializeOwned>(body: &Value, camel: &str, snake: &str) -> Result<Option<T>, String> {
    match body.get(camel).or_else(|| body.get(snake)) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("参数 {camel} 无效: {e}")),
    }
}

fn to_json<T: serde::Serialize>(res: Result<T, String>) -> Result<Value, String> {
    res.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

async fn dispatch_command(ctx: Arc<AppContext>, cmd: &str, body: Value) -> Result<Value, String> {
    let api = ctx.api.as_ref();
    match cmd {
        "app_boot" => api::platform::app_boot(api).await,
        "media_http_origin" => Err("纯 Rust HTTP 模式下媒体已同源代理".into()),
        "playback_log_status" => to_json(playback_log::playback_log_status()),
        "playback_log_set" => to_json(playback_log::playback_log_set(arg(&body, "enabled", "enabled")?)),
        "playback_log_write" => to_json(playback_log::playback_log_write(arg(&body, "entries", "entries")?)),
        "playback_log_read" => to_json(playback_log::playback_log_read()),
        "security_set" => to_json(api::platform::security_set(arg(&body, "strict", "strict")?).await),
        "debug_log" => to_json(api::platform::debug_log(&ctx, arg(&body, "msg", "msg")?).await),
        "net_diag" => api::platform::net_diag().await,
        "line_state" => api::lines::line_state().await,
        "line_set" => to_json(api::lines::line_set(api, arg(&body, "url", "url")?).await),
        "line_custom_set" => api::lines::line_custom_set(api, arg(&body, "url", "url")?).await,
        "line_reset" => api::lines::line_reset(api).await,
        "line_cancel" => to_json(api::lines::line_cancel().await),
        "line_mirrors" => api::lines::line_mirrors().await,
        "line_probe" => api::lines::line_probe(arg(&body, "urls", "urls")?).await,
        "line_auto" => api::lines::line_auto(api).await,
        "line_try_main" => api::lines::line_try_main(api).await,
        "line_health" => api::lines::line_health().await,
        "proxy_state" => api::lines::proxy_state().await,
        "proxy_set" => api::lines::proxy_set(arg(&body, "mode", "mode")?, arg(&body, "url", "url")?).await,
        "proxy_test" => api::lines::proxy_test(arg(&body, "mode", "mode")?, arg(&body, "url", "url")?).await,
        "check_update" => {
            api::update::check_update(
                opt_arg(&body, "edition", "edition")?,
                opt_arg(&body, "currentVersion", "current_version")?,
            )
            .await
        }
        "download_tv_update" => {
            api::update::download_tv_update(
                &ctx,
                arg(&body, "url", "url")?,
                opt_arg(&body, "size", "size")?,
                opt_arg(&body, "sha256", "sha256")?,
            )
            .await
        }
        "cancel_tv_update" => {
            api::update::cancel_tv_update();
            Ok(Value::Null)
        }
        "open_url" => to_json(api::update::open_url(arg(&body, "url", "url")?).await),
        "set_orientation" => to_json(api::platform::set_orientation(arg(&body, "landscape", "landscape")?).await),
        "set_text_interaction" => to_json(api::platform::set_text_interaction(arg(&body, "enabled", "enabled")?)),
        "api_login" => {
            api::account::api_login(
                api,
                arg(&body, "username", "username")?,
                arg(&body, "password", "password")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
                opt_arg(&body, "credentialOrigin", "credential_origin")?,
            )
            .await
        }
        "api_register" => {
            api::account::api_register(
                api,
                arg(&body, "username", "username")?,
                arg(&body, "password", "password")?,
                arg(&body, "email", "email")?,
            )
            .await
        }
        "api_logout" => {
            api::account::api_logout(
                api,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_me" => {
            api::account::api_me(
                api,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_account_history" => {
            api::account::api_account_history(
                api,
                arg(&body, "expectedUserId", "expected_user_id")?,
                arg(&body, "expectedOrigin", "expected_origin")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_account_history_record" => {
            api::account::api_account_history_record(
                api,
                arg(&body, "expectedUserId", "expected_user_id")?,
                arg(&body, "expectedOrigin", "expected_origin")?,
                arg(&body, "videoId", "video_id")?,
                arg(&body, "ep", "ep")?,
            )
            .await
        }
        "api_account_history_delete" => {
            api::account::api_account_history_delete(
                api,
                arg(&body, "expectedUserId", "expected_user_id")?,
                arg(&body, "expectedOrigin", "expected_origin")?,
                arg(&body, "ids", "ids")?,
            )
            .await
        }
        "api_feed" => api::feed::api_feed(api, arg(&body, "page", "page")?, arg(&body, "size", "size")?).await,
        "api_original_videos" => {
            api::feed::api_original_videos(
                api,
                arg(&body, "cate", "cate")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "sort", "sort")?,
                opt_arg(&body, "jsonOnly", "json_only")?,
            )
            .await
        }
        "api_topics" => api::feed::api_topics(api).await,
        "api_topic" => {
            api::feed::api_topic(
                api,
                arg(&body, "slug", "slug")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_tags" => api::feed::api_tags(api).await,
        "api_hot_tags" => api::feed::api_hot_tags(api).await,
        "api_tag_videos" => {
            api::feed::api_tag_videos(
                api,
                arg(&body, "tag", "tag")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "sort", "sort")?,
            )
            .await
        }
        "api_post_tag" => {
            api::feed::api_post_tag(
                api,
                arg(&body, "tag", "tag")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_category" => {
            api::feed::api_category(
                api,
                arg(&body, "cate", "cate")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "sort", "sort")?,
                opt_arg(&body, "jsonOnly", "json_only")?,
            )
            .await
        }
        "api_rank" => api::feed::api_rank(api, arg(&body, "kind", "kind")?).await,
        "api_search" => {
            api::feed::api_search(
                api,
                arg(&body, "q", "q")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "kind", "kind")?,
            )
            .await
        }
        "api_search_hot" => api::feed::api_search_hot(api).await,
        "cache_clear" => to_json(api::platform::cache_clear().await),
        "api_posts" => {
            api::feed::api_posts(
                api,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "sort", "sort")?,
            )
            .await
        }
        "api_post" => api::feed::api_post(api, arg(&body, "id", "id")?).await,
        "api_post_playback" => api::feed::api_post_playback(api, arg(&body, "id", "id")?).await,
        "api_post_like" => {
            api::interact::api_post_like(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "on", "on")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_post_collect" => {
            api::interact::api_post_collect(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "on", "on")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_post_comments" => {
            api::interact::api_post_comments(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_post_categories" => api::feed::api_post_categories(api).await,
        "api_post_category" => {
            api::feed::api_post_category(
                api,
                arg(&body, "route", "route")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
                opt_arg(&body, "sort", "sort")?,
            )
            .await
        }
        "api_related" => api::feed::api_related(api, arg(&body, "id", "id")?).await,
        "api_following" => api::feed::api_following(api, arg(&body, "page", "page")?, arg(&body, "size", "size")?).await,
        "api_user" => api::feed::api_user(api, arg(&body, "id", "id")?).await,
        "api_user_stats" => api::feed::api_user_stats(api, arg(&body, "id", "id")?).await,
        "api_user_videos" => {
            api::feed::api_user_videos(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_video" => api::video::api_video(api, arg(&body, "id", "id")?).await,
        "api_play" => api::video::api_play(api, arg(&body, "id", "id")?, arg(&body, "ep", "ep")?).await,
        "api_comments" => {
            api::video::api_comments(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "ep", "ep")?,
                arg(&body, "page", "page")?,
                arg(&body, "size", "size")?,
            )
            .await
        }
        "api_comment_send" => {
            api::video::api_comment_send(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "ep", "ep")?,
                arg(&body, "text", "text")?,
                opt_arg(&body, "replyId", "reply_id")?,
            )
            .await
        }
        "api_like" => {
            api::interact::api_like(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "ep", "ep")?,
                arg(&body, "on", "on")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_collect" => {
            api::interact::api_collect(
                api,
                arg(&body, "id", "id")?,
                arg(&body, "on", "on")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "api_follow" => {
            api::interact::api_follow(
                api,
                arg(&body, "uid", "uid")?,
                arg(&body, "on", "on")?,
                opt_arg(&body, "expectedUserId", "expected_user_id")?,
                opt_arg(&body, "expectedOrigin", "expected_origin")?,
            )
            .await
        }
        "download_start" => to_json(
            download::download_start(
                ctx,
                arg(&body, "url", "url")?,
                arg(&body, "title", "title")?,
                opt_arg(&body, "subtitle", "subtitle")?,
            )
            .await,
        ),
        "download_series" => to_json(
            download::download_series(
                ctx,
                arg(&body, "videoId", "video_id")?,
                arg(&body, "title", "title")?,
                arg(&body, "episodes", "episodes")?,
                opt_arg(&body, "preferHevc", "prefer_hevc")?,
            )
            .await,
        ),
        "download_cancel" => to_json(download::download_cancel(&ctx, arg(&body, "id", "id")?).await),
        "download_cancel_all" => to_json(download::download_cancel_all(&ctx).await),
        "download_list" => to_json(download::download_list().await),
        "download_remove" => to_json(download::download_remove(arg(&body, "id", "id")?).await),
        "download_dir" => to_json(download::download_dir(&ctx).await),
        "download_open_dir" => to_json(download::download_open_dir(&ctx).await),
        "app_exit" => {
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                std::process::exit(0);
            });
            Ok(Value::Null)
        }
        _ => Err(format!("未知命令: {cmd}")),
    }
}

async fn handle_invoke(
    State(state): State<ServerState>,
    Path(cmd): Path<String>,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let payload = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    match dispatch_command(state.ctx.clone(), &cmd, payload).await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(err) => (StatusCode::BAD_REQUEST, Json(json!({ "error": err }))),
    }
}

async fn handle_events(
    State(state): State<ServerState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.ctx.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| match item {
        Ok(ev) => {
            let data = serde_json::to_string(&ev.payload).unwrap_or_else(|_| "null".into());
            Some(Ok(Event::default().event(ev.event).data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn handle_media(
    State(state): State<ServerState>,
    Path(subpath): Path<String>,
    method: Method,
    headers: HeaderMap,
    Query(query_map): Query<HashMap<String, String>>,
) -> Response<Body> {
    let path = format!("/{subpath}");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query_map.iter())
        .finish();
    let range = if method == Method::GET
        && !headers.contains_key("If-Range")
        && headers.get_all("Range").iter().count() == 1
    {
        headers.get("Range").and_then(|v| v.to_str().ok()).map(str::to_owned)
    } else {
        None
    };

    let media_resp = media::handle(&path, &query, &state.media_origin, range.as_deref()).await;
    let (parts, bytes) = media_resp.into_parts();
    let mut builder = Response::builder().status(parts.status);
    for (k, v) in parts.headers.iter() {
        builder = builder.header(k, v);
    }
    builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::empty()).unwrap())
}

fn serve_asset(path: &str) -> Response<Body> {
    let trimmed = path.trim_start_matches('/');
    let key = if trimmed.is_empty() { "index.html" } else { trimmed };
    let asset = FrontendAssets::get(key).or_else(|| FrontendAssets::get("index.html"));
    let Some(file) = asset else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Frontend assets not embedded. Run `pnpm build` first."))
            .unwrap();
    };
    let mime = mime_guess::from_path(if FrontendAssets::get(key).is_some() { key } else { "index.html" })
        .first_or_octet_stream();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")),
        );
    if key.starts_with("assets/") {
        builder = builder.header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    builder.body(Body::from(file.data.into_owned())).unwrap()
}

async fn handle_index() -> Response<Body> {
    serve_asset("index.html")
}

async fn handle_static(Path(path): Path<String>) -> Response<Body> {
    serve_asset(&path)
}

fn parse_cli() -> (Option<u16>, bool) {
    let mut port: Option<u16> = std::env::var("HGHUBX_PORT")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok());
    let mut no_open = std::env::var("HGHUBX_NO_OPEN")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"));

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--no-open" {
            no_open = true;
        } else if arg == "--port" {
            if let Some(p) = args.next().and_then(|v| v.parse::<u16>().ok()) {
                port = Some(p);
            }
        } else if let Some(rest) = arg.strip_prefix("--port=") {
            if let Ok(p) = rest.parse::<u16>() {
                port = Some(p);
            }
        }
    }
    (port, no_open)
}

pub fn run() {
    init_tls();
    let (requested_port, no_open) = parse_cli();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    rt.block_on(async move {
        let data_dir = default_data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        net::bind_base(&data_dir);
        playback_log::bind(&data_dir);

        let api = Arc::new(Api::new());
        api.session.bind(data_dir.clone());
        let download_dir = default_download_dir(&data_dir);
        let ctx = Arc::new(AppContext::new(api, data_dir, download_dir));

        let listener = match requested_port {
            Some(p) => tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], p)))
                .await
                .unwrap_or_else(|e| panic!("无法监听 127.0.0.1:{p}: {e}")),
            None => match tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT))).await {
                Ok(l) => l,
                Err(_) => tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
                    .await
                    .expect("无法绑定本地回环端口"),
            },
        };
        let addr = listener.local_addr().expect("无法获取监听地址");
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let state = ServerState {
            ctx,
            media_origin: format!("{base_url}/hg"),
        };

        let app = Router::new()
            .route("/api/invoke/{cmd}", post(handle_invoke))
            .route("/api/events", get(handle_events))
            .route("/hg/{*subpath}", get(handle_media))
            .route("/", get(handle_index))
            .route("/{*path}", get(handle_static))
            .with_state(state);

        println!("HGhubX v{} running at {base_url}", env!("CARGO_PKG_VERSION"));
        if !no_open {
            let _ = open::that_detached(&base_url);
        }

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .expect("HTTP server error");
    });
}
