//! Process commands: logging, diagnostics, orientation, boot, security, cache.

use super::Api;
use crate::app_context::AppContext;
use crate::net::base;
use serde_json::{json, Value};

/// API/运行错误与自检统一写入 debug.log，每行附 RFC 3339 UTC 时间和进程编号。
pub async fn debug_log(ctx: &AppContext, msg: String) -> Result<(), String> {
    crate::logging::append_debug(&ctx.data_dir, &msg)
}

/// 网络自检：系统代理 / DNS / 连通性快照，用于远程定位「打不开」。
pub async fn net_diag() -> Result<Value, String> {
    Ok(crate::net::diagnose().await)
}

pub async fn set_orientation(_landscape: bool) -> Result<(), String> {
    Ok(())
}

pub fn set_text_interaction(_enabled: bool) -> Result<(), String> {
    Ok(())
}

/* ───────────────────────── 会话 ───────────────────────── */

pub async fn app_boot(state: &Api) -> Result<Value, String> {
    let origin = base();
    if crate::network_policy::strict_security() {
        state.session.reconcile_on_boot(&origin).await;
    }
    let origin = base();
    let credentials_allowed = crate::session::boot_credentials_visible(
        crate::network_policy::strict_security(),
        crate::net::trusted_auth_origin(&origin),
        state.session.token(&origin).is_some(),
    );
    Ok(json!({
        "user": if credentials_allowed { state.session.user(&origin) } else { Value::Null },
        "logged_in": credentials_allowed && state.session.token(&origin).is_some(),
        "base": origin,
        "strict_security": crate::network_policy::strict_security(),
        "cli_relaxed": crate::network_policy::cli_strict_override() == Some(false),
    }))
}

/// 切换严格安全模式（关闭后回退到兼容清洗与网络守卫模式，立即生效并落盘）。
pub async fn security_set(strict: bool) -> Result<bool, String> {
    crate::network_policy::set_strict_security(strict)?;
    crate::net::refresh_client();
    crate::playback_log::record(
        "security.mode_changed",
        json!({"strict": crate::network_policy::strict_security()}),
    );
    Ok(crate::network_policy::strict_security())
}

/* ───────────────────────── 本地缓存 ───────────────────────── */

pub async fn cache_clear() -> Result<crate::cache::CacheClearReport, String> {
    crate::media::clear_caches();
    crate::cache::clear_web_cache().await
}
