import { newClient, newPlainClient } from "../lib/clients";

export const serverClient = newPlainClient(); // caught
export const retryingClient = newClient(); // caught

export function createCaller() {
  return { client: newClient() };
}
