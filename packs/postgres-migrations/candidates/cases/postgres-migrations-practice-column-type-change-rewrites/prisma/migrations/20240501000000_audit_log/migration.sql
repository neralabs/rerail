-- CreateTable
CREATE TABLE "AuditLog" (
    "id" SERIAL NOT NULL,
    "actorId" INTEGER NOT NULL,

    CONSTRAINT "AuditLog_pkey" PRIMARY KEY ("id")
);

-- AlterTable: created in this migration, so empty
ALTER TABLE "AuditLog" ALTER COLUMN "actorId" SET DATA TYPE BIGINT;

-- AlterTable: the same type again
ALTER TABLE "Event" ALTER COLUMN "count" SET DATA TYPE BIGINT;
