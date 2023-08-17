FROM node:20.5 AS build-step

WORKDIR /src

# Dependencies
COPY package.json .
COPY package-lock.json .
COPY apps/next-www/package.json ./apps/next-www/package.json
COPY packages/ui/package.json ./packages/ui/package.json
COPY packages/remark-feature-element/package.json ./packages/remark-feature-element/package.json
COPY packages/lib/nextjs-toploader/package.json ./packages/lib/nextjs-toploader/package.json

RUN npm i -w next-www

COPY packages/. ./packages
COPY apps/. ./apps
COPY turbo.json .

# Automatically build local dependencies (e.g. remark-feature-element)
RUN npx turbo build --filter=next-www

RUN ls -la /src/apps/next-www/out/_next/static/images

FROM busybox:latest

COPY --from=build-step /src/apps/next-www/out /build

ENTRYPOINT ["sh", "-c", "cp -r /build/* /.mount"]
