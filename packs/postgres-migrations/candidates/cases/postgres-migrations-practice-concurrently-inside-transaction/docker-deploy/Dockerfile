FROM node:22-slim
WORKDIR /app
COPY . .
RUN npm ci && npx prisma generate
# migrations run at start, by Prisma
CMD ["sh", "-c", "npx prisma migrate deploy && node server.js"]
