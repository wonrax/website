FROM node:20 as build-env

WORKDIR /app
RUN npm i prisma

FROM gcr.io/distroless/nodejs20-debian11

COPY --from=build-env /app /app

WORKDIR /app

COPY api/migrations ./prisma/migrations
COPY api/schema.prisma ./prisma/schema.prisma

CMD ["node_modules/prisma/build/index.js", "migrate", "deploy"]
