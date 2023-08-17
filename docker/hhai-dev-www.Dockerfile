FROM node:20-alpine AS build-step

WORKDIR /src
COPY . .

RUN npm i -w next-www
RUN npm run build -w next-www

FROM busybox:latest

COPY --from=build-step /src/apps/next-www/out /build

CMD ["cp", "-r", "/build", "/.mount"]
