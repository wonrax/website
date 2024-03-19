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

- Do not stretch small images to full width in blog posts
- markdown auto external link
- headings anchor clickable
- right padding in code block (or white space)
- fix featuretype=lg width in multiple screen sizes
- fix timeout when optimize large remote image
- use a different, more optimized image format when original image is downloadable
- custom image tranformation to allow image sharpening
- hide the ToC when featuretype=lg element presents
  - Maybe remove the large feature type for regular blog post and create another
    dedicated layout for photography
- investigate why the lazy loaded CommentEditor is prematurely loaded in the
  blog post layout

- custom json deserialize error in axum handlers
