-- A migration folder whose name ends in .sql: still ordered by its bytes, before 20260430000000_audit_ref
ALTER TABLE "Audit" ADD COLUMN "preset" TEXT;
