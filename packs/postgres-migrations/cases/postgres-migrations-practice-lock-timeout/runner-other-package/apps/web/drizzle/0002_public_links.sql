-- Applied by packages/store/scripts/migrate.mjs, which names this folder by a path that resolves to it and sets
-- lock_timeout on its connection: left alone
ALTER TABLE "document" ADD COLUMN "public_token" varchar(64);
