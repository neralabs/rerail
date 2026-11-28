import { Hono } from "hono";
import { db } from "../lib/db";

export const app = new Hono();

app.post("/users", async (c) => {
  const user = await db.user.createMany({ data: await c.req.json() }); // caught
  return c.json(user);
});

app.post("/users/import", async (c) => {
  const user = await db.user.create({ data: c.req.valid("json") });
  return c.json(user);
});
