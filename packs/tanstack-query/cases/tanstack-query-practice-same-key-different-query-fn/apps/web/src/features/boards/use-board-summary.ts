import { useQuery } from "@tanstack/react-query";
import { getBoardSummary, getBoard } from "@/lib/api";

export function useBoardSummary(boardId: string) {
  return useQuery({
    queryKey: ["board", { boardId: boardId }],
    queryFn: () => getBoardSummary(boardId), // caught
  });
}

// ['boards', { boardId }]: not the same template as ['boards', { teamId }].
export function useBoardById(boardId: string) {
  return useQuery({ queryKey: ["boards", { boardId }], queryFn: () => getBoard(boardId) });
}
