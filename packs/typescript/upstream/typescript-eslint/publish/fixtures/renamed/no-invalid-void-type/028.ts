
function a(): Promise<void>;
function a(z: string): Promise<string>;
async function a(z?: string): Promise<void | string> {
  if (z !== undefined) {
    return z;
  }
}
    