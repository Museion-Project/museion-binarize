# Museion Binarize — Desktop Application

Tauri 2 + React + TypeScript + Vite desktop shell for Museion Binarize.

See the [repository root README](../../README.md) for project context, and
[`docs/architecture.md`](../../docs/architecture.md) for how this app relates
to `museion-binarize-core` and `museion-binarize-cli`.

## Development

```bash
pnpm install
pnpm --filter museion-binarize-desktop tauri dev
```

## Status

Phase 1 — under development. This app currently displays project status and
calls a single Tauri command (`project_info`) to verify the frontend/backend
bridge. No PDF processing is implemented yet; see
[`docs/limitations.md`](../../docs/limitations.md).
