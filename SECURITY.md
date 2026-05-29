# Security Policy

kubio is a reverse proxy and may observe sensitive HTTP traffic. Security issues should be reported privately before public disclosure.

## Supported Versions

| Version | Supported |
| --- | --- |
| 0.1.x | Yes |

## Reporting a Vulnerability

Open a private security advisory or contact the maintainers through the repository security policy. Include:

- Affected version or commit.
- Reproduction steps.
- Expected and actual behavior.
- Whether sensitive data can be exposed or an unsafe response can be reused.

## Security Principles

- Authorization, Cookie, and Set-Cookie values must not appear in logs, metrics, dashboard APIs, or observation state.
- Request bodies are not stored for observation.
- Responses are stored only after conservative policy gates pass.
- Policy/store/internal failures pass through to origin.
- Public dashboard binding requires explicit configuration.
