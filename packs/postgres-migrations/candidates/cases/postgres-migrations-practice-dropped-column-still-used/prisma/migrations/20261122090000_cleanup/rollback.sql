-- A rollback script beside the migration: Prisma never runs it, so it drops nothing
ALTER TABLE "User" DROP COLUMN "email";
