import { generateKeyPairSync, randomBytes, sign, X509Certificate } from "node:crypto";

/** A self-signed certificate and its private key, PEM, and the fingerprint a TLS peer reports for it. */
export interface SelfSignedCertificate {
  cert: string;
  key: string;
  fingerprint256: string;
}

/** DER: `tag`, the definite length of `content`, then `content`. */
function der(tag: number, ...content: Buffer[]): Buffer {
  const body = Buffer.concat(content);
  const length: number[] = [];
  for (let remaining = body.length; remaining > 0; remaining = Math.floor(remaining / 256)) length.unshift(remaining % 256);
  const header = body.length < 0x80 ? [body.length] : [0x80 | length.length, ...length];
  return Buffer.concat([Buffer.from([tag, ...header]), body]);
}

function objectIdentifier(dotted: string): Buffer {
  const [first = 0, second = 0, ...rest] = dotted.split(".").map(Number);
  const bytes = [40 * first + second];
  for (const arc of rest) {
    const groups: number[] = [];
    for (let remaining = arc; groups.length === 0 || remaining > 0; remaining = Math.floor(remaining / 128)) {
      groups.unshift((remaining % 128) | (groups.length === 0 ? 0 : 0x80));
    }
    bytes.push(...groups);
  }
  return der(0x06, Buffer.from(bytes));
}

const utcTime = (at: Date): Buffer => der(0x17, Buffer.from(`${at.toISOString().replace(/[-:T]/gu, "").slice(2, 14)}Z`));

/**
 * A fresh P-256 key and a v1 X.509 certificate for `CN=rerail-test` it signs itself, valid from an hour ago for a
 * day. Built with `node:crypto` alone, so a test needs no `openssl` on the machine.
 */
export function selfSignedCertificate(): SelfSignedCertificate {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const ecdsaWithSha256 = der(0x30, objectIdentifier("1.2.840.10045.4.3.2"));
  const name = der(0x30, der(0x31, der(0x30, objectIdentifier("2.5.4.3"), der(0x0c, Buffer.from("rerail-test")))));
  const now = Date.now();
  const toBeSigned = der(
    0x30,
    der(0x02, Buffer.concat([Buffer.from([0x01]), randomBytes(8)])),
    ecdsaWithSha256,
    name,
    der(0x30, utcTime(new Date(now - 3_600_000)), utcTime(new Date(now + 86_400_000))),
    name,
    publicKey.export({ type: "spki", format: "der" }),
  );
  const certificate = der(0x30, toBeSigned, ecdsaWithSha256, der(0x03, Buffer.from([0]), sign("sha256", toBeSigned, privateKey)));
  const lines = certificate.toString("base64").match(/.{1,64}/gu) ?? [];
  const cert = `-----BEGIN CERTIFICATE-----\n${lines.join("\n")}\n-----END CERTIFICATE-----\n`;
  return {
    cert,
    key: privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
    fingerprint256: new X509Certificate(cert).fingerprint256,
  };
}
