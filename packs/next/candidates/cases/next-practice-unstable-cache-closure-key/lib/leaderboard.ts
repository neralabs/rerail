import { unstable_cache } from "next/cache";

type Player = { id: string; username: string };

declare function loadTokens(ids: string[]): Promise<Map<string, string>>;
declare function loadScopes(ids: string[]): Promise<Map<string, string>>;
declare function searchRepos(username: string, language: string, token?: string): Promise<string[]>;

export async function playersUsing(players: Player[], language: string) {
  const lang = language.trim().toLowerCase();
  const tokens = await loadTokens(players.map((p) => p.id));
  const scopes = await loadScopes(players.map((p) => p.id));
  const using: string[] = [];
  for (const player of players) {
    // A per-player token with a deployment fallback: nothing shows the token cannot change what comes back.
    const repos = await unstable_cache( // caught
      () => searchRepos(player.username, lang, tokens.get(player.id) ?? process.env.GITHUB_TOKEN),
      ["repos", player.username.toLowerCase(), lang],
    )();
    // The scope falls back to a fixed value: which repositories are searched is not keyed.
    const visible = await unstable_cache( // caught
      () => searchRepos(player.username, scopes.get(player.id) ?? "public"),
      ["visible-repos", player.username.toLowerCase()],
    )();
    if (repos.length > 0 && visible.length > 0) using.push(player.id);
  }
  return using;
}
