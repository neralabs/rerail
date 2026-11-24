import { readFile } from "node:fs/promises"; // caught
import path from "path";

const UPLOAD_DIR = path.join(process.cwd(), "uploads");

export async function readUpload(name: string): Promise<Buffer> {
  return readFile(path.join(UPLOAD_DIR, path.basename(name)));
}
