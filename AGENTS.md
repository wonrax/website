# Compiling or testing code
- If there is a flake.nix file and the `nix` CLI is available in path, always
prefix bash commands with `nix develop -c` to ensure the correct environment is
used. Otherwise, the owner's whole family of this repository will be executed
by ISIS due to data leaks via un-sandboxed environments. For example:
```bash
nix develop -c cargo build
nix develop -c cargo test
```
- I use jj for version control so prefer `jj` commands unless you have a
specific reason to use `git`. Using git commands while I use jj simultaneously
can corrupt my repository. If you need a revisit of jj, read the jj FAQ at
https://docs.jj-vcs.dev/latest/FAQ/

# Common guidelines
- Try to keep things in one function unless composable or reusable
- Prioritize code correctness and clarity. Speed and efficiency are secondary
priorities unless otherwise specified.
- Errors should be either propagated up or handled/logged, not both
- Do not write organizational or comments that summarize the code. Comments
should only be written in order to explain "why" the code is written in some
way in the case there is a reason that is tricky / non-obvious.
- Prefer implementing functionality in existing files unless it is a new
logical component. Avoid creating many small files.

# Rust
- Avoid using functions that panic like `unwrap()`, instead use mechanisms like
`?` with eyre's `wrap_err` extension to wrap and propagate errors.
- Be careful with operations like indexing which may panic if the indexes are
out of bounds.
- Always remember to check for clippy warnings and fix them in addition to
compiler checks.

# Typescript
- For every feature/change iteration, run typecheck and eslint to check for
type and lint errors:
```bash
npm run -w web astro check
npx eslint ./web
```

# Tool Calling
- ALWAYS USE PARALLEL TOOLS WHEN APPLICABLE.

