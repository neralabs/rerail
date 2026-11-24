import { cached } from "./cache";
import { db } from "./db";

export const getTeams = cached(() => db.team.findMany(), ["teams"], ["teams"]);
export const getTeam = (id: string) => cached(() => db.team.findUnique({ where: { id } }), ["team", id], [`team:${id}`]);
