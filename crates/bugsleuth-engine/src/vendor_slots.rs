//! Coding CLIs share mutable sessions, including across repository reviews.
use crate::sweep::Vendor;
use tokio::sync::{Mutex, MutexGuard};

static CODEX_APPLIES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

/// Codex applies use ephemeral sessions and private answer files. Bound their
/// concurrency without serializing independent repositories behind a vendor lock.
pub(crate) async fn acquire_apply(
    vendor: Vendor,
) -> Result<
    (
        Option<MutexGuard<'static, ()>>,
        Option<tokio::sync::SemaphorePermit<'static>>,
    ),
    tokio::sync::AcquireError,
> {
    if matches!(vendor, Vendor::Codex) {
        Ok((None, Some(CODEX_APPLIES.acquire().await?)))
    } else {
        Ok((Some(acquire(vendor).await), None))
    }
}

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
    async fn codex_applies_overlap_with_a_bound_and_release_cancelled_waiters() {
        let (first, second, third) =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                tokio::join!(
                    acquire_apply(Vendor::Codex),
                    acquire_apply(Vendor::Codex),
                    acquire_apply(Vendor::Codex)
                )
            })
            .await
            .expect("three Codex applies must acquire slots together");
        let (first, second, third) = (first.unwrap(), second.unwrap(), third.unwrap());
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                acquire_apply(Vendor::Codex)
            )
            .await
            .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                acquire_apply(Vendor::Codex)
            )
            .await
            .is_ok()
        );
        drop((second, third));
    }

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
