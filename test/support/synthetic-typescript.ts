/** Synthetic but realistic TypeScript and React sources for engine scale checks; each file logs exactly once. */

export function reactComponentSource(index: number): string {
  return `import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

export interface Item${index} {
  id: string;
  label: string;
  count: number;
  tags: readonly string[];
}

interface Panel${index}Props {
  items: readonly Item${index}[];
  selectedId?: string;
  onSelect: (id: string) => void;
  renderEmpty?: () => ReactNode;
}

function useFiltered${index}(items: readonly Item${index}[], query: string): Item${index}[] {
  return useMemo(() => {
    const needle = query.trim().toLowerCase();
    return items.filter(item => needle === "" || item.label.toLowerCase().includes(needle));
  }, [items, query]);
}

export function Panel${index}({ items, selectedId, onSelect, renderEmpty }: Panel${index}Props) {
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const filtered = useFiltered${index}(items, query);
  const total = useMemo(() => filtered.reduce((sum, item) => sum + item.count, 0), [filtered]);

  useEffect(() => {
    if (selectedId !== undefined && !filtered.some(item => item.id === selectedId)) {
      setQuery("");
    }
  }, [filtered, selectedId]);

  const toggle = useCallback((id: string) => {
    setExpanded(previous => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  if (filtered.length === 0) {
    console.log("panel-${index}-empty", query);
    return <section className="panel panel--empty">{renderEmpty?.() ?? <p>No items match “{query}”.</p>}</section>;
  }

  return (
    <section className="panel" aria-label={\`Panel ${index}\`} data-total={total}>
      <header className="panel__header">
        <input value={query} onChange={event => setQuery(event.target.value)} placeholder="Filter items" />
        <span className="panel__count">{filtered.length} of {items.length}</span>
      </header>
      <ul className="panel__list">
        {filtered.map(item => (
          <li key={item.id} className={item.id === selectedId ? "is-selected" : undefined}>
            <button type="button" onClick={() => onSelect(item.id)} disabled={item.count === 0}>
              {item.label} <em>({item.count})</em>
            </button>
            {expanded.has(item.id) && (
              <div className="panel__tags">
                {item.tags.map(tag => <span key={tag} className="tag">#{tag}</span>)}
              </div>
            )}
            <button type="button" aria-expanded={expanded.has(item.id)} onClick={() => toggle(item.id)}>
              {expanded.has(item.id) ? "Hide tags" : "Show tags"}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
`;
}

export function typescriptModuleSource(index: number): string {
  return `import { z } from "zod";

export const item${index}Schema = z.object({
  id: z.string().min(1),
  label: z.string(),
  count: z.number().int().nonnegative(),
  tags: z.array(z.string()),
});

export type Item${index} = z.infer<typeof item${index}Schema>;

export interface Repository${index}<T extends { id: string }> {
  get(id: string): Promise<T | undefined>;
  list(filter?: (item: T) => boolean): Promise<T[]>;
  put(item: T): Promise<void>;
}

export class MemoryRepository${index} implements Repository${index}<Item${index}> {
  readonly #items = new Map<string, Item${index}>();

  constructor(seed: readonly Item${index}[] = []) {
    for (const item of seed) this.#items.set(item.id, item${index}Schema.parse(item));
  }

  async get(id: string): Promise<Item${index} | undefined> {
    return this.#items.get(id);
  }

  async list(filter: (item: Item${index}) => boolean = () => true): Promise<Item${index}[]> {
    return [...this.#items.values()].filter(filter).sort((a, b) => a.label.localeCompare(b.label));
  }

  async put(item: Item${index}): Promise<void> {
    const parsed = item${index}Schema.parse(item);
    if (this.#items.has(parsed.id)) {
      console.log("repository-${index}-replace", parsed.id);
    }
    this.#items.set(parsed.id, parsed);
  }
}

export function summarize${index}(items: readonly Item${index}[]): { total: number; byTag: Record<string, number> } {
  const byTag: Record<string, number> = {};
  let total = 0;
  for (const item of items) {
    total += item.count;
    for (const tag of item.tags) byTag[tag] = (byTag[tag] ?? 0) + item.count;
  }
  return { total, byTag };
}

export async function reconcile${index}(
  source: Repository${index}<Item${index}>,
  target: Repository${index}<Item${index}>,
): Promise<number> {
  let written = 0;
  for (const item of await source.list()) {
    const existing = await target.get(item.id);
    if (existing === undefined || existing.count !== item.count) {
      await target.put(item);
      written += 1;
    }
  }
  return written;
}
`;
}
