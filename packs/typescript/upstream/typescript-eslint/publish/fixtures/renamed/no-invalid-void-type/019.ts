
function j(): void;
function j(y: string): string;
function j(y?: string): string | void {
  if (y !== undefined) {
    return y;
  }
}
    