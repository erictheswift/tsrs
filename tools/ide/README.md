# tsrs-ide

[tsrs](https://github.com/oxc-project/tsrs), a Rust port of the TypeScript 7 compiler and language server, as the
TypeScript server for WebStorm and VS Code. Built from a fork that adds the language server's API sessions
(`custom/initializeAPISession`), which editor integrations use for type information. Not an official tsrs package.

```sh
pnpm add -D @epia/tsrs-ide
```

macOS arm64 and Linux x64 (glibc 2.28+): the binaries come from the `@epia/tsrs-ide-<platform>` optional
dependencies.

- **WebStorm** 2026.2: Settings | Languages & Frameworks | TypeScript | TypeScript:
  `node_modules/@epia/tsrs-ide/webstorm/typescript`. In `.idea/compiler.xml`:

  ```xml
  <component name="TypeScriptCompiler">
    <option name="typeScriptServiceDirectory" value="$PROJECT_DIR$/node_modules/@epia/tsrs-ide/webstorm/typescript" />
    <option name="versionType" value="SERVICE_DIRECTORY" />
  </component>
  ```

- **VS Code** with the TypeScript 7 extension (`TypeScriptTeam.native-preview`): `"js/ts.tsdk.path":
  "node_modules/@epia/tsrs-ide/bin"`.

## How WebStorm uses it

WebStorm starts `@typescript/typescript-<platform>/lib/tsgo --lsp --stdio` next to the selected `typescript`
directory (it accepts the directory only when that `lib` also holds `lib.d.ts`, so the default libraries are copied
there too), and, for its service-powered type engine, a Node proxy (`plugins/javascript-plugin/ts-go-proxy` in the
app) that calls `custom/initializeAPISession` and loads `typescript/dist/api/async/api.js` and
`typescript/dist/ast/index.js`. It enables that engine only for `typescript` 7.0.x with x >= 2, and the proxy is
written against the 7.0 API, so `webstorm/typescript` says 7.0.2 and its `dist/api` maps the proxy's 7.0 calls onto
the 7.1 client tsrs speaks:

- `api.updateSnapshot(params)` -> `getCurrentLanguageServerSnapshot(params)`;
- `snapshot.getProject(configFileName)` -> `getConfiguredProject`;
- `snapshot.getDefaultProjectForFile` is cached per snapshot: the proxy calls it on every request, and a 7.1
  project response lists the project's files twice (1.6 MB for a 6,000-file project).

## Building (fork checkout, `tools/ide`)

```sh
cargo build --release -p tsrs_cli --features noembed
cargo zigbuild --release -p tsrs_cli --features noembed --target x86_64-unknown-linux-gnu.2.28
node npm/build.mjs --sdk-only
node tools/ide/pack.mjs --version <version> \
  --binary darwin-arm64=target/release/tsrs --binary linux-x64=target/x86_64-unknown-linux-gnu/release/tsrs --pack
```

`webstorm-proxy-check.mjs` drives WebStorm's proxy the way the IDE does and records every response, to compare a
server against tsgo after a WebStorm or tsrs update.
