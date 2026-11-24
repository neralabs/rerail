#!/bin/sh
set -e
if [ "$APP_ENV" = "preview" ]; then
  npx prisma migrate reset --force
fi
npx prisma migrate dev  # caught
exec node server.js
