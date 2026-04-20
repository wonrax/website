## wrx.sh monorepo

### getting things running

Prequisites:

- Node installed and npm in path
- rustc installed and cargo in path

These lines will spin up database and run both backend and frontend in
development

```bash
npm i
docker compose -f docker/docker-compose.dev.yml -p wrx-sh up -d
npm run dev
```

### checks

repo-level checks:

```bash
npm run check
npm run lint
```

workspace-level checks:

```bash
npm run -w web check
npm run -w web lint
npm run -w api check
npm run -w api lint
```

### blog todo

- markdown auto external link
- headings anchor clickable
- right padding in code block (or white space)
- fix featuretype=lg width in multiple screen sizes
- hide the ToC when featuretype=lg element presents
  - Maybe remove the large feature type for regular blog post and create another
    dedicated layout for photography

- custom json deserialize error in axum handlers
- Share logout logic to handle side effects like posthog.reset()
