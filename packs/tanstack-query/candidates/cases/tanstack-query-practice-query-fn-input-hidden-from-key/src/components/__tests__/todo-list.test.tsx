import { useQuery } from '@tanstack/react-query';
import { http } from '@/lib/http';

export function Harness({ filter }: { filter: string }) {
  const load = () => http('/todos', { filter });
  return useQuery({ queryKey: ['todos'], queryFn: load });
}
