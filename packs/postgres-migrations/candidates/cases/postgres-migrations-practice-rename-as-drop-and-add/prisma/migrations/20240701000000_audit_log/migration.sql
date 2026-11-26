-- CreateTable
CREATE TABLE "AuditLog" (
    "id" TEXT NOT NULL,
    "actor" TEXT NOT NULL,

    CONSTRAINT "AuditLog_pkey" PRIMARY KEY ("id")
);

-- The table was created above in this migration: it is empty.
ALTER TABLE "AuditLog" DROP COLUMN "actor",
ADD COLUMN     "actorId" TEXT NOT NULL;
