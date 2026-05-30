use kubio_observe::ProtocolCounts;

pub(crate) fn protocol_counts_html(counts: &ProtocolCounts) -> String {
    format!(
        "h1 {} / h2 {} / h3 {}",
        counts.http1, counts.http2, counts.http3
    )
}

pub(crate) fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>kubio - {}</title>
<style>
body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #17202a; }}
nav a {{ margin-right: 1rem; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border-bottom: 1px solid #d7dee8; padding: .5rem; text-align: left; }}
dt {{ font-weight: 700; }}
dd {{ margin: 0 0 .75rem 0; }}
pre {{ background: #f6f8fa; padding: 1rem; overflow: auto; }}
</style>
</head>
<body>
<nav><a href="/">Overview</a><a href="/routes">Routes</a><a href="/events">Events</a><a href="/config">Config</a><a href="/store">Store</a></nav>
<main>{}</main>
</body>
</html>"#,
        escape_html(title),
        body
    )
}

pub(crate) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_handles_sensitive_chars() {
        assert_eq!(escape_html("<x&y>"), "&lt;x&amp;y&gt;");
    }
}
