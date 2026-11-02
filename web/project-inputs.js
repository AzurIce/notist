// Prepare one immutable document input batch. Language evaluation stays sync;
// only this authorized host adapter fetches missing runtime resource inputs.
const encoder = new TextEncoder();
const logical = url => decodeURIComponent(url.pathname);
const sourceURL = (path, base) => new URL(path.split('/').map(encodeURIComponent).join('/'), base);
export const emptyInputs = () => ({ root: '/preview', config: null, files: {}, module_urls: {}, failures: {}, incomplete: false });

const canonicalRoot = root => {
  const parts = root.split('/').filter(Boolean);
  if (!root.startsWith('/') || parts.some(part => part === '.' || part === '..' || /[\\\u0000-\u001f\u007f]/.test(part))) throw new Error('Invalid logical resource root');
  return `/${parts.join('/')}`;
};
const contained = (root, path) => root === '/' || path === root || path.startsWith(`${root}/`);

async function boundedBytes(response, limit) {
  if (Number(response.headers.get('content-length')) > limit) {
    await response.body?.cancel();
    throw Object.assign(new Error('Resource size limit exceeded'), { failure: 'TooLarge' });
  }
  if (!response.body) return [];
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) {
        await reader.cancel();
        throw Object.assign(new Error('Resource size limit exceeded'), { failure: 'TooLarge' });
      }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return [...bytes];
}

export async function readSourceResponse(response, limit = 16 * 1024 * 1024) {
  return new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(await boundedBytes(response, limit)));
}

export async function loadInputs(documentURL, {
  source_imports, describe_prepared, rootURL = new URL('.', documentURL), configURL = null, source,
  resourceRoots = [], signal, maxRequests = 1024, maxRounds = 32,
  maxFileBytes = 16 * 1024 * 1024, maxTotalBytes = 128 * 1024 * 1024,
}, fetcher = fetch) {
  const mappings = [{ root: logical(rootURL), url: rootURL }, ...resourceRoots].map(mapping => ({
    root: canonicalRoot(mapping.root), url: new URL(mapping.url),
  })).sort((a, b) => b.root.length - a.root.length);
  for (const mapping of mappings) {
    if (!['http:', 'https:'].includes(mapping.url.protocol) || !mapping.url.pathname.endsWith('/') || mapping.url.search || mapping.url.hash) throw new Error('Resource mappings require an HTTP(S) directory URL');
  }
  if (new Set(mappings.map(mapping => mapping.root)).size !== mappings.length) throw new Error('Duplicate logical resource mapping');
  const inputs = { root: logical(rootURL), config: configURL ? logical(configURL) : null, files: {}, module_urls: {}, failures: {}, incomplete: true, read_roots: mappings.map(mapping => mapping.root) };
  const visited = new Set();
  const roots = new Map();
  let requests = 0;
  let totalBytes = 0;
  const check = () => { if (signal?.aborted) throw new DOMException('Input preparation superseded', 'AbortError'); };
  async function request(url, options = {}) {
    check();
    if (++requests > maxRequests) throw new Error('Input preparation request limit exceeded');
    const response = await fetcher(url, { ...options, signal, redirect: 'error' });
    check();
    return response;
  }
  const store = (path, bytes) => {
    if (bytes.length > maxFileBytes) throw Object.assign(new Error('Resource size limit exceeded'), { failure: 'TooLarge' });
    if (totalBytes + bytes.length > maxTotalBytes) throw new Error('Input preparation cache limit exceeded');
    inputs.files[path] = bytes;
    delete inputs.failures[path];
    totalBytes += bytes.length;
  };
  async function read(url, required) {
    const response = await request(url);
    if (response.status === 404 && !required) { inputs.failures[logical(url)] = 'NotFound'; return; }
    if (!response.ok) throw new Error(`${response.status}: ${url}`);
    return boundedBytes(response, Math.min(maxFileBytes, maxTotalBytes - totalBytes));
  }
  async function visit(url, required, override) {
    if (visited.has(url.href)) return;
    visited.add(url.href);
    if (override !== undefined && override.length > maxFileBytes) throw Object.assign(new Error('Resource size limit exceeded'), { failure: 'TooLarge' });
    const bytes = override === undefined ? await read(url, required) : [...encoder.encode(override)];
    if (bytes === undefined) return;
    store(logical(url), bytes);
    const text = new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(bytes));
    roots.set(logical(new URL('.', url)).replace(/\/$/, '') || '/', new URL('.', url));
    const parsed = JSON.parse(source_imports(logical(url), text));
    if (!parsed.imports) throw new Error(parsed.error ?? parsed.diagnostics.map(d => `${url}: ${d.message}`).join('\n'));
    for (const dependency of parsed.imports) {
      if (dependency.path.startsWith('/') || dependency.path.startsWith('@') || dependency.path.split('/')[0].includes(':')) throw new Error('Imports require relative local module paths');
      await visit(sourceURL(dependency.path, url), false);
    }
  }
  async function prepareResource(resource) {
    if (typeof resource.root !== 'string' || typeof resource.path !== 'string' || resource.path.split('/').some(part => !part || part === '.' || part === '..' || /[\\\u0000-\u001f\u007f]/.test(part))) throw new Error('Invalid requested resource identity');
    const root = canonicalRoot(resource.root);
    const path = `${root === '/' ? '' : root}/${resource.path}`;
    if (Object.hasOwn(inputs.files, path) || Object.hasOwn(inputs.failures, path)) return false;
    const mapping = mappings.find(mapping => contained(mapping.root, path));
    if (!mapping) { inputs.failures[path] = 'AccessDenied'; return true; }
    if (path === mapping.root) { inputs.failures[path] = 'NotFile'; return true; }
    const relative = path.slice(mapping.root === '/' ? 1 : mapping.root.length + 1);
    const url = sourceURL(relative, mapping.url);
    try {
      const response = await request(url);
      if (!response.ok) {
        inputs.failures[path] = response.status === 404 ? 'NotFound' : [401, 403].includes(response.status) ? 'AccessDenied' : { Io: `HTTP ${response.status}` };
      } else {
        const limit = Math.min(maxFileBytes, maxTotalBytes - totalBytes);
        let bytes;
        try { bytes = await boundedBytes(response, limit); }
        catch (error) {
          if (error.failure === 'TooLarge' && limit < maxFileBytes) throw new Error('Input preparation cache limit exceeded');
          throw error;
        }
        check();
        store(path, bytes);
      }
    } catch (error) {
      check();
      if (error.name === 'AbortError' || requests > maxRequests || /cache limit/.test(error.message)) throw error;
      inputs.failures[path] = error.failure ?? { Io: error.message };
    }
    return true;
  }
  try {
    if (configURL && !documentURL.pathname.endsWith('.notc')) await visit(configURL, true);
    await visit(documentURL, true, source);
    if (documentURL.pathname.endsWith('.notc')) return inputs;
    let description;
    for (let round = 0; ; round++) {
      check();
      description = JSON.parse(describe_prepared(logical(documentURL), JSON.stringify(inputs)));
      if (!description.need_resources) break;
      if (round >= maxRounds) throw new Error('Input preparation retry limit exceeded');
      let progress = false;
      for (const resource of description.need_resources) progress = await prepareResource(resource) || progress;
      if (!progress) throw new Error('Input preparation made no progress');
    }
    if (!description.functions) throw new Error(description.error ?? description.diagnostics.map(d => `${d.path}: ${d.message}`).join('\n'));
    for (const fn of description.functions) {
      const entries = fn.entries.map(path => sourceURL(path, roots.get(fn.root)));
      const available = await Promise.all(entries.map(async url => {
        const response = await request(url, { method: 'HEAD' });
        if (response.status === 404) return false;
        if (!response.ok) throw new Error(`${response.status}: ${url}`);
        return true;
      }));
      if (available.every(Boolean)) throw new Error(`Conflicting component entries: ${fn.module}.${fn.name}`);
      const entry = entries.find((_, index) => available[index]);
      if (entry) {
        // URL availability is not an empty file or a data-read capability.
        inputs.module_urls[logical(entry)] = entry.href;
      }
    }
    return inputs;
  } catch (error) {
    // Preserve this attempt's source view so recovery never uses an older graph.
    throw Object.assign(error, { inputs });
  }
}
