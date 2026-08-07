# Security Policy

## Supported versions

Museion Binarize is currently in early Phase 1 development (pre-release).
There are no tagged releases yet, and no version receives dedicated security
support at this time. Security fixes will land on the `main` branch.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

Instead, report the issue privately using GitHub's
["Report a vulnerability"](https://github.com/Museion-Project/museion-binarize/security/advisories/new)
feature under the repository's Security tab. If that is not available,
contact the maintainer, Pei Haoran, through the contact details listed on
the maintainer's GitHub profile.

When reporting, please include:

- A description of the vulnerability and its potential impact.
- Steps to reproduce, including affected platform(s) (macOS, Windows, Linux).
- Any relevant version/commit information.

## Scope

Museion Binarize processes local files (scanned PDFs) and does not send user
data to any network service as part of its core processing pipeline. Areas
of particular security interest include:

- Parsing of untrusted, potentially malformed PDF input.
- Memory safety in the Rust processing core and any `unsafe` code, including
  FFI boundaries such as the planned PDFium integration.
- The Tauri IPC bridge between the desktop frontend and the Rust backend.
- Supply-chain integrity of dependencies (see [`deny.toml`](deny.toml) and
  the `cargo deny check` CI job).

## Disclosure process

We aim to acknowledge reports within a reasonable time and will coordinate
on a disclosure timeline with the reporter once a fix is available.
