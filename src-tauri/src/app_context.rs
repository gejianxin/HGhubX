use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Once};
use tokio::sync::broadcast;

static TLS_INIT: Once = Once::new();

/// Install the pure-Rust RustCrypto TLS provider once per process.
pub fn init_tls() {
    TLS_INIT.call_once(|| {
        let _ = rustls::crypto::CryptoProvider::install_default(rustls_rustcrypto::provider());
    });
}

#[derive(Clone, Debug, Serialize)]
pub struct ServerEvent {
    pub event: String,
    pub payload: Value,
}

pub struct AppContext {
    pub api: Arc<crate::api::Api>,
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub events: broadcast::Sender<ServerEvent>,
}

impl AppContext {
    pub fn new(api: Arc<crate::api::Api>, data_dir: PathBuf, download_dir: PathBuf) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            api,
            data_dir,
            download_dir,
            events,
        }
    }

    pub fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        if let Ok(value) = serde_json::to_value(payload) {
            let _ = self.events.send(ServerEvent {
                event: event.to_string(),
                payload: value,
            });
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("com.hghubx.app")
}

pub fn default_download_dir(data_dir: &std::path::Path) -> PathBuf {
    let base = dirs::download_dir().unwrap_or_else(|| data_dir.to_path_buf());
    base.join("HGhubX")
}
