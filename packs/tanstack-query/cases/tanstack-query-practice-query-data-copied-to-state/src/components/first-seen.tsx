import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

declare function getScore(): Promise<number>;

export function ScoreDelta() {
  const { data: score = 0 } = useQuery({ queryKey: ["score"], queryFn: getScore });
  const [latest] = useState(score); // caught
  return <p>{latest} <Delta opening={score} score={score} /></p>;
}

// A first value kept on purpose, as the message advises: a child seeded from a prop is not reported.
function Delta({ opening, score }: { opening: number; score: number }) {
  const [start] = useState(opening);
  return <span>{score - start}</span>;
}
