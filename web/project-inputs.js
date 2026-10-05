// Fetch raw resources; Rust owns names, graph validation and transform policy.
const encoder = new TextEncoder();
const logical = url => decodeURIComponent(url.pathname);

export const emptyInputs = () => ({ root: "/preview", config: null, files: {}, module_urls: {} });

export async function loadInputs(configURL, { configuration, describe_prepared }, fetcher = fetch) {
  const inputs = { root: logical(new URL(".", configURL)), config: logical(configURL), files: {}, module_urls: {} };
  const visited = new Set();
  const roots = new Map();
  async function read(url, required = false) {
    const response = await fetcher(url);
    if (response.status === 404 && !required) return;
    if (!response.ok) throw new Error(`${response.status}: ${url}`);
    const source = await response.text();
    inputs.files[logical(url)] = [...encoder.encode(source)];
    return source;
  }
  async function visit(url, root) {
    if (visited.has(url.href)) return;
    visited.add(url.href);
    const source = await read(url, root);
    if (source === undefined) return;
    const config = JSON.parse(configuration(source));
    if (!config.dependencies) throw new Error(config.diagnostics.map(diagnostic => `${url}: ${diagnostic.message}`).join("\n"));
    if (config.package) {
      const directory = new URL(".", url);
      roots.set(logical(directory).replace(/\/$/, "") || "/", directory);
      await read(new URL("lib.notc", directory));
    }
    const dependencies = root ? [...config.dependencies, ...config.dev_dependencies] : config.dependencies;
    for (const dependency of dependencies) {
      const directory = new URL(dependency.path.replace(/\/?$/, "/"), url);
      if (directory.origin !== configURL.origin) throw new Error("Package dependencies must use local paths on the configuration's origin");
      await visit(new URL("Notist.toml", directory), false);
    }
  }
  await visit(configURL, true);
  const description = JSON.parse(describe_prepared(JSON.stringify(inputs)));
  if (!description.functions) throw new Error(description.error ?? description.diagnostics.map(diagnostic => `${diagnostic.path}: ${diagnostic.message}`).join("\n"));
  for (const fn of description.functions) {
    const directory = roots.get(fn.root);
    const entries = fn.entries.map(path => new URL(path.split("/").map(encodeURIComponent).join("/"), directory));
    const available = await Promise.all(entries.map(async url => {
      const response = await fetcher(url, { method: "HEAD" });
      if (response.status === 404) return false;
      if (!response.ok) throw new Error(`${response.status}: ${url}`);
      return true;
    }));
    if (available.every(Boolean)) throw new Error(`Conflicting component entries: ${fn.package}::${fn.name}`);
    const entry = entries.find((_, index) => available[index]);
    if (entry) {
      inputs.files[logical(entry)] = [];
      inputs.module_urls[logical(entry)] = entry.href;
    }
  }
  return inputs;
}
