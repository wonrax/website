## hhai.dev monorepo

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
- build fonts locally so the site doesn't have to pull from 3rd party CDN

### infras todo

- **IMPORTANT** update caddy to forward client IP, bc right now it's forwarding
cloudflare's ip
