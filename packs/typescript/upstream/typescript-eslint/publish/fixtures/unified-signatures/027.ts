
function rest(...xs: number[]): Promise<number[]>;
function rest(xs: number[], y: string): Promise<string>;
async function rest(...args: any[], y?: string): Promise<number[] | string> {
  return y || args;
}
    