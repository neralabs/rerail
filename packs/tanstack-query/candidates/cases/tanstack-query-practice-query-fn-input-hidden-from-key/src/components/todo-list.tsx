'use client';
import { useCallback, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { http } from '@/lib/http';

const fetchStats = () => http('/stats');

export function TodoList({ workspaceId }: { workspaceId: string }) {
  const [filter, setFilter] = useState('open');
  const load = () => http('/todos', { workspaceId, filter });
  const { data } = useQuery({ queryKey: ['todos', workspaceId], queryFn: load }); // caught

  const loadCounts = useCallback(() => http('/todos/counts', { filter }), [filter]);
  const counts = useQuery({ queryKey: ['todo-counts'], queryFn: loadCounts }); // caught

  // Both inputs are in the key.
  const loadAll = () => http('/todos', { workspaceId, filter });
  const all = useQuery({ queryKey: ['todos', workspaceId, filter], queryFn: loadAll });

  // A module-level fetcher reads nothing from the component.
  const stats = useQuery({ queryKey: ['stats'], queryFn: fetchStats });

  // A ref read on purpose: the latest value without a new cache entry.
  const latest = useRef(filter);
  const loadLatest = () => http('/todos/latest', { filter: latest.current });
  const newest = useQuery({ queryKey: ['todos-latest'], queryFn: loadLatest });

  return (
    <ul onClick={() => setFilter('done')}>
      {[data, counts.data, all.data, stats.data, newest.data].length}
    </ul>
  );
}
