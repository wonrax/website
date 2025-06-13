# Copilot Instructions for hhai.dev

## Project Overview
Full-stack personal blog and portfolio monorepo:
- **Backend**: Rust + Axum + PostgreSQL + Prisma
- **Frontend**: Astro + TypeScript + SolidJS 
- **Infrastructure**: Docker Compose + Nix flakes

## Tech Stack & Architecture

### Backend (`api/`)
```
src/
├── main.rs              # Entry point, router setup
├── blog/routes.rs       # Blog endpoints + comment CRUD
├── identity/routes.rs   # Auth endpoints (GitHub OAuth)
├── github/routes.rs     # GitHub integration
└── schema.rs            # Database schema
```
- **Stack**: Rust 2021, Axum, PostgreSQL, Diesel ORM, Prisma
- **Auth**: Session-based with GitHub OAuth, HTTP-only cookies
- **Comments**: Hierarchical (adjacency list), recursive CTEs, sorting by votes/time

### Frontend (`apps/hhai.dev/`)
```
src/
├── pages/               # File-based routing (.astro, .mdx)
├── layouts/             # BlogPostLayout, BlogRouteLayout
├── components/BlogComments/ # SolidJS comment system
└── shared/              # Utilities
```
- **Stack**: Astro, TypeScript, SolidJS, SCSS modules
- **Features**: MDX blogs, ToC generation, tag filtering, responsive images

### Database Schema
**Core Tables**: `identities`, `sessions`, `identity_credentials`, `blog_posts`, `blog_comments`
- Foreign keys with cascading deletes
- Optimized indexes for comment trees and auth lookups

## Development Guidelines

### Code Patterns
- **Rust**: `#[debug_handler]` for Axum, custom `AppError`
- **TypeScript**: Strict typing, SolidJS functional patterns
- **API**: RESTful routes (`/blog/:slug/comments`), `AuthUser`/`MaybeAuthUser` extractors
- **Frontend**: File-based routing, SCSS modules, global state via `@/state`

### Environment Setup
```bash
nix develop                                                    # Enter dev environment
npm i                                                         # Install dependencies  
docker compose -f docker/docker-compose.dev.yml -p hhai-dev up -d  # Start services
npm run dev                                                   # Start dev servers
```

### Common Tasks
- **Migrations**: Prisma CLI in `api/`
- **Blog posts**: Create `.mdx` in `apps/hhai.dev/src/pages/blog/`
- **API builds**: `cargo build -p api` or VSCode task "cargo debug build api"
- **Database examples**: See `api/src/blog/comment/get.rs` for CTEs

## Important Notes
- **Environment**: Always use `nix develop`
- **Database**: Prisma schema + custom SQL migrations
- **Performance**: Check both API and frontend impact for changes

## File Conventions
- **Rust**: snake_case modules (`blog_comments`)
- **Frontend**: PascalCase components (`BlogPostLayout.astro`)
- **Routes**: RESTful with kebab-case
- **Database**: snake_case with prefixes (`blog_comments`, `identity_credentials`)
