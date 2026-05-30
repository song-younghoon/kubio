use crate::{CacheControlClass, ContentTypeClass, VaryClass};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSignals {
    pub method_cacheable: bool,
    pub has_authorization: bool,
    pub has_cookie: bool,
    pub has_range: bool,
    pub has_body_on_get_or_head: bool,
    pub query_param_count: u16,
    pub sensitive_path_score: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseSignals {
    pub status_cacheable: bool,
    pub has_set_cookie: bool,
    pub cache_control: CacheControlClass,
    pub vary: VaryClass,
    pub content_length: Option<u64>,
    pub content_type_class: ContentTypeClass,
}
