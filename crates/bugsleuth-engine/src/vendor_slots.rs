//! Coding CLIs share mutable sessions, including across repository reviews.
use crate::sweep::Vendor;
use tokio::sync::{Mutex, MutexGuard};

static CLAUDE: Mutex<()> = Mutex::const_new(());
static CODEX: Mutex<()> = Mutex::const_new(());
static CURSOR: Mutex<()> = Mutex::const_new(());
static OPENCODE: Mutex<()> = Mutex::const_new(());

pub(crate) async fn acquire(vendor: Vendor) -> MutexGuard<'static, ()> {
    match vendor {
        Vendor::Claude => &CLAUDE,
        Vendor::Codex => &CODEX,
        Vendor::Cursor => &CURSOR,
        Vendor::OpenCode => &OPENCODE,
    }
    .lock()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn different_vendors_overlap_but_same_vendor_waits_and_cancellation_releases() {
        let first = acquire(Vendor::Cursor).await;
        let other = acquire(Vendor::OpenCode).await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                acquire(Vendor::Cursor)
            )
            .await
            .is_err()
        );
        drop(first);
        let next =
            tokio::time::timeout(std::time::Duration::from_secs(1), acquire(Vendor::Cursor)).await;
        assert!(next.is_ok());
        drop(other);
    }
}
