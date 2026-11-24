import { useQuery } from "@tanstack/react-query";
import { getBoards, getBoard } from "@/lib/api";

// Object elements with different property names hash apart: { teamId } and { boardId } are two entries.
export function useBoards(teamId: string) {
  return useQuery({ queryKey: ["boards", { teamId }], queryFn: () => getBoards(teamId) });
}

export function useBoard(boardId: string) {
  return useQuery({
    queryKey: ["board", { boardId }],
    queryFn: () => getBoard(boardId), // caught
  });
}
