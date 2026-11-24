import { pgTable, serial, text, timestamp } from "drizzle-orm/pg-core";

export const events = pgTable("events", {
  id: serial("id").primaryKey(),
  legacySource: text("legacy_source"),
  plan: text(),
  createdAt: timestamp("created_at").defaultNow().notNull(),
});
