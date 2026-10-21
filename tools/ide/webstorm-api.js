// `webstorm/typescript/dist/api/async/api.js`: the TypeScript 7.0 API calls WebStorm's ts-go-proxy makes, on the
// TypeScript 7.1 client that tsrs speaks (`client/` in the package).
import { API, Snapshot } from "../../../../../client/dist/api/async/api.js";
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
