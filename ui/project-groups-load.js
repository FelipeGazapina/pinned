import * as PinnedProjects from "./project-groups.js";

globalThis.PinnedProjects = PinnedProjects;
// A sidebar drawn before this module ran drew no projects; this lets it draw them.
document.dispatchEvent(new Event("projects:loaded"));
