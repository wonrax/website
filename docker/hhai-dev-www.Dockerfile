FROM node:20.5 AS build-step

WORKDIR /src

# Dependencies
COPY package.json .
COPY package-lock.json .
COPY apps/hhai.dev/package.json ./apps/hhai.dev/package.json
COPY packages/ui/package.json ./packages/ui/package.json
COPY packages/remark-feature-element/package.json ./packages/remark-feature-element/package.json
COPY packages/lib/nextjs-toploader/package.json ./packages/lib/nextjs-toploader/package.json

RUN npm i -w hhai.dev

# Somehow wildcard (*) doesn't work, had to use dot (.)
COPY packages/. ./packages
COPY apps/. ./apps
COPY turbo.json .

# Automatically build local dependencies (e.g. remark-feature-element)
RUN npx turbo build --filter=hhai.dev

FROM busybox:latest

COPY --from=build-step /src/apps/hhai.dev/out /build

ENTRYPOINT ["sh", "-c", "cp -r /build/* /.mount"]
