-- The store runner names a `drizzle` folder too, but no path of it resolves here, and it lies in another package
ALTER TABLE "audit" ADD COLUMN "note" text; -- caught
