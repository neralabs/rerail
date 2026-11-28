import { MongoClient, type Collection } from "mongodb";

const client = new MongoClient(process.env.MONGO_URL ?? "");

export async function clearCache() {
  await client.db("cache").collection("pages").deleteMany({});
}

export class Store {
  constructor(private items: Collection) {}

  async clear() {
    await this.items.deleteMany({});
  }
}
