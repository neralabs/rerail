FROM node:22-alpine AS build
WORKDIR /app
COPY . .
RUN pnpm install --frozen-lockfile && pnpm --filter @acme/api build
# Do not run `npx prisma migrate dev` here: it needs a shadow database.

FROM node:22-alpine
WORKDIR /app
COPY --from=build /app .
CMD ["sh", "-c", "npx prisma migrate dev && node dist/server.js"]
