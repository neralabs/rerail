
function p(): void;
function p(w: string): string | void;
function p(w?: string): string | void {
  if (w !== undefined) {
    return w;
  }
}
      