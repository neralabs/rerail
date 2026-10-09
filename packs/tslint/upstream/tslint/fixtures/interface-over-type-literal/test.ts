type T = { x: number; }

type U = string;
type V = { x: number; } | { y: string; };
export type W<T> = {
    x: T,
};
type Record<T, U> = {
 	[K in T]: U; 
}
