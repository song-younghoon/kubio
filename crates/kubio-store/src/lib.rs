//! Cache store abstractions, memory store, and process-local disk store.

mod disk;
mod entry;
mod error;
mod memory;
mod metadata;
mod metrics;
mod purge;
mod store;

pub use disk::DiskStore;
pub use entry::CacheEntry;
pub use error::StoreError;
pub use memory::MemoryStore;
pub use metrics::{StoreOperationMetrics, StoreOperationStats, StoreStats};
pub use purge::{PurgeResult, PurgeSelector};
pub use store::{CacheStore, StoreKind};
