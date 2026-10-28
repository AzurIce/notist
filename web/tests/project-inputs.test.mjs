import assert from 'node:assert/strict';
import test from 'node:test';
import { loadInputs } from '../project-inputs.js';
const imports = (_path, source) => JSON.stringify({ imports: [...source.matchAll(/#?import "([^"]+)" as (\w+);/g)].map(([, path, alias]) => ({ path, alias })) });
const text = bytes => new TextDecoder().decode(Uint8Array.from(bytes));

test('document source imports deduplicate modules and preserve Unicode component paths', async () => {
  const sources = {
    '/root/doc.not': '#import "../a/widgets.notc" as a;\n#import "../d/lib.notc" as d;',
    '/a/widgets.notc': 'import "../shared/lib.notc" as shared; fn 图(text: String = "α<&") -> BlockContent;',
    '/d/lib.notc': 'import "../shared/lib.notc" as shared;',
    '/shared/lib.notc': 'fn value() -> String { "shared" }',
  };
  const requests = [];
  const inputs = await loadInputs(new URL('https://host.test/root/doc.not'), {
    source_imports: imports,
    describe_prepared: (path, source) => {
      const inputs = JSON.parse(source);
      assert.equal(path, '/root/doc.not');
      assert.equal(inputs.root, '/root/');
      assert.equal(inputs.config, null);
      assert.deepEqual(Object.keys(inputs.files).sort(), Object.keys(sources).sort());
      return JSON.stringify({ functions: [{ module: '/a/widgets.notc', root: '/a', name: '图', entries: ['图.js', '图/index.js'] }] });
    },
  }, async (url, options) => {
    const path = decodeURIComponent(url.pathname);
    requests.push([path, options?.method ?? 'GET']);
    if (path in sources) return new Response(sources[path]);
    return new Response(null, { status: path === '/a/图.js' ? 200 : 404 });
  });
  assert.equal(text(inputs.files['/a/widgets.notc']), sources['/a/widgets.notc']);
  assert.equal(requests.filter(([path]) => path === '/shared/lib.notc').length, 1);
  assert(!requests.some(([path]) => path.endsWith('Notist.toml')));
  assert.equal(inputs.module_urls['/a/图.js'], 'https://host.test/a/%E5%9B%BE.js');
});

test('missing imports reach Rust diagnostics; no neighboring lib.notc is discovered', async () => {
  await assert.rejects(loadInputs(new URL('https://host.test/doc.not'), {
    source_imports: imports,
    describe_prepared: (_path, source) => {
      assert.deepEqual(Object.keys(JSON.parse(source).files), ['/doc.not']);
      return JSON.stringify({ diagnostics: [{ path: '/doc.not', message: 'cannot import missing.notc' }] });
    },
  }, async url => new Response(url.pathname === '/doc.not' ? '#import "missing.notc" as missing;' : null, { status: url.pathname === '/doc.not' ? 200 : 404 })), /cannot import/);
});

test('source discovery terminates cycles and leaves semantic rejection to Rust', async () => {
  const requests = [];
  const inputs = await loadInputs(new URL('https://host.test/root/lib.notc'), {
    source_imports: imports,
    describe_prepared: () => { throw new Error('Code preparation must not bind HTML'); },
  }, async url => { requests.push(url.pathname); return new Response('import "./lib.notc" as recursive;'); });
  assert.deepEqual(Object.keys(inputs.files), ['/root/lib.notc']);
  assert.deepEqual(requests, ['/root/lib.notc']);
});

test('literal filesystem paths preserve URL punctuation', async () => {
  const urls = [];
  await loadInputs(new URL('https://host.test/notes/doc.not'), {
    source_imports: imports,
    describe_prepared: (_path, source) => {
      assert.equal(text(JSON.parse(source).files['/notes/视图 #%.notc']), 'fn f() -> Int { 1 }');
      return JSON.stringify({ functions: [] });
    },
  }, async url => {
    urls.push(url.href);
    return new Response(url.pathname === '/notes/doc.not' ? '#import "./视图 #%.notc" as view;' : 'fn f() -> Int { 1 }');
  });
  assert.equal(urls[1], 'https://host.test/notes/%E8%A7%86%E5%9B%BE%20%23%25.notc');
});

test('edited source replaces its import graph and output config supplies no source entry', async () => {
  const requests = [];
  const inputs = await loadInputs(new URL('https://host.test/notes/doc.md'), {
    configURL: new URL('https://host.test/notes/Notist.toml'),
    source: '#import "chosen.notc" as chosen;',
    source_imports: imports,
    describe_prepared: (_path, source) => {
      const files = JSON.parse(source).files;
      assert.equal(text(files['/notes/doc.md']), '#import "chosen.notc" as chosen;');
      assert(!('/notes/lib.notc' in files));
      return JSON.stringify({ functions: [] });
    },
  }, async url => { requests.push(url.pathname); return new Response(''); });
  assert.deepEqual(requests, ['/notes/Notist.toml', '/notes/chosen.notc']);
  assert.equal(inputs.config, '/notes/Notist.toml');
});

test('configuration imports are relative to config and share document module requests', async () => {
  const requests = [];
  const sources = {
    '/settings/Notist.toml': 'output rules',
    '/notes/doc.not': '#import "../modules/output.notc" as user;',
    '/modules/output.notc': 'import "helper.notc" as helper;',
    '/modules/helper.notc': 'fn value() -> String { "ok" }',
  };
  const inputs = await loadInputs(new URL('https://host.test/notes/doc.not'), {
    configURL: new URL('https://host.test/settings/Notist.toml'),
    source_imports: (path, source) => path.endsWith('.toml')
      ? JSON.stringify({ imports: [
        { path: '../modules/output.notc', alias: 'target' },
        { path: '../modules/./output.notc', alias: 'target' },
      ] })
      : imports(path, source),
    describe_prepared: () => JSON.stringify({ functions: [] }),
  }, async url => {
    requests.push(url.pathname);
    return new Response(sources[url.pathname]);
  });
  assert.equal(requests.filter(path => path === '/modules/output.notc').length, 1);
  assert.deepEqual(Object.keys(inputs.files).sort(), Object.keys(sources).sort());
});

test('Code entry preparation skips output configuration and HTML discovery', async () => {
  const requests = [];
  await loadInputs(new URL('https://host.test/module.notc'), {
    configURL: new URL('https://host.test/Notist.toml'),
    source_imports: imports,
    describe_prepared: () => { throw new Error('Code preparation must not bind HTML'); },
  }, async url => { requests.push(url.pathname); return new Response('fn f() -> InlineContent;'); });
  assert.deepEqual(requests, ['/module.notc']);
});

test('HTML discovery failures retain the edited source graph for recovery', async () => {
  const source = '#import "changed.notc" as changed;\n#changed.view()';
  await assert.rejects(loadInputs(new URL('https://host.test/doc.not'), {
    source,
    source_imports: imports,
    describe_prepared: () => JSON.stringify({ functions: [{ module: '/changed.notc', root: '/', name: 'view', entries: ['view.js', 'view/index.js'] }] }),
  }, async (url, options) => options?.method === 'HEAD'
    ? new Response(null, { status: 503 })
    : new Response('fn view() -> InlineContent;')),
  error => {
    assert.match(error.message, /503/);
    assert.equal(text(error.inputs.files['/doc.not']), source);
    assert.equal(text(error.inputs.files['/changed.notc']), 'fn view() -> InlineContent;');
    assert.deepEqual(Object.keys(error.inputs.files).sort(), ['/changed.notc', '/doc.not']);
    return true;
  });
});

test('member and wildcard selections discover the same canonical dependency graph', async () => {
  const sources = {
    '/root/doc.md': '#import "parts/issue.notc": issue as ticket;\n#import "parts/issue.notc": *;',
    '/root/parts/issue.notc': 'import "helper.notc": message; fn issue() -> InlineContent { [#message()] }',
    '/root/parts/helper.notc': 'fn message() -> InlineContent { [hello] }',
  };
  const requests = [];
  const inputs = await loadInputs(new URL('https://host.test/root/doc.md'), {
    // Selection and ranges are supplied by Rust. Discovery reads only paths.
    source_imports: path => JSON.stringify({ imports: path.endsWith('doc.md') ? [
      { path: 'parts/issue.notc', kind: 'members', members: [{ name: 'issue', alias: 'ticket', start: 33, end: 48 }] },
      { path: 'parts/issue.notc', kind: 'wildcard' },
    ] : path.endsWith('issue.notc') ? [
      { path: 'helper.notc', kind: 'members', members: [{ name: 'message', alias: 'message' }] },
    ] : [] }),
    describe_prepared: () => JSON.stringify({ functions: [{ module: '/root/parts/issue.notc', root: '/root/parts', name: 'issue', entries: ['issue.js', 'issue/index.js'] }] }),
  }, async (url, options) => {
    requests.push([url.pathname, options?.method ?? 'GET']);
    if (url.pathname in sources) return new Response(sources[url.pathname]);
    return new Response(null, { status: url.pathname === '/root/parts/issue.js' ? 200 : 404 });
  });
  assert.equal(requests.filter(([path, method]) => path === '/root/parts/issue.notc' && method === 'GET').length, 1);
  assert.equal(text(inputs.files['/root/parts/helper.notc']), sources['/root/parts/helper.notc']);
  assert.equal(inputs.module_urls['/root/parts/issue.js'], 'https://host.test/root/parts/issue.js');
});
