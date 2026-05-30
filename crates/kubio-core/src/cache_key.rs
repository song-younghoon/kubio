use crate::{normalize_query_with_config_and_verified_ignores, short_hash, RouteQueryConfig};
use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKeyHash(pub String);

impl Display for CacheKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub method: String,
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub normalized_query: String,
    pub vary_headers: Vec<(String, String)>,
}

impl CacheKey {
    pub fn hash(&self) -> CacheKeyHash {
        let mut material = String::new();
        material.push_str(&self.method);
        material.push('\n');
        material.push_str(&self.scheme);
        material.push('\n');
        material.push_str(&self.authority);
        material.push('\n');
        material.push_str(&self.path);
        material.push('\n');
        material.push_str(&self.normalized_query);
        material.push('\n');
        for (name, value) in &self.vary_headers {
            material.push_str(name);
            material.push('=');
            material.push_str(value);
            material.push('\n');
        }
        CacheKeyHash(short_hash(&material))
    }
}

pub fn build_cache_key(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: &[&str],
) -> CacheKey {
    build_cache_key_with_query_config(
        method,
        scheme,
        authority,
        path,
        query,
        request_headers,
        vary_names,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_cache_key_with_query_config(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: &[&str],
    query_config: Option<&RouteQueryConfig>,
) -> CacheKey {
    build_cache_key_with_query_names(
        method,
        scheme,
        authority,
        path,
        query,
        request_headers,
        vary_names.iter().copied(),
        query_config,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_cache_key_with_query_names<'a, I>(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: I,
    query_config: Option<&RouteQueryConfig>,
) -> CacheKey
where
    I: IntoIterator<Item = &'a str>,
{
    build_cache_key_with_query_names_and_verified_ignores(
        method,
        scheme,
        authority,
        path,
        query,
        request_headers,
        vary_names,
        query_config,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_cache_key_with_query_names_and_verified_ignores<'a, I>(
    method: &Method,
    scheme: &str,
    authority: &str,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    vary_names: I,
    query_config: Option<&RouteQueryConfig>,
    verified_ignores: &[String],
) -> CacheKey
where
    I: IntoIterator<Item = &'a str>,
{
    let mut vary_headers = vary_names
        .into_iter()
        .map(|name| {
            let value = request_headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
            (name.to_ascii_lowercase(), value)
        })
        .collect::<Vec<_>>();
    vary_headers.sort_by(|left, right| left.0.cmp(&right.0));

    CacheKey {
        method: method.as_str().to_string(),
        scheme: scheme.to_string(),
        authority: authority.to_string(),
        path: path.to_string(),
        normalized_query: query
            .map(|query| {
                normalize_query_with_config_and_verified_ignores(
                    query,
                    query_config,
                    verified_ignores,
                )
            })
            .unwrap_or_default(),
        vary_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_hash_changes_with_vary_header_values() {
        let mut first = HeaderMap::new();
        first.insert("accept-language", "en".parse().unwrap());
        let mut second = HeaderMap::new();
        second.insert("accept-language", "ko".parse().unwrap());

        let method = Method::GET;
        let first_key = build_cache_key(
            &method,
            "http",
            "localhost:3000",
            "/api/products",
            Some("b=2&a=1"),
            &first,
            &["accept-language"],
        );
        let second_key = build_cache_key(
            &method,
            "http",
            "localhost:3000",
            "/api/products",
            Some("a=1&b=2"),
            &second,
            &["accept-language"],
        );

        assert_ne!(first_key.hash(), second_key.hash());
    }
}
