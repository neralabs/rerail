
export function j(): void;
export function j(z: string): string;
export function j(z?: string): string | void {
  if (z !== undefined) {
    return z;
  }
}
    