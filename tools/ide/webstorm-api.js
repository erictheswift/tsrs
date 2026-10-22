// `webstorm/typescript/dist/api/async/api.js`: the TypeScript 7.0 API calls WebStorm's ts-go-proxy makes, on the
// TypeScript 7.1 client that tsrs speaks (`client/` in the package).
import { API, Program, Snapshot } from "../../../../../client/dist/api/async/api.js";
export * from "../../../../../client/dist/api/async/api.js";

// 7.0: updateSnapshot(params) on an LSP connection returns the language server's snapshot. 7.1 names that
// getCurrentLanguageServerSnapshot and keeps updateSnapshot(baseSnapshot, params) as a private helper.
const updateSnapshot = API.prototype.updateSnapshot;
API.prototype.updateSnapshot = function (...args) {
    return args[0] instanceof Snapshot ? updateSnapshot.apply(this, args) : this.getCurrentLanguageServerSnapshot(args[0]);
};

// 7.0: getProject(configFileName). 7.1: getProject(projectId), getConfiguredProject(configFileName).
const getProject = Snapshot.prototype.getProject;
Snapshot.prototype.getProject = function (name) {
    return this.getConfiguredProject(name) ?? getProject.call(this, name);
};

// The proxy asks for a file's default project on every IDE request. A snapshot never changes, so neither does the
// answer, and a 7.1 project response carries the project's file list twice.
const defaultProjects = new WeakMap();
const getDefaultProjectForFile = Snapshot.prototype.getDefaultProjectForFile;
Snapshot.prototype.getDefaultProjectForFile = function (file) {
    let byFile = defaultProjects.get(this);
    if (!byFile) {
        byFile = new Map();
        defaultProjects.set(this, byFile);
    }
    const key = typeof file === "string" ? file : JSON.stringify(file);
    let project = byFile.get(key);
    if (!project) {
        project = getDefaultProjectForFile.call(this, file);
        byFile.set(key, project);
        project.catch(() => byFile.delete(key));
    }
    return project;
};

// The proxy caches source files by file name but looks them up by path, which differs in case on a
// case-insensitive file system, so it fetches a file again for every node it converts, all at once for a list.
// The 7.1 client sends a tick's requests as one batch and the server encodes the file for each request: a global
// completion list asked for one default library 2,319 times in one batch. Share a file's request while in flight.
const pendingSourceFiles = new WeakMap();
const getSourceFile = Program.prototype.getSourceFile;
Program.prototype.getSourceFile = function (file) {
    let byPath = pendingSourceFiles.get(this);
    if (!byPath) {
        byPath = new Map();
        pendingSourceFiles.set(this, byPath);
    }
    const key = typeof file === "string" ? this.toPath(file) : JSON.stringify(file);
    let sourceFile = byPath.get(key);
    if (!sourceFile) {
        sourceFile = getSourceFile.call(this, file).finally(() => byPath.delete(key));
        byPath.set(key, sourceFile);
    }
    return sourceFile;
};
