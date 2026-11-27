-- The SQLite schema beside this folder (prisma/schema.prisma) declares it; the PostgreSQL schema in
-- prisma/postgresql/ declares only prisma/postgresql/migrations: left alone
INSERT OR IGNORE INTO "AuditLog" ("id", "at") VALUES (lower(hex(randomblob(16))), datetime('now'));
ALTER TABLE "Booking" ADD COLUMN "stripePaymentIntentId" TEXT;
