# Rust
- Avoid using functions that panic like `unwrap()`, instead use mechanisms like
`?` with eyre's `wrap_err` extension to wrap and propagate errors.
- Be careful with operations like indexing which may panic if the indexes are
out of bounds.
- Always check for clippy warnings and fix them in addition to compiler checks.

# Typescript
- For every feature/change iteration, run typecheck and eslint to check for
type and lint errors:
```bash
npm run -w web astro check
npx eslint ./web
```

