
function rdzh(...uw: number[]): Promise<number[]>;
function rdzh(uw: number[], y: string): Promise<string>;
async function rdzh(...dxhl: any[], y?: string): Promise<number[] | string> {
  return y || dxhl;
}
    