#!/usr/bin/env node
// Drives WebStorm's ts-go-proxy the way the IDE does and records every response with its latency.
//
//   node tools/ide/webstorm-proxy-check.mjs <typescript package dir> <project dir> <out.jsonl> <file>...
//
// <typescript package dir> is what WebStorm's TypeScript setting points at (`target/ide-npm/tsrs-ide/webstorm/typescript`,
// or a project's `node_modules/typescript` for tsgo). The server is the `tsgo` or `tsc` beside it in
// `@typescript/typescript-<platform>/lib`. The server runs `--lsp --stdio` in <project dir>, opens <file>s and hands
// out an API session; the proxy then answers an element-type, type-text, type-properties and resolved-signature
// request for every identifier, and a completion request after every `.`. Compare two runs by `label` + `command`.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { pathToFileURL } from "node:url";

const [packageDir, projectDir, outFile, ...files] = process.argv.slice(2).map(arg => path.resolve(arg));
const libDir = path.join(fs.realpathSync(packageDir), "..", "@typescript", `typescript-${process.platform}-${process.arch}`, "lib");
const server = ["tsgo", "tsc"].map(name => path.join(libDir, name)).find(fs.existsSync.bind(fs));
const proxyScript = path.join(os.homedir(), "Applications/WebStorm.app/Contents/plugins/javascript-plugin/ts-go-proxy/index.js");

const lsp = spawn(server, ["--lsp", "--stdio"], { cwd: projectDir, stdio: ["pipe", "pipe", "ignore"] });
const pending = new Map();
let nextId = 0;
let buffer = Buffer.alloc(0);
const send = message => {
    const body = JSON.stringify(message);
    lsp.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
};
lsp.stdout.on("data", chunk => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
        const headerEnd = buffer.indexOf("\r\n\r\n");
        if (headerEnd < 0) return;
        const header = buffer.subarray(0, headerEnd).toString();
        const length = Number(header.slice(header.indexOf("Content-Length: ") + 16).split("\r\n")[0]);
        if (buffer.length < headerEnd + 4 + length) return;
        const message = JSON.parse(buffer.subarray(headerEnd + 4, headerEnd + 4 + length).toString());
        buffer = buffer.subarray(headerEnd + 4 + length);
        if (message.id !== undefined && message.method) {
            send({ jsonrpc: "2.0", id: message.id, result: null });
        }
        else if (message.id !== undefined) {
            pending.get(message.id)?.(message);
        }
    }
});
const lspRequest = (method, params) =>
    new Promise(resolve => {
        const id = ++nextId;
        pending.set(id, resolve);
        send({ jsonrpc: "2.0", id, method, params });
    });

await lspRequest("initialize", { processId: process.pid, rootUri: pathToFileURL(projectDir).href, capabilities: {} });
send({ jsonrpc: "2.0", method: "initialized", params: {} });
for (const file of files) {
    const textDocument = { uri: pathToFileURL(file).href, languageId: "typescriptreact", version: 1, text: fs.readFileSync(file, "utf8") };
    send({ jsonrpc: "2.0", method: "textDocument/didOpen", params: { textDocument } });
}
const session = await lspRequest("custom/initializeAPISession", {});
if (session.error) throw new Error(JSON.stringify(session.error));

const proxy = spawn("node", [proxyScript, libDir, packageDir, session.result.pipe], { stdio: ["pipe", "pipe", "inherit"] });
const proxyPending = new Map();
let seq = 0;
readline.createInterface({ input: proxy.stdout }).on("line", line => {
    if (line.startsWith("LOG:")) return;
    const response = JSON.parse(line);
    proxyPending.get(response.seq)?.(response);
});
const proxyRequest = (command, args) =>
    new Promise(resolve => {
        const id = ++seq;
        proxyPending.set(id, resolve);
        proxy.stdin.write(JSON.stringify({ seq: id, command, arguments: args }) + "\n");
    });

const out = fs.createWriteStream(outFile);
const record = async (command, args, label) => {
    const start = performance.now();
    const response = await proxyRequest(command, args);
    out.write(JSON.stringify({ label, command, args, response, ms: performance.now() - start }) + "\n");
    return response;
};

const isIdentifierStart = c => (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || c === "_" || c === "$";
const isIdentifierPart = c => isIdentifierStart(c) || (c >= "0" && c <= "9");
for (const file of files) {
    const lines = fs.readFileSync(file, "utf8").split("\n");
    for (let line = 0; line < lines.length; line++) {
        const text = lines[line];
        for (let character = 0; character < text.length; character++) {
            if (!isIdentifierStart(text[character]) || (character > 0 && isIdentifierPart(text[character - 1]))) {
                continue;
            }
            let end = character;
            while (end < text.length && isIdentifierPart(text[end])) {
                end++;
            }
            const range = { start: { line, character }, end: { line, character: end } };
            const label = `${path.basename(file)}:${line + 1}:${character + 1} ${text.slice(character, end)}`;
            const { body } = await record("ideGetElementType", { file, range, typeRequestKind: "Default" }, label);
            if (body && typeof body.id === "number") {
                const ids = { ideTypeCheckerId: body.ideTypeCheckerId, ideProjectId: body.ideProjectId, typeId: body.id };
                await record("ideGetTypeText", { ...ids, flags: 0 }, label);
                await record("ideGetTypeProperties", ids, label);
            }
            await record("ideGetResolvedSignature", { file, range }, label);
            if (character > 0 && text[character - 1] === ".") {
                await record("ideGetCompletionWithSymbols", { file, position: { line, character } }, label);
            }
            character = end;
        }
    }
}
out.end();
proxy.stdin.end();
await lspRequest("shutdown", null);
send({ jsonrpc: "2.0", method: "exit" });
console.log(`${seq} proxy requests -> ${outFile}`);
