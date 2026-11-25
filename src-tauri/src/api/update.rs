//! Update check, TV package download, and external link opening commands.

use crate::app_context::AppContext;
use serde_json::Value;

/// 对比 GitHub Release 的 tag 与当前版本（支持标准版与 TV 独立版本通道）。
pub async fn check_update(edition: Option<String>, current_version: Option<String>) -> Result<Value, String> {
    crate::updater::check_for_edition(edition.as_deref(), current_version.as_deref()).await
}

/// 下载并校验 Android TV 更新包（内置精简加速前缀自动回退 + SHA-256 校验）。
pub async fn download_tv_update(ctx: &AppContext, url: String, size: Option<u64>, sha256: Option<String>) -> Result<Value, String> {
    crate::updater::download_tv_update(ctx, &url, size, sha256.as_deref()).await
}

/// 取消正在进行的 Android TV 更新包下载。
pub fn cancel_tv_update() {
    crate::updater::cancel_tv_update();
}

/// 用系统默认浏览器打开链接（只允许 http/https）。
pub async fn open_url(url: String) -> Result<(), String> {
    let target = crate::network_policy::parse_target(&url)?;
    open::that_detached(target.as_str()).map_err(|e| e.to_string())
}
