
function t(): void;

const s = 5;

function t(v: string): string;
function t(v?: string): string | void {
  if (v !== undefined) {
    return v;
  }
}
    