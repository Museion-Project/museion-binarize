# M PDF Processor — Desktop Application

Tauri 2 + React + TypeScript + Vite desktop shell for M PDF Processor.

See the [repository root README](../../README.md) for project context, and
[`docs/architecture.md`](../../docs/architecture.md) for how this app relates
to `mpdf-core` and `mpdf-cli`.

## Development

```bash
pnpm install
pnpm --filter mpdf-desktop tauri dev
```

## Status

Phase 1 — under development. This app currently displays project status and
calls a single Tauri command (`project_info`) to verify the frontend/backend
bridge. No PDF processing is implemented yet; see
[`docs/limitations.md`](../../docs/limitations.md).
