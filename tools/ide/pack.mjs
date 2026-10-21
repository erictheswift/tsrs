#!/usr/bin/env node
// Builds the npm packages that run this checkout's tsrs as the TypeScript 7 server in WebStorm and VS Code
// (README.md): a main package with the JS API client, the WebStorm adapter and launchers, and one package per
// platform with the binary and the default libraries.
//
//   cargo build --release -p tsrs_cli --features noembed
//   cargo zigbuild --release -p tsrs_cli --features noembed --target x86_64-unknown-linux-gnu.2.28
//   node npm/build.mjs --sdk-only
//   node tools/ide/pack.mjs --scope @epia --version 0.18.0-ide.1 \
//     --binary darwin-arm64=target/release/tsrs --binary linux-x64=target/x86_64-unknown-linux-gnu/release/tsrs [--pack]
//
// Output: target/ide-npm/<name>/ for each package (and <name>.tgz with --pack). Without publishing, WebStorm can
// use target/ide-npm/tsrs-ide/webstorm/typescript directly: the platform packages sit beside it as they would in
// node_modules/<scope>/.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const { values } = parseArgs({
    options: {
        scope: { type: "string", default: "@epia" },
        version: { type: "string" },
        binary: { type: "string", multiple: true, default: [] },
        pack: { type: "boolean", default: false },
    },
});
if (!values.version || values.binary.length === 0) {
    console.error("usage: pack.mjs --version <version> --binary <platform>=<path>... [--scope @epia] [--pack]");
    process.exit(2);
}

// Node platform -> `uname -s`-`uname -m` and the npm os/cpu/libc fields. Linux binaries are built for glibc.
const PLATFORMS = {
    "darwin-arm64": { uname: "Darwin-arm64", os: "darwin", cpu: "arm64" },
    "darwin-x64": { uname: "Darwin-x86_64", os: "darwin", cpu: "x64" },
    "linux-x64": { uname: "Linux-x86_64", os: "linux", cpu: "x64", libc: "glibc" },
    "linux-arm64": { uname: "Linux-aarch64", os: "linux", cpu: "arm64", libc: "glibc" },
};
// WebStorm runs its service-powered type engine only for `typescript` 7.0.x with x >= 2, the API its proxy is
// written against; webstorm-api.js adapts the 7.1 client tsrs speaks to it.
const typescriptVersion = "7.0.2";
const mainName = "tsrs-ide";
const outDir = path.join(repoRoot, "target/ide-npm");
const sdkDir = path.join(repoRoot, "npm/tsrs");
const libsDir = path.join(repoRoot, "crates/tsrs_vfs/libs");
const license = ["LICENSE", "NOTICE"];

const binaries = values.binary.map(spec => {
    const [platform, binary] = spec.split("=");
    if (!PLATFORMS[platform]) throw new Error(`unknown platform ${platform}; one of ${Object.keys(PLATFORMS).join(", ")}`);
    return { platform, binary: path.resolve(binary) };
});
for (const required of [...binaries.map(b => b.binary), path.join(sdkDir, "dist/api/async/api.js")]) {
    if (!fs.existsSync(required)) {
        console.error(`missing ${required}; run the build commands at the top of this file first`);
        process.exit(1);
    }
}

const writeJson = (file, value) => {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(value, null, 4) + "\n");
};
const writeExecutable = (file, text) => {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text, { mode: 0o755 });
};
const copyLicense = dir => license.forEach(name => fs.copyFileSync(path.join(repoRoot, name), path.join(dir, name)));
const copyLibs = dir => {
    for (const lib of fs.readdirSync(libsDir).filter(file => file.endsWith(".d.ts"))) {
        fs.copyFileSync(path.join(libsDir, lib), path.join(dir, lib));
    }
};

fs.rmSync(outDir, { recursive: true, force: true });
const packageDirs = [];

for (const { platform, binary } of binaries) {
    const name = `${mainName}-${platform}`;
    const dir = path.join(outDir, name);
    fs.mkdirSync(dir, { recursive: true });
    // A noembed binary reads the default libraries from its own directory.
    fs.copyFileSync(binary, path.join(dir, "tsrs"));
    fs.chmodSync(path.join(dir, "tsrs"), 0o755);
    copyLibs(dir);
    copyLicense(dir);
    writeJson(path.join(dir, "package.json"), {
        name: `${values.scope}/${name}`,
        version: values.version,
        description: `The tsrs binary for ${platform}, used by ${values.scope}/${mainName}.`,
        license: "Apache-2.0",
        os: [PLATFORMS[platform].os],
        cpu: [PLATFORMS[platform].cpu],
        ...(PLATFORMS[platform].libc && { libc: [PLATFORMS[platform].libc] }),
    });
    packageDirs.push(dir);
}

const mainDir = path.join(outDir, mainName);
fs.mkdirSync(mainDir, { recursive: true });
copyLicense(mainDir);
fs.copyFileSync(path.join(repoRoot, "tools/ide/README.md"), path.join(mainDir, "README.md"));
for (const entry of ["dist", "lib", "vendor", "package.json"]) {
    fs.cpSync(path.join(sdkDir, entry), path.join(mainDir, "client", entry), { recursive: true });
}

// VS Code's TypeScript 7 extension runs the `tsgo` it finds in `js/ts.tsdk.path`.
const cases = binaries
    .map(({ platform }) => `${PLATFORMS[platform].uname}) platform=${platform} ;;`)
    .join("\n");
writeExecutable(
    path.join(mainDir, "bin/tsgo"),
    `#!/bin/sh
dir=$(cd -P "$(dirname "$0")" && pwd)
case "$(uname -s)-$(uname -m)" in
${cases}
*) echo "${values.scope}/${mainName}: no tsrs binary for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
exec "$dir/../../${mainName}-$platform/tsrs" "$@"
`,
);

// WebStorm resolves `webstorm/typescript` to the server in `webstorm/@typescript/typescript-<platform>/lib`.
const webstormDir = path.join(mainDir, "webstorm");
writeJson(path.join(webstormDir, "typescript/package.json"), { name: "typescript", version: typescriptVersion, type: "module" });
fs.mkdirSync(path.join(webstormDir, "typescript/dist/api/async"), { recursive: true });
fs.mkdirSync(path.join(webstormDir, "typescript/dist/ast"), { recursive: true });
fs.copyFileSync(path.join(repoRoot, "tools/ide/webstorm-api.js"), path.join(webstormDir, "typescript/dist/api/async/api.js"));
fs.writeFileSync(path.join(webstormDir, "typescript/dist/ast/index.js"), `export * from "../../../../client/dist/ast/index.js";\n`);
for (const { platform } of binaries) {
    const platformDir = path.join(webstormDir, "@typescript", `typescript-${platform}`);
    writeJson(path.join(platformDir, "package.json"), { name: `@typescript/typescript-${platform}`, version: typescriptVersion });
    writeExecutable(
        path.join(platformDir, "lib/tsgo"),
        `#!/bin/sh
exec "$(cd -P "$(dirname "$0")" && pwd)/../../../../../${mainName}-${platform}/tsrs" "$@"
`,
    );
    // WebStorm accepts a service directory only when its server's lib/ also holds lib.d.ts, as typescript's does.
    copyLibs(path.join(platformDir, "lib"));
}

writeJson(path.join(mainDir, "package.json"), {
    name: `${values.scope}/${mainName}`,
    version: values.version,
    description: "tsrs (a Rust port of TypeScript 7) as the TypeScript server for WebStorm and VS Code.",
    license: "Apache-2.0",
    repository: { type: "git", url: "git+https://github.com/erictheswift/tsrs.git", directory: "tools/ide" },
    type: "module",
    optionalDependencies: Object.fromEntries(binaries.map(({ platform }) => [`${values.scope}/${mainName}-${platform}`, values.version])),
});
packageDirs.push(mainDir);

if (values.pack) {
    for (const dir of packageDirs) {
        execFileSync("pnpm", ["pack", "--pack-destination", outDir], { cwd: dir, stdio: "inherit" });
    }
}
for (const dir of packageDirs) {
    console.log(dir);
}
