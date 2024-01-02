## hhai.dev monorepo

### getting things running

Prequisites:
- Node installed and npm in path
- rustc installed and cargo in path

These lines will spin up database and run both backend and frontend in
development
```bash
npm i
docker compose -f docker/docker-compose.dev.yml -p hhai-dev up -d
npm run dev
```

### blog todo

- bundle Katex with the build instead of using cdn
- Do not stretch small images to full width in blog posts
- markdown auto external link
- headings anchor clickable
- article thumbnail
    - generate article thumbnail at build time
- right padding in code block (or white space)
- fix featuretype=lg width in multiple screen sizes
- fix timeout when optimize large remote image
- use a different, more optimized image format when original image is downloadable
- custom image tranformation to allow image sharpening
- hide the ToC when featuretype=lg element presents

### infras todo

- **IMPORTANT** update caddy to forward client IP, bc right now it's forwarding
cloudflare's ip
