//! An exclusively owned temporary directory, also providing a unique agent ID.
use super::{ProviderError, VENDOR};
use std::path::PathBuf;

pub(super) struct Session {
    pub(super) dir: PathBuf,
    pub(super) agent: String,
}
impl Session {
    pub(super) async fn new() -> Result<Self, ProviderError> {
        tokio::task::spawn_blocking(|| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let agent = format!(
                "bugsleuth-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(&agent);
            let builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            builder.create(&dir).map_err(|e| scratch(e.to_string()))?;
            Ok(Self { dir, agent })
        })
        .await
        .map_err(|e| scratch(e.to_string()))?
    }
}
fn scratch(detail: String) -> ProviderError {
    ProviderError::Scratch {
        vendor: VENDOR,
        detail,
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("OpenCode temporary directory cleanup failed: {error}");
        }
    }
}
