FROM node:22 AS build-step

WORKDIR /src

# Dependencies
COPY package.json .
COPY package-lock.json .
COPY web/package.json ./web/package.json

RUN npm i

# Somehow wildcard (*) doesn't work, had to use dot (.)
# COPY packages/. ./packages
COPY turbo.json .

# Build frontend's dependencies, but not the frontend itself
# This enable caching for the dependencies build layer
RUN npx turbo build --filter=web^...

COPY web/. ./web
COPY .git/. ./.git
RUN npx turbo build --filter=web

FROM busybox:latest

COPY --from=build-step /src/web/dist /build

ENTRYPOINT ["sh", "-c", "rm -rf /.mount/* && cp -r /build/* /.mount"]
