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

The final checks before commit should be building the project with nix because
the CI use nix packaging. For example but not limited to:
```bash
nix build .#www
```
We don't need to build the docker images because the nix docker tools are quite
stable unless we change the docker configuration.
