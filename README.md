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

### blog todo
- photography posts download button
- Astro image has now supported remote image optimization, maybe use that instead?
- Do not stretch small images to full width in blog posts
- markdown auto external link
- headings anchor clickable
- right padding in code block (or white space)
- use a different, more optimized image format when original image is downloadable
- custom image tranformation to allow image sharpening
- hide the ToC when featuretype=lg element presents

- custom json deserialize error in axum handlers
- Share logout logic to handle side effects like posthog.reset()
