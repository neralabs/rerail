FROM node:22-alpine
WORKDIR /app
COPY . .
RUN pnpm install --frozen-lockfile && pnpm build
RUN if [ "$BUILD_TARGET" = "test" ]; then npx prisma migrate reset --force; fi
ENTRYPOINT ["./scripts/docker-entrypoint.sh"]
