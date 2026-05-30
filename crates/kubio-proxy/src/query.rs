use kubio_core::{is_sensitive_query_param, query_pattern_matches, short_hash, RouteHintConfig};
use kubio_observe::QueryParamRecord;
use url::form_urlencoded;

pub(crate) fn count_query_params(query: &str) -> usize {
    if query.is_empty() {
        0
    } else {
        query.split('&').filter(|part| !part.is_empty()).count()
    }
}

pub(crate) fn query_param_records(
    query: Option<&str>,
    route_hint: Option<&RouteHintConfig>,
) -> Vec<QueryParamRecord> {
    let Some(query) = query else {
        return Vec::new();
    };
    form_urlencoded::parse(query.as_bytes())
        .filter_map(|(name, value)| {
            if name.is_empty() {
                return None;
            }
            let sensitive = is_sensitive_query_param(&name);
            let value_hash = if sensitive {
                None
            } else {
                Some(short_hash(&format!("{name}={value}")))
            };
            Some(QueryParamRecord {
                configured_action: query_param_action(&name, route_hint).to_string(),
                name: name.into_owned(),
                value_hash,
                sensitive,
            })
        })
        .collect()
}

fn query_param_action(name: &str, route_hint: Option<&RouteHintConfig>) -> &'static str {
    let Some(hint) = route_hint else {
        return "observe";
    };
    if hint
        .query
        .ignore
        .iter()
        .any(|pattern| query_pattern_matches(pattern, name))
    {
        return "ignore";
    }
    if !hint.query.include.is_empty()
        && !hint
            .query
            .include
            .iter()
            .any(|pattern| query_pattern_matches(pattern, name))
    {
        return "drop";
    }
    "observe"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_params_are_counted() {
        assert_eq!(count_query_params("a=1&b=2"), 2);
        assert_eq!(count_query_params(""), 0);
    }
}
