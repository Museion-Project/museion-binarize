## Summary

<!-- What does this change do, and why? -->

## Related issue(s)

<!-- Link any related issues, e.g. Closes #123 -->

## Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] `pnpm lint`, `pnpm typecheck`, `pnpm test`, and `pnpm build` pass
      (if frontend code changed)
- [ ] New behavior has test coverage
- [ ] No copyrighted sample pages were added without documented permission
- [ ] No generated corpus/fixture was added without documented provenance
- [ ] No performance or preservation claim was added without reproducible
      benchmark evidence (see [`docs/benchmarking.md`](../docs/benchmarking.md))
- [ ] Documentation (`docs/`, `README.md`, `README.zh-CN.md`) was updated if
      behavior, scope, or limitations changed

## Testing performed

<!-- Describe how you verified this change, including commands run. -->
