use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("cache object is too large: {size} > {max}")]
    ObjectTooLarge { size: u64, max: u64 },
    #[error("store error: {0}")]
    Other(String),
}
