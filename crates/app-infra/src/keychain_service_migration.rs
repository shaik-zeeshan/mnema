//! One-shot read-through migration for the `com.shaikzeeshan.mnema.*` →
//! `day.mnema.*` keychain service rename: read the new service first, fall
//! back to the legacy one, and move a legacy value over gated on a proven
//! write. A denied/failed read on either service propagates as `Err` so
//! callers never mint a replacement secret over one that still exists.

use crate::error::Result;

/// Resolve a value across the new and legacy keychain services.
///
/// Ordering contract (same proof-gating as the capture-index migration):
/// the legacy item is deleted only after `store_new` succeeded, and a failed
/// delete is ignored — both items remaining is safe and idempotent, the next
/// read short-circuits on the new service.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn read_with_legacy_migration<T>(
    read_new: impl FnOnce() -> Result<Option<T>>,
    read_legacy: impl FnOnce() -> Result<Option<T>>,
    store_new: impl FnOnce(&T) -> Result<()>,
    delete_legacy: impl FnOnce(),
) -> Result<Option<T>> {
    if let Some(value) = read_new()? {
        return Ok(Some(value));
    }
    let Some(value) = read_legacy()? else {
        return Ok(None);
    };
    store_new(&value)?;
    delete_legacy();
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::error::AppInfraError;

    fn denied() -> AppInfraError {
        AppInfraError::LicenseTokenStore("keychain read denied".to_string())
    }

    #[test]
    fn new_present_short_circuits_without_touching_legacy() {
        let legacy_reads = Cell::new(0u32);
        let value = read_with_legacy_migration(
            || Ok(Some("new-value")),
            || {
                legacy_reads.set(legacy_reads.get() + 1);
                Ok(Some("legacy-value"))
            },
            |_| panic!("must not store"),
            || panic!("must not delete"),
        )
        .expect("read should succeed");

        assert_eq!(value, Some("new-value"));
        assert_eq!(legacy_reads.get(), 0);
    }

    #[test]
    fn legacy_present_migrates_store_before_delete() {
        let stored = Cell::new(false);
        let deleted = Cell::new(false);
        let value = read_with_legacy_migration(
            || Ok(None),
            || Ok(Some("legacy-value")),
            |value| {
                assert_eq!(*value, "legacy-value");
                assert!(!deleted.get(), "store must precede delete");
                stored.set(true);
                Ok(())
            },
            || {
                assert!(stored.get(), "delete only after a proven store");
                deleted.set(true);
            },
        )
        .expect("migration should succeed");

        assert_eq!(value, Some("legacy-value"));
        assert!(stored.get());
        assert!(deleted.get());
    }

    #[test]
    fn denied_legacy_read_propagates_and_never_stores() {
        let result = read_with_legacy_migration::<&str>(
            || Ok(None),
            || Err(denied()),
            |_| panic!("a denied read must never mint a replacement"),
            || panic!("must not delete"),
        );

        assert!(result.is_err());
    }

    #[test]
    fn failed_store_keeps_legacy_item() {
        let result = read_with_legacy_migration(
            || Ok(None),
            || Ok(Some("legacy-value")),
            |_| Err(denied()),
            || panic!("an unproven store must never delete the legacy item"),
        );

        assert!(result.is_err());
    }

    #[test]
    fn both_absent_resolves_none_without_writes() {
        let value = read_with_legacy_migration::<&str>(
            || Ok(None),
            || Ok(None),
            |_| panic!("must not store"),
            || panic!("must not delete"),
        )
        .expect("read should succeed");

        assert_eq!(value, None);
    }
}
