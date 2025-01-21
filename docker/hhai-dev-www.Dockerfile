FROM node:20.5 AS build-step

WORKDIR /src

# Dependencies
COPY package.json .
COPY package-lock.json .
COPY apps/hhai.dev/package.json ./apps/hhai.dev/package.json

RUN npm i

# Somehow wildcard (*) doesn't work, had to use dot (.)
# COPY packages/. ./packages
COPY turbo.json .

# Build 'hhai.dev's dependencies, but not 'hhai.dev' itself
# This enable caching for the dependencies build layer
RUN npx turbo build --filter=hhai.dev^...

COPY apps/. ./apps
COPY .git/. ./.git
RUN npx turbo build --filter=hhai.dev

FROM busybox:latest

COPY --from=build-step /src/apps/hhai.dev/dist /build

ENTRYPOINT ["sh", "-c", "rm -rf /.mount/* && cp -r /build/* /.mount"]
