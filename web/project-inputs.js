// Prepare one document and the dependency graph declared by its source.
const encoder = new TextEncoder();
const logical = url => decodeURIComponent(url.pathname);
const sourceURL = (path, base) => new URL(path.split('/').map(encodeURIComponent).join('/'), base);
export const emptyInputs = () => ({ root: '/preview', config: null, files: {}, module_urls: {} });

export async function loadInputs(documentURL, { source_imports, describe_prepared, rootURL = new URL('.', documentURL), configURL = null, source }, fetcher = fetch) {
  const inputs = { root: logical(rootURL), config: configURL ? logical(configURL) : null, files: {}, module_urls: {} };
  const visited = new Set();
  const roots = new Map();
  async function read(url, required) {
    const response = await fetcher(url);
    if (response.status === 404 && !required) return;
    if (!response.ok) throw new Error(`${response.status}: ${url}`);
    return response.text();
  }
  async function visit(url, required, override) {
    if (visited.has(url.href)) return;
    visited.add(url.href);
    const text = override ?? await read(url, required);
    if (text === undefined) return;
    inputs.files[logical(url)] = [...encoder.encode(text)];
    roots.set(logical(new URL('.', url)).replace(/\/$/, '') || '/', new URL('.', url));
    const parsed = JSON.parse(source_imports(logical(url), text));
    if (!parsed.imports) throw new Error(parsed.error ?? parsed.diagnostics.map(d => `${url}: ${d.message}`).join('\n'));
    for (const dependency of parsed.imports) {
      if (dependency.path.startsWith('/') || dependency.path.startsWith('@') || dependency.path.split('/')[0].includes(':')) throw new Error('Imports require relative local module paths');
      await visit(sourceURL(dependency.path, url), false);
    }
  }
  try {
    if (configURL && !documentURL.pathname.endsWith('.notc')) await visit(configURL, true);
    await visit(documentURL, true, source);
    if (documentURL.pathname.endsWith('.notc')) return inputs;
    const description = JSON.parse(describe_prepared(logical(documentURL), JSON.stringify(inputs)));
    if (!description.functions) throw new Error(description.error ?? description.diagnostics.map(d => `${d.path}: ${d.message}`).join('\n'));
    for (const fn of description.functions) {
      const entries = fn.entries.map(path => sourceURL(path, roots.get(fn.root)));
      const available = await Promise.all(entries.map(async url => {
        const response = await fetcher(url, { method: 'HEAD' });
        if (response.status === 404) return false;
        if (!response.ok) throw new Error(`${response.status}: ${url}`);
        return true;
      }));
      if (available.every(Boolean)) throw new Error(`Conflicting component entries: ${fn.module}.${fn.name}`);
      const entry = entries.find((_, index) => available[index]);
      if (entry) {
        inputs.files[logical(entry)] = [];
        inputs.module_urls[logical(entry)] = entry.href;
      }
    }
    return inputs;
  } catch (error) {
    // Preserve this attempt's source view so recovery never uses an older graph.
    throw Object.assign(error, { inputs });
  }
}
