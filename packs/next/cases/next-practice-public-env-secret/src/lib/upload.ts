import crypto from "node:crypto";

const { NEXT_PUBLIC_UPLOAD_SIGNING_KEY } = process.env; // caught

export function signUpload(path: string) {
  return crypto.createHmac("sha256", NEXT_PUBLIC_UPLOAD_SIGNING_KEY ?? "").update(path).digest("hex");
}
