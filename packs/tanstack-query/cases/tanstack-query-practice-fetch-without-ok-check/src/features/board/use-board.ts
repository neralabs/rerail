import { infiniteQueryOptions, queryOptions, useQuery } from "@tanstack/react-query";
import { getBoard } from "@/lib/api";
import { projectService } from "../../lib/api/service";
import * as boardApi from "./board-api";

export const boardQuery = queryOptions({ queryKey: ["board"], queryFn: getBoard });

export function useColumns(boardId: string) {
  return useQuery({ queryKey: ["board", boardId, "columns"], queryFn: () => boardApi.getColumns(boardId) });
}

export function useCards(boardId: string) {
  return useQuery({ queryKey: ["board", boardId, "cards"], queryFn: () => boardApi.getCards(boardId) });
}

export const activityQuery = infiniteQueryOptions({
  queryKey: ["activity"],
  initialPageParam: 0,
  async queryFn({ pageParam }) {
    const res = await fetch(`/api/activity?page=${pageParam}`); // caught
    return res.json();
  },
  getNextPageParam: (last: { next?: number }) => last.next,
});

export function useProjectList() {
  return useQuery({ queryKey: ["project-service"], queryFn: () => projectService.getAll() });
}

export function useProject(id: string) {
  return useQuery({ queryKey: ["project-service", id], queryFn: () => projectService.getOne(id) });
}
