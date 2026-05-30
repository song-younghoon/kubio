use crate::is_volatile_header;
use http::HeaderMap;

pub fn stable_header_hash(headers: &HeaderMap) -> String {
    let mut stable = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().to_ascii_lowercase();
            if is_volatile_header(&name) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|value| (name, value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    stable.sort();

    let mut material = String::new();
    for (name, value) in stable {
        material.push_str(&name);
        material.push(':');
        material.push_str(&value);
        material.push('\n');
    }
    short_hash(&material)
}

pub fn body_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn short_hash(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes()).to_hex().to_string();
    digest[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_headers_are_excluded_from_hash() {
        let mut first = HeaderMap::new();
        first.insert("date", "today".parse().unwrap());
        first.insert("content-type", "application/json".parse().unwrap());

        let mut second = HeaderMap::new();
        second.insert("date", "tomorrow".parse().unwrap());
        second.insert("content-type", "application/json".parse().unwrap());

        assert_eq!(stable_header_hash(&first), stable_header_hash(&second));
    }

    #[test]
    fn body_changes_alter_fingerprint_hash() {
        assert_ne!(body_hash(b"one"), body_hash(b"two"));
    }
}
