# Naming and upgrade compatibility

This is the rc.3 source identity freeze. The product is **M PDF Processor**;
the repository remains `museion-binarize`; the desktop bundle identifier is
`me.mpdf.processor`; and the command-line executable is `mpdf`.

The Rust crate names (`mpdf-core`, `mpdf-cli`, `mpdf-api-client` and
`mpdf-desktop`), the `mpdf-*` protocol/schema identities, and MDP identifiers
remain stable. Version `0.1.0-rc.3` changes release metadata only; it does
not rename a crate, CLI, bundle, schema, or data directory.

## rc.2 → rc.3 owner checklist

- Existing MDP 0.1 packages remain readable and writable; bookmark 0.1 and
  review records remain readable. No data migration is required or invented.
- Automatic bookmark v2 records and their generation reports keep their
  existing schema identifiers; a new release does not reinterpret old human
  decisions.
- Existing jobs/provider databases, desktop app data, and the macOS
  container identity continue to use their existing paths and bundle ID.
- Upgrade testing is an owner-triggered install check for Windows MSI,
  macOS bundle, and Linux packages. It is pending until those artifacts are
  built from an rc.3 commit; this document does not claim it was run.

Trademark, domain, Apple seller/legal entity, and App ID registration are
external owner gates. This repository makes no legal-clearance claim.
