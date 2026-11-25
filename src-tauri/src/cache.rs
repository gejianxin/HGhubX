//! Local cache maintenance (in-memory media caches).

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheClearReport {
    /// Whether a platform WebView HTTP/media cache was selectively cleared.
    pub web_cache: bool,
    pub platform: &'static str,
}

pub async fn clear_web_cache() -> Result<CacheClearReport, String> {
    Ok(CacheClearReport {
        web_cache: false,
        platform: std::env::consts::OS,
    })
}

#[cfg(test)]
mod tests {
    use super::CacheClearReport;

    #[test]
    fn report_matches_frontend_camel_case_contract() {
        let value = serde_json::to_value(CacheClearReport {
            web_cache: true,
            platform: "linux",
        })
        .unwrap();
        assert_eq!(value["webCache"], true);
        assert_eq!(value["platform"], "linux");
    }
}
