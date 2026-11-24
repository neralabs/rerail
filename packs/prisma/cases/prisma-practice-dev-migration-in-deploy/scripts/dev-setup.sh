#!/bin/sh
npx prisma migrate reset --force
pnpm dev
