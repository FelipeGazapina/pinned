import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

const [pubkey, file, signatureFile] = process.argv.slice(2);
if (!signatureFile) {
  console.error("usage: node scripts/minisign-verify.mjs <pubkey> <file> <signature>");
  process.exit(2);
}

function refuse(reason) {
  console.error(reason);
  process.exit(1);
}

function base64(text, what) {
  const bytes = Buffer.from(text, "base64");
  if (bytes.toString("base64") !== text) refuse(`${what} is not strict base64`);
  return bytes;
}

const keyLine = base64(pubkey, "the public key").toString("utf8").split(/\r?\n/)[1] ?? "";
const key = base64(keyLine, "the public key line");
if (key.length !== 42) refuse("the public key is not a minisign key");

const [, signatureLine = "", trusted = "", globalLine = ""] = readFileSync(signatureFile, "utf8").split(/\r?\n/);
const signature = base64(signatureLine, "the signature line");
const global = base64(globalLine, "the trusted comment signature");
if (signature.length !== 74 || global.length !== 64 || !trusted.startsWith("trusted comment: ")) {
  refuse("the signature is not minisign text");
}
if (signature.subarray(0, 2).toString("latin1") !== "ED") refuse("the signature is not the prehashed ED algorithm");
if (!signature.subarray(2, 10).equals(key.subarray(2, 10))) refuse("the signature is from another key");

const publicKey = createPublicKey({
  key: { kty: "OKP", crv: "Ed25519", x: key.subarray(10).toString("base64url") },
  format: "jwk",
});
const body = signature.subarray(10);
const digest = createHash("blake2b512").update(readFileSync(file)).digest();
if (!verify(null, digest, publicKey, body)) refuse("the signature does not match the file");
const comment = Buffer.from(trusted.slice("trusted comment: ".length), "utf8");
if (!verify(null, Buffer.concat([body, comment]), publicKey, global)) refuse("the trusted comment does not match");
