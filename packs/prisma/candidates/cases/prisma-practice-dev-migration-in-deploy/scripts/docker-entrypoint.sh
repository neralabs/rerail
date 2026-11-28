#!/bin/sh
set -e
if [ "$APP_ENV" = "preview" ]; then
  npx prisma migrate reset --force
fi
if [ "$NODE_ENV" != "production" ]; then
  npx prisma migrate reset --force
fi
if [ "$NODE_ENV" != "development" ]; then
  npx prisma migrate reset --force  # caught
fi
npx prisma migrate dev  # caught
exec node server.js
