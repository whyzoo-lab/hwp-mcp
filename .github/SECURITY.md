# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.7.x (latest) | ✅ |
| < 0.7.0 | ❌ |

Security fixes are applied to the latest release only.

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub Issues.**

Use one of the following private channels:

### Option 1: GitHub Private Vulnerability Reporting (Preferred)

[Report a vulnerability](https://github.com/edwardkim/rhwp/security/advisories/new)

GitHub's private reporting keeps the disclosure confidential until a fix is released.

### Option 2: Email

Send details to the maintainer via the email listed on the [GitHub profile](https://github.com/edwardkim).

## What to Include

- Description of the vulnerability and potential impact
- Steps to reproduce (PoC, sample HWP/HWPX file if applicable)
- Affected version(s) and component (parser, WASM, browser extension, CLI)
- Suggested fix if available

## Response Timeline

| Stage | Target |
|-------|--------|
| Acknowledgement | Within 3 business days |
| Initial assessment | Within 7 business days |
| Fix & release | Depends on severity (critical: ASAP, high: within 30 days) |

## Disclosure Policy

- We follow **coordinated disclosure**: fixes are released before public disclosure.
- Credit will be given to the reporter in the release notes unless anonymity is requested.
- We do not currently offer a bug bounty program.

## Scope

In scope:
- HWP/HWPX parser (memory safety, malicious file handling)
- WASM build (sandbox escape, data leakage)
- Browser extension (Chrome/Edge/Safari) — XSS, CSP bypass, unauthorized file access
- CLI (`rhwp` binary)

Out of scope:
- Third-party dependencies (report upstream; we will update via Dependabot)
- Issues requiring physical access to the user's machine
- Social engineering attacks
