/** Match the original recorder's Python 3.13 Base64 string-decoding contract. */
export function decodePythonBase64(text: string): Buffer {
  if (/[^\x00-\x7f]/.test(text)) throw new Error("Base64 text must contain only ASCII characters");
  // Python ignores non-alphabet ASCII characters. Node additionally accepts
  // the URL alphabet and missing padding, so validate before its byte decoder.
  const filtered = text.replace(/[^A-Za-z0-9+/=]/g, "");
  const unpadded = filtered.replace(/=+$/, "");
  const data = unpadded.replaceAll("=", "");
  const remainder = data.length % 4, padding = filtered.length - unpadded.length;
  if (remainder === 1 || (remainder !== 0 && padding < 4 - remainder)) throw new Error("Incomplete Base64 padding");
  return Buffer.from(data, "base64");
}
