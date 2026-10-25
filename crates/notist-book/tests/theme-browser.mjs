// Real-browser smoke tests, without npm dependencies (Node 22+ and Chromium).
// Build the grammar module WASM first (modules/grammar/justfile), then:
// cargo build -p notist-cli && node crates/notist-book/tests/theme-browser.mjs
// Optional: CHROMIUM, NOTIST_BIN, THEME_SCREENSHOT_DIR.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
await mkdir(join(repository, 'target'), { recursive: true });
const temporary = await mkdtemp(join(repository, 'target/theme-browser-'));
const vault = join(temporary, 'vault');
const output = join(temporary, 'site');
const screenshots = process.env.THEME_SCREENSHOT_DIR;
let browser;
let socket;
let server;
const checks = [];
const check = (name, value) => { assert.ok(value, name); checks.push(name); };

try {
  await mkdir(join(vault, 'notes'), { recursive: true });
  await mkdir(join(vault, '_logs'), { recursive: true });
  await mkdir(join(vault, 'empty'), { recursive: true });
  await writeFile(join(vault, 'Notist.toml'), `[book]\ntitle = "Field Notes"\nadditional-css = ["custom/first.css", "custom/last.css"]\nadditional-js = ["custom/first.js", "custom/last.js"]\n`);
  await mkdir(join(vault, 'custom'), { recursive: true });
  await writeFile(join(vault, 'custom/first.css'), ':root[data-brand="custom"] { --accent: #6d28d9; }');
  await writeFile(join(vault, 'custom/last.css'), ':root[data-brand="custom"] { --accent: #7040b8; }');
  await writeFile(join(vault, 'custom/first.js'), 'window.readerExtensions = ["first"]; window.themeReady = document.documentElement.classList.contains("js");');
  await writeFile(join(vault, 'custom/last.js'), 'window.readerExtensions.push("last");');
  await writeFile(join(vault, 'lib.notc'), `import ${JSON.stringify(relative(vault, join(repository, 'modules/grammar/lib.notc')))} as grammar;`);
  await writeFile(join(vault, 'README.not'), `= 文字、代码与思考

好的工具让表达变得自然。这里记录关于语言、软件与写作的思考：让技术清晰，也让文字保有自己的节奏。

== 从一篇文章开始

我们相信，文档不只是功能的说明。它也可以是一场有条理的对话：先交代问题，再展开推理，最后留下值得继续探索的线索。

[阅读文章](notes/essay.md) · [阅读原生文档](notes/native.not)

== 简单，但不单薄

- 留白不是空缺，而是给思考留出空间。
- 代码与文字并置，各自保持清晰。
- 阅读路径明确，界面退到内容之后。

== 写下来，慢慢想

一段文字，一小段代码，一次更清楚的理解。构建工具应当让这些事情变得容易，而不是让它们看起来复杂。
`);
  const paragraph = 'Good software leaves room for thought. A small, well-defined interface is easier to understand than a clever one with hidden rules. We can build useful tools without making the reader carry all of their complexity.';
  await writeFile(join(vault, 'notes/essay.md'), `# A quieter way to build

Notes on clear interfaces, deliberate choices, and the space between prose and code.

## Begin with the reader

${paragraph}

> Simplicity is not the absence of detail. It is knowing which details deserve our attention.

${paragraph}

## A small, explicit contract

A document should say what it means. Its interface should make the common case easy, and leave the unusual case visible.

\`\`\`rust
pub fn render(document: &Document) -> Result<Page, Error> {
    let content = analyze(document)?;
    let html = Renderer::new().render(&content);
    Ok(Page { html })
}
// A deliberately long line verifies that code scrolls inside its own container: ${'readable_'.repeat(18)}
\`\`\`

### Trade-offs worth naming

${paragraph}

| Layer | Responsibility | Boundary |
| --- | --- | --- |
| Content | Analysis and transformation | No filesystem publication |
| Theme | Typography and navigation | No document semantics |
| Host | Resources and publication | No hidden content mutation |

## 留白，也是设计

技术文章需要准确，但不必因此失去温度。好的排版让论证更容易被理解，也让读者有余地停下来，想一想刚刚读过的内容。

${paragraph}

${paragraph}

## Keep the next step small

- Prefer one coherent rule over several exceptions.
- Validate the page as a reader would use it.
- Leave the next person a clear place to begin.

${paragraph}

${paragraph}

Read the [native document](native.not#中文%20%26%3F%23%2F) or return [home](../README.not).
`);
  await writeFile(join(vault, 'notes/native.not'), `@(id: "中文 &?#/")
= 原生文档与特殊锚点

这是一个原生 Notist 页面，用来验证中文排版与锚点定位。

== 技术细节

代码应该保持可选择、可复制，也应该在窄屏上独立滚动。

\`\`\`text
hello, notist
\`\`\`
`);
  await writeFile(join(vault, 'raw.not'), `= Raw and grammar source

\`\`\`signature
text(text: String) -> InlineContent
\`\`\`

#import "${relative(vault, join(repository, 'modules/grammar/lib.notc'))}" as grammar;\n#grammar.diagram(r#"""
lex Token -> \`word\` | \`(\` Token \`)\`;
syntax PlainText -> Token;
"""#)
`);
  await writeFile(join(vault, 'plain.md'), 'A page without headings still has a complete reading surface.');
  await writeFile(join(vault, 'notes/README.md'), '# Notes\n\nWriting and examples.');
  const grammarImport = `#import ${JSON.stringify(relative(vault, join(repository, 'modules/grammar/lib.notc')))} as grammar;`;
  const brokenGrammar = 'syntax Broken -> (';
  const unsafeGrammar = 'syntax Safe -> `<script>globalThis.grammarInjected = true</script>`;';
  await writeFile(join(vault, 'broken.not'), `= Invalid grammar\n\n${grammarImport}\n#grammar.diagram(${JSON.stringify(brokenGrammar)})`);
  await writeFile(join(vault, 'unsafe.not'), `= Literal grammar source\n\n${grammarImport}\n#grammar.diagram(${JSON.stringify(unsafeGrammar)})`);
  await writeFile(join(vault, '_logs/entry.md'), '# Archive entry\n\nA short log entry.');
  for (let index = 1; index <= 45; index++) {
    const suffix = String(index).padStart(2, '0');
    await writeFile(join(vault, `_logs/more${suffix}.md`), `# Archive ${suffix}\n\nAnother log entry.`);
  }
  await writeFile(join(vault, 'SUMMARY.not'), `= Summary

[Home](README.not)

= Writing

- [Notes](notes/)
  - [Essay](notes/essay.md)
  - [Native](notes/native.not)

= Reference

- [Raw](raw.not)
- [Plain](plain.md)
- [Broken](broken.not)
- [Unsafe](unsafe.not)
- [Future chapter]()

---

= Archive

- @(collapsed: true)[History](_logs/)
- [Empty](empty/)
`);
  const build = spawnSync(process.env.NOTIST_BIN || join(repository, 'target/debug/notist'), ['book', 'build', vault, '--out-dir', output], { encoding: 'utf8' });
  assert.equal(build.status, 0, build.stderr || build.error?.message);

  // Serve the exact generated artifact on loopback, including nested clean URLs.
  server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      const file = resolve(output, `.${pathname}${pathname.endsWith('/') ? 'index.html' : ''}`);
      if (!file.startsWith(`${output}${sep}`)) throw new Error('Invalid path');
      const mime = file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'text/javascript' : file.endsWith('.wasm') ? 'application/wasm' : 'text/html';
      const bytes = await readFile(file);
      response.writeHead(200, { 'Content-Type': mime });
      response.end(bytes);
    } catch {
      response.writeHead(404);
      response.end('Not found');
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = spawn(process.env.CHROMIUM || 'chromium', ['--headless', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu', '--remote-debugging-port=0', `--user-data-dir=${join(temporary, 'browser')}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const endpoint = await new Promise((resolve, reject) => {
    let log = '';
    const timer = setTimeout(() => reject(new Error(`Chromium startup timed out: ${log}`)), 15000);
    browser.stderr.on('data', data => {
      log += data;
      const match = log.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
    browser.once('error', error => { clearTimeout(timer); reject(error); });
    browser.once('exit', code => { clearTimeout(timer); reject(new Error(`Chromium exited ${code}: ${log}`)); });
  });
  socket = new WebSocket(endpoint);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  let sequence = 0;
  const pending = new Map();
  const exceptions = [];
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') exceptions.push(message.params.exceptionDetails);
    const handler = pending.get(message.id);
    if (handler) {
      pending.delete(message.id);
      clearTimeout(handler.timer);
      if (message.error) handler.reject(new Error(message.error.message));
      else handler.resolve(message.result);
    }
  });
  const command = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timed out: ${method}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const { targetId } = await command('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await command('Target.attachToTarget', { targetId, flatten: true });
  const cdp = (method, params) => command(method, params, sessionId);
  await cdp('Page.enable');
  await cdp('Runtime.enable');
  const evaluate = async expression => {
    const result = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
      .catch(error => { throw new Error(`Evaluation failed: ${expression}`, { cause: error }); });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text + ': ' + result.exceptionDetails.exception?.description);
    return result.result.value;
  };
  // A condition wait is for browser navigation/layout, not background-agent polling.
  const waitFor = async expression => {
    const end = Date.now() + 10000;
    while (Date.now() < end) {
      if (await evaluate(expression)) return;
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    throw new Error(`Browser condition timed out: ${expression}`);
  };
  const viewport = async (width, height = 960) => {
    await cdp('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: width <= 760 });
    // Media-query change handlers and responsive layout settle on the next frame.
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
  };
  const navigate = async (path, javascript = true) => {
    await cdp('Emulation.setScriptExecutionDisabled', { value: !javascript });
    const url = `${origin}${path}`;
    await cdp('Page.navigate', { url });
    await waitFor(`location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`);
    if (javascript) await evaluate('document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))');
  };
  const screenshot = async (name, clip) => {
    if (!screenshots) return;
    await mkdir(screenshots, { recursive: true });
    await evaluate('Promise.all(document.getAnimations().filter(animation => animation.effect.getComputedTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => {})))');
    const { data } = await cdp('Page.captureScreenshot', { format: 'png', ...(clip && { clip: { ...clip, scale: 2 } }) });
    await writeFile(join(screenshots, `${name}.png`), Buffer.from(data, 'base64'));
  };
  const noOverflow = () => evaluate('document.documentElement.scrollWidth <= window.innerWidth');
  const navigationPosition = () => evaluate(`(() => {
    const navigation = document.querySelector('#book-nav');
    const current = navigation.querySelector('[aria-current=page]');
    return {scroll: navigation.scrollTop, top: current.getBoundingClientRect().top};
  })()`);
  const pressSpace = async () => {
    await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
    await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
  };
  const checkDisclosureAlignment = async name => {
    await evaluate(`(() => {
      const summary = document.querySelector('#book-nav summary');
      const title = summary.querySelector('.nav-title');
      const number = summary.querySelector('.nav-number');
      window.disclosureSample = {summary, title, number, text: title.textContent, digits: number.textContent};
      number.textContent = '11.3.';
      title.textContent = '文档管线';
    })()`);
    const measure = () => evaluate(`(() => {
      const {summary, title} = window.disclosureSample;
      const icon = getComputedStyle(summary, '::after');
      const text = title.getClientRects();
      const box = summary.getBoundingClientRect();
      const iconCenter = box.top + parseFloat(icon.marginTop) + parseFloat(icon.height) / 2;
      return {delta: Math.abs(iconCenter - (text[0].top + text[0].height / 2)), lines: text.length,
        clip: {x: box.x, y: box.y, width: box.width, height: box.height}};
    })()`);
    const single = await measure();
    await screenshot(`theme-nav-${name}-single`, single.clip);
    await evaluate('window.disclosureSample.title.textContent = "文档管线：用于文档分析、求值和输出的完整流程"');
    const wrapped = await measure();
    await screenshot(`theme-nav-${name}-wrapped`, wrapped.clip);
    check(`${name} disclosure aligns with the first text line for short and wrapped titles`,
      single.lines === 1 && wrapped.lines > 1 && single.delta < 2 && wrapped.delta < 2);
    await evaluate('const sample = window.disclosureSample; sample.title.textContent = sample.text; sample.number.textContent = sample.digits; delete window.disclosureSample');
  };
  const checkNavigationPageStates = async name => {
    check(`${name} navigation uses muted text for chapters without pages and reserves emphasis for the current page`, await evaluate(`(() => {
      const navigation = document.querySelector('#book-nav');
      const labels = [...navigation.querySelectorAll('.nav-title')];
      const label = title => labels.find(label => label.textContent === title).parentElement;
      const page = label('Notes');
      const directory = label('History');
      const empty = label('Empty');
      const draft = label('Future chapter');
      const current = navigation.querySelector('[aria-current=page]');
      const child = directory.closest('details').querySelector('.nav-children a');
      const text = element => element.textContent.replace(/\\s+/g, ' ').trim();
      const tone = element => getComputedStyle(element).color;
      return page.matches('a[href]') && directory.matches('.nav-chapter') &&
        draft.matches('[role=link][aria-disabled=true]') &&
        text(directory) === '7. History' && text(empty) === '8. Empty' && text(draft) === '6. Future chapter' &&
        tone(page) !== tone(directory) && tone(directory) === tone(empty) && tone(empty) === tone(draft) &&
        tone(child) === tone(page) && tone(current) !== tone(page) &&
        getComputedStyle(current).backgroundColor !== 'rgba(0, 0, 0, 0)';
    })()`));
  };

  await viewport(1440);
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'light' }] });
  await navigate('/');
  check('declared top-level chapters remain peers and Home is a leaf', await evaluate(`(() => {
    const items = [...document.querySelectorAll('.navigation-tree > .nav-item')];
    const labels = items.map(item => item.querySelector('.nav-title').textContent);
    return labels.join(',') === 'Home,Notes,Raw,Plain,Broken,Unsafe,Future chapter,History,Empty' && !items[0].querySelector('details');
  })()`));
  check('part titles are visual headings rather than chapter parents or links', await evaluate(`(() => {
    const parts = [...document.querySelectorAll('.navigation-tree > .nav-part')];
    return parts.map(part => part.textContent.trim()).join(',') === 'Writing,Reference,Archive' &&
      parts.every(part => part.querySelector('[role=heading]') && !part.querySelector('a, details, ul'));
  })()`));
  check('prefix is unnumbered and chapter numbering continues across parts', await evaluate(`(() => {
    const labels = [...document.querySelectorAll('#book-nav .nav-title')];
    const number = title => labels.find(label => label.textContent === title).parentElement.querySelector('.nav-number')?.textContent;
    return number('Home') === undefined && number('Notes') === '1.' && number('Essay') === '1.1.' &&
      number('Native') === '1.2.' && number('Raw') === '2.' && number('Future chapter') === '6.';
  })()`));
  check('drafts are disabled links and separators render a rule', await evaluate('document.querySelector("#book-nav .nav-draft[role=link][aria-disabled=true]").textContent.includes("Future chapter") && document.querySelectorAll(".navigation-tree > .nav-separator > hr").length === 1'));
  check('discovered directories are numbered chapter parents without requiring a home page', await evaluate(`(() => {
    const archive = [...document.querySelectorAll('#book-nav details')].at(-1);
    return archive.querySelector(':scope > summary .nav-chapter .nav-number').textContent === '7.' &&
      archive.querySelector('.nav-children .nav-number').textContent === '7.1.';
  })()`));
  await checkNavigationPageStates('desktop');
  await checkDisclosureAlignment('desktop');
  const disclosure = await evaluate('(() => { const rect = document.querySelector("#book-nav summary").getBoundingClientRect(); return { x: rect.right - 22, y: rect.top + rect.height / 2 }; })()');
  await cdp('Input.dispatchMouseEvent', { type: 'mousePressed', ...disclosure, button: 'left', clickCount: 1 });
  await cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', ...disclosure, button: 'left', clickCount: 1 });
  check('clicking the arrow beside a title link collapses without navigating', await evaluate('!document.querySelector("#book-nav details").open && location.pathname === "/"'));
  await evaluate('document.querySelector("#book-nav summary").focus()');
  await pressSpace();
  check('unannotated branches start open and the archive starts closed', await evaluate(`(() => {
    const branches = [...document.querySelectorAll('#book-nav details')];
    return branches.find(branch => branch.querySelector(':scope > summary .nav-title')?.textContent === 'Notes').open &&
      !branches.find(branch => branch.querySelector(':scope > summary .nav-title')?.textContent === 'History').open;
  })()`));
  await evaluate(`window.archive = [...document.querySelectorAll('#book-nav details')].find(branch => branch.querySelector(':scope > summary .nav-title')?.textContent === 'History'); archive.querySelector('summary').click()`);
  check('clicking an unlinked directory chapter opens its children', await evaluate('archive.open && archive.querySelector("a").checkVisibility()'));
  await evaluate('archive.querySelector("summary").focus()');
  await pressSpace();
  check('keyboard Space collapses a branch', await evaluate('!archive.open'));
  await pressSpace();
  check('keyboard Space expands a branch', await evaluate('archive.open'));
  await evaluate('archive.querySelector("a").click()');
  await waitFor('location.pathname === "/_logs/entry/" && document.readyState === "complete"');
  check('visiting an archive page automatically opens its ancestors', await evaluate('[...document.querySelectorAll("#book-nav details")].every(branch => branch.open) && document.querySelector("#book-nav [aria-current=page]").checkVisibility()'));
  await navigate('/');
  check('branch state resets to its declaration on navigation', await evaluate('![...document.querySelectorAll("#book-nav details")].at(-1).open'));
  check('expanding a long archive preserves navigation width and content position', await evaluate(`(() => {
    const navigation = document.querySelector('#book-nav');
    const tree = navigation.querySelector('.navigation-tree');
    const archive = [...navigation.querySelectorAll('details')].at(-1);
    const children = archive.querySelector('.nav-children');
    const extra = Array.from({length: 40}, (_, index) => {
      const item = document.createElement('li');
      item.className = 'nav-item';
      const label = document.createElement('span');
      label.className = 'nav-group';
      label.textContent = 'Older entry ' + index;
      item.append(label);
      children.append(item);
      return item;
    });
    const measure = () => [navigation.clientWidth, tree.getBoundingClientRect().width,
      navigation.querySelector('summary').getBoundingClientRect().right,
      document.querySelector('.prose').getBoundingClientRect().left];
    const before = measure();
    const closedFits = navigation.scrollHeight <= navigation.clientHeight;
    archive.open = true;
    const opened = measure();
    const overflows = navigation.scrollHeight > navigation.clientHeight;
    navigation.scrollTop = navigation.scrollHeight;
    const scrolls = navigation.scrollTop > 0;
    archive.open = false;
    navigation.scrollTop = 0;
    const closed = measure();
    extra.forEach(item => item.remove());
    return closedFits && overflows && scrolls && before.every((value, i) =>
      Math.abs(value - opened[i]) < .5 && Math.abs(value - closed[i]) < .5);
  })()`));
  check('book identity comes from configuration without a publisher brand', await evaluate('document.querySelector(".book-brand").textContent === "Field Notes" && !document.querySelector(".footer-brand")'));
  check('additional scripts run after the theme in declaration order', await evaluate('themeReady && readerExtensions.join(",") === "first,last"'));
  await evaluate('document.documentElement.dataset.brand = "custom"');
  check('additional CSS overrides public theme variables in declaration order', await evaluate('getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() === "#7040b8"'));
  await evaluate('delete document.documentElement.dataset.brand');
  check('native CJK article retains a single title', await evaluate('document.querySelectorAll(".prose h1").length === 1'));
  check('theme chrome is icon-free and controls have visible text', await evaluate('!document.querySelector(".book-header svg, .rail-heading svg, .mobile-outline summary svg, .book-mark") && document.querySelector("#nav-toggle").textContent.trim() === "Contents" && document.querySelector("#nav-close").textContent.trim() === "Close" && document.querySelector("#theme-toggle").textContent.trim() === "Dark"'));
  await screenshot('theme-native-desktop');
  await navigate('/notes/essay/');
  check('breadcrumbs follow declared ancestry without adding Home', await evaluate('[...document.querySelectorAll(".breadcrumbs li")].map(item => item.textContent).join(",") === "Notes,Essay"'));
  check('nested chapter links are visibly indented beneath their parent', await evaluate(`(() => {
    const item = document.querySelector('#book-nav [aria-current=page]');
    const parent = item.closest('.nav-children').previousElementSibling.querySelector('a');
    return item.getBoundingClientRect().left > parent.getBoundingClientRect().left;
  })()`));
  check('desktop has no page-wide overflow', await noOverflow());
  check('desktop outline and native code controls are present', await evaluate('getComputedStyle(document.querySelector(".page-outline")).display !== "none" && document.querySelectorAll(".copy-code").length === 1'));
  check('long code scrolls inside its pre', await evaluate('document.querySelector("pre").scrollWidth > document.querySelector("pre").clientWidth'));
  check('nested assets loaded correctly', await evaluate('getComputedStyle(document.body).backgroundColor === "rgb(255, 255, 255)" && document.documentElement.classList.contains("js")'));
  // Rust Reference uses 16px body, 32/24/18.72px headings, 14px code and a 750px measure.
  // Use 1.6 leading rather than its 1.45, to leave a little more room for CJK glyphs.
  check('typography follows the reference scale with CJK-friendly leading', await evaluate(`(() => {
    const style = selector => getComputedStyle(document.querySelector(selector));
    return parseFloat(style('.prose').fontSize) === 16 &&
      Math.abs(parseFloat(style('.prose').lineHeight) - 25.6) < .1 &&
      style('.prose').fontFamily.endsWith('sans-serif') &&
      parseFloat(style('.prose h1').fontSize) === 32 &&
      parseFloat(style('.prose h2').fontSize) === 24 &&
      Math.abs(parseFloat(style('.prose h3').fontSize) - 18.72) < .1 &&
      parseFloat(style('.prose pre code').fontSize) === 14 &&
      document.querySelector('.prose').getBoundingClientRect().width <= 750;
  })()`));
  check('the introductory paragraph uses ordinary body typography', await evaluate(`(() => {
    const prose = getComputedStyle(document.querySelector('.prose'));
    const intro = getComputedStyle(document.querySelector('.prose h1 + p'));
    return intro.fontSize === prose.fontSize && intro.color === prose.color && intro.fontWeight === '400';
  })()`));
  const palette = await evaluate('Object.fromEntries(["muted", "bg", "code", "accent", "surface"].map(key => [key, getComputedStyle(document.documentElement).getPropertyValue(`--${key}`).trim()]))');
  const luminance = hex => {
    const [r, g, b] = hex.match(/[a-f\d]{2}/gi).map(value => parseInt(value, 16) / 255)
      .map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4);
    return r * .2126 + g * .7152 + b * .0722;
  };
  const contrast = (foreground, background) => {
    const values = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
    return (values[0] + .05) / (values[1] + .05);
  };
  check('secondary text and code controls meet 4.5:1 contrast', contrast(palette.muted, palette.bg) >= 4.5 && contrast(palette.muted, palette.code) >= 4.5);
  check('active navigation meets 4.5:1 contrast', contrast(palette.accent, palette.surface) >= 4.5);
  await screenshot('theme-desktop');

  await evaluate('document.getElementById("留白-也是设计").scrollIntoView()');
  await waitFor('document.querySelector("#page-outline [aria-current=location]")?.hash === "#%E7%95%99%E7%99%BD-%E4%B9%9F%E6%98%AF%E8%AE%BE%E8%AE%A1"');
  check('outline tracks the current section', true);
  check('reading progress advances', await evaluate('getComputedStyle(document.querySelector("#reading-progress span")).transform !== "matrix(0, 0, 0, 1, 0, 0)"'));
  await evaluate('window.scrollTo(0, 0)');

  await evaluate('document.querySelector(".code-block").scrollIntoView()');
  await screenshot('theme-code');

  // Test both clipboard outcomes without depending on host clipboard permission.
  await evaluate('window.copied = null; Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { window.copied = text; } } }); document.querySelector(".copy-code").click()');
  await waitFor('document.querySelector(".copy-code").textContent === "Copied"');
  check('copy preserves exact code text', await evaluate('window.copied === document.querySelector("pre code").textContent'));
  await evaluate('Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async () => { throw new Error("denied"); } } }); document.querySelector(".copy-code").click()');
  await waitFor('document.querySelector(".copy-code").textContent === "Select to copy"');
  check('denied clipboard selects code and reports failure honestly', await evaluate('getSelection().rangeCount === 1 && getSelection().getRangeAt(0).toString() === document.querySelector("pre code").textContent'));
  await evaluate('getSelection().removeAllRanges(); window.scrollTo(0, 0)');

  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'dark' }] });
  await waitFor('document.querySelector("#theme-toggle").getAttribute("aria-label") === "Switch to light theme"');
  check('system dark theme is followed before an explicit choice', await evaluate('getComputedStyle(document.body).backgroundColor === "rgb(21, 23, 27)" && document.querySelector("#theme-toggle").textContent.trim() === "Light"'));
  await screenshot('theme-dark');
  await evaluate('document.querySelector("#theme-toggle").click()');
  await navigate('/notes/essay/');
  check('explicit light choice survives navigation and overrides system dark', await evaluate('document.documentElement.dataset.theme === "light" && getComputedStyle(document.body).backgroundColor === "rgb(255, 255, 255)"'));

  for (const width of [320, 390, 760, 768, 1024, 1120]) {
    await viewport(width, 844);
    check(`no page-wide overflow at ${width}px`, await noOverflow());
  }
  await viewport(390, 844);
  check('mobile spacing is compact while controls retain touch targets', await evaluate(`(() => {
    const prose = getComputedStyle(document.querySelector('.prose'));
    return document.querySelector('.book-header').getBoundingClientRect().height <= 60 &&
      parseFloat(prose.fontSize) === 16 && parseFloat(prose.lineHeight) >= 25 &&
      ['#nav-toggle', '#theme-toggle', '.mobile-outline summary'].every(selector =>
        document.querySelector(selector).getBoundingClientRect().height >= 44);
  })()`));
  await screenshot('theme-mobile');
  await evaluate('document.querySelector(".mobile-outline").open = true; document.querySelector(`.mobile-outline a[href="#a-small-explicit-contract"]`).click()');
  await waitFor('location.hash === "#a-small-explicit-contract"');
  check('mobile outline closes after selection and lands below the masthead', await evaluate('!document.querySelector(".mobile-outline").open && document.getElementById("a-small-explicit-contract").getBoundingClientRect().top >= document.querySelector(".book-header").getBoundingClientRect().bottom'));
  await evaluate('window.scrollTo(0, 0)');
  await evaluate('document.querySelector("#nav-toggle").focus(); document.querySelector("#nav-toggle").click()');
  check('drawer opens, locks background and focuses close control', await evaluate('document.body.classList.contains("nav-open") && document.querySelector("#page-content").inert && document.activeElement.id === "nav-close" && document.querySelector("#book-nav").getAttribute("aria-modal") === "true"'));
  await checkNavigationPageStates('mobile');
  await checkDisclosureAlignment('mobile');
  await evaluate('[...document.querySelectorAll("#book-nav summary")].at(-1).focus()');
  await pressSpace();
  check('mobile keyboard can expand a branch without closing the drawer', await evaluate('document.body.classList.contains("nav-open") && [...document.querySelectorAll("#book-nav details")].at(-1).open'));
  await pressSpace();
  await evaluate('document.querySelector("#nav-close").focus()');
  await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab', modifiers: 8 });
  await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', modifiers: 8 });
  check('drawer traps reverse tab at its first control including branch toggles', await evaluate('document.activeElement === [...document.querySelectorAll("#book-nav summary")].at(-1)'));
  await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab' });
  await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab' });
  check('drawer traps forward tab at its last link', await evaluate('document.activeElement.id === "nav-close"'));
  await screenshot('theme-mobile-contents');
  await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape' });
  check('Escape closes drawer and returns focus', await evaluate('!document.body.classList.contains("nav-open") && !document.querySelector("#page-content").inert && document.activeElement.id === "nav-toggle" && document.querySelector("#nav-backdrop").hidden'));
  await evaluate('document.querySelector("#nav-toggle").click(); document.querySelector("#nav-backdrop").click()');
  check('backdrop closes drawer', await evaluate('!document.body.classList.contains("nav-open")'));
  await evaluate('document.querySelector("#nav-toggle").click()');
  await viewport(1440);
  await waitFor('!document.body.classList.contains("nav-open")');
  check('desktop resize clears modal and scroll lock', await evaluate('!document.querySelector("#page-content").inert && !document.querySelector("#book-nav").hasAttribute("aria-modal") && getComputedStyle(document.body).overflow !== "hidden"'));
  await evaluate('for (let i = 0; i < 40; i++) { const item = document.createElement("li"); item.className = "nav-item"; const label = document.createElement("span"); label.className = "nav-group"; label.textContent = "Earlier chapter " + i; item.append(label); document.querySelector(".navigation-tree").prepend(item); }');
  await viewport(390, 844);
  await evaluate('document.querySelector("#nav-toggle").click()');
  const contentsState = await evaluate('({ scroll: document.querySelector("#book-nav").scrollTop, bottom: document.querySelector("#book-nav [aria-current=page]").getBoundingClientRect().bottom, close: document.querySelector("#nav-close").getBoundingClientRect().top, focused: document.activeElement.id, height: window.innerHeight })');
  assert.ok(contentsState.scroll > 0 && contentsState.bottom <= contentsState.height && contentsState.close >= 0 && contentsState.focused === 'nav-close', JSON.stringify(contentsState));
  check('long contents reveal the current page without hiding the focused close button', true);
  await evaluate('document.querySelector("#nav-close").click()');
  await viewport(1440);

  await navigate('/notes/native/');
  // A branch with a page keeps its label link independent of the disclosure.
  await evaluate('window.chapterBranch = document.querySelector("#book-nav details"); window.branchLink = chapterBranch.querySelector(":scope > summary a"); branchLink.href = "#page-content"; branchLink.click()');
  check('clicking a branch title link follows its target without toggling', await evaluate('chapterBranch.open && location.hash === "#page-content"'));
  check('special-character Unicode anchor has an active outline link', await evaluate('document.querySelector("#page-outline [aria-current=location]")?.hash === "#%E4%B8%AD%E6%96%87%20&%3F%23%2F"'));
  await viewport(390, 844);
  await screenshot('theme-native-mobile');
  await navigate('/plain/');
  check('headingless page omits outline rather than showing an empty rail', await evaluate('!document.querySelector(".page-outline") && !document.querySelector(".mobile-outline")'));

  // Use real, identical long navigation trees on both sides of each navigation.
  await viewport(1440, 640);
  await navigate('/');
  const beforeChapter = await evaluate(`(() => {
    const navigation = document.querySelector('#book-nav');
    [...navigation.querySelectorAll('details')].at(-1).open = true;
    const link = navigation.querySelector('a[href="_logs/more30/"]');
    navigation.scrollTop += link.getBoundingClientRect().top - navigation.getBoundingClientRect().top - navigation.clientHeight / 2;
    return {scroll: navigation.scrollTop, top: link.getBoundingClientRect().top};
  })()`);
  await evaluate('document.querySelector("#book-nav a[href=\\"_logs/more30/\\"]").click()');
  await waitFor('location.pathname === "/_logs/more30/" && document.readyState === "complete"');
  const afterChapter = await navigationPosition();
  check('clicking a scrolled chapter preserves the sidebar offset and link position', beforeChapter.scroll > 0 &&
    Math.abs(afterChapter.scroll - beforeChapter.scroll) < 1 && Math.abs(afterChapter.top - beforeChapter.top) < 1);
  const beforeNeighbor = await navigationPosition();
  await evaluate('document.querySelector("#book-nav a[href=\\"../more31/\\"]").click()');
  await waitFor('location.pathname === "/_logs/more31/" && document.readyState === "complete"');
  const afterNeighbor = await navigationPosition();
  check('consecutive chapter clicks keep the same sidebar scroll position', Math.abs(afterNeighbor.scroll - beforeNeighbor.scroll) < 1);
  await evaluate('window.beforeReload = true');
  await cdp('Page.reload');
  await waitFor('window.beforeReload === undefined && document.readyState === "complete"');
  check('reloading a chapter restores sidebar scroll', Math.abs((await navigationPosition()).scroll - afterNeighbor.scroll) < 1);
  await evaluate('history.back()');
  await waitFor('location.pathname === "/_logs/more30/" && document.readyState === "complete"');
  check('back navigation retains sidebar scroll', Math.abs((await navigationPosition()).scroll - beforeNeighbor.scroll) < 1);
  await evaluate('history.forward()');
  await waitFor('location.pathname === "/_logs/more31/" && document.readyState === "complete"');
  check('forward navigation retains sidebar scroll', Math.abs((await navigationPosition()).scroll - afterNeighbor.scroll) < 1);

  await viewport(390, 844);
  await navigate('/');
  await evaluate('document.querySelector("#nav-toggle").click()');
  const beforeMobileChapter = await evaluate(`(() => {
    const navigation = document.querySelector('#book-nav');
    [...navigation.querySelectorAll('details')].at(-1).open = true;
    const link = navigation.querySelector('a[href="_logs/more30/"]');
    navigation.scrollTop += link.getBoundingClientRect().top - navigation.clientHeight / 2;
    return {scroll: navigation.scrollTop, top: link.getBoundingClientRect().top};
  })()`);
  await evaluate('document.querySelector("#book-nav a[href=\\"_logs/more30/\\"]").click()');
  await waitFor('location.pathname === "/_logs/more30/" && document.readyState === "complete"');
  await evaluate('document.querySelector("#nav-toggle").click()');
  const afterMobileChapter = await navigationPosition();
  check('mobile chapter navigation restores the drawer position when reopened', beforeMobileChapter.scroll > 0 &&
    Math.abs(afterMobileChapter.scroll - beforeMobileChapter.scroll) < 1 && Math.abs(afterMobileChapter.top - beforeMobileChapter.top) < 1);
  await evaluate('document.querySelector("#nav-close").click()');
  await evaluate('document.querySelector("#nav-toggle").click()');
  check('closing and reopening the drawer preserves its scroll', Math.abs((await navigationPosition()).scroll - afterMobileChapter.scroll) < 1);
  await evaluate('document.querySelector("#nav-close").click()');

  await viewport(1440, 640);
  await evaluate('sessionStorage.clear()');
  await navigate('/_logs/more45/');
  check('a direct deep chapter link reveals the current chapter without saved state', await evaluate(`(() => {
    const navigation = document.querySelector('#book-nav');
    const link = navigation.querySelector('[aria-current=page]').getBoundingClientRect();
    const bounds = navigation.getBoundingClientRect();
    return navigation.scrollTop > 0 && link.top >= bounds.top && link.bottom <= bounds.bottom;
  })()`));

  await viewport(390, 844);
  await navigate('/notes/essay/', false);
  check('without JavaScript the current branch is open and archive is closed', await evaluate('document.querySelector("#book-nav [aria-current=page]").closest("details").open && ![...document.querySelectorAll("#book-nav details")].at(-1).open'));
  await evaluate('[...document.querySelectorAll("#book-nav summary")].at(-1).click()');
  check('native branch controls work without JavaScript', await evaluate('[...document.querySelectorAll("#book-nav details")].at(-1).open'));
  check('without JavaScript mobile contents stay visible and usable', await evaluate('getComputedStyle(document.querySelector("#book-nav")).display !== "none" && !document.documentElement.classList.contains("js")'));
  check('without JavaScript code remains readable', await evaluate('document.querySelector("pre code").textContent.includes("pub fn render") && !document.querySelector(".copy-code")'));
  const { identifier } = await cdp('Page.addScriptToEvaluateOnNewDocument', { source: 'for (const key of ["localStorage", "sessionStorage"]) Object.defineProperty(window, key, { get() { throw new DOMException("Storage denied"); } });' });
  await navigate('/notes/essay/');
  check('blocked storage does not prevent theme switching', await evaluate('document.querySelector("#theme-toggle").click(); document.documentElement.dataset.theme === "light" && document.documentElement.classList.contains("js")'));
  await viewport(1440, 640);
  await navigate('/_logs/more45/');
  check('blocked session storage still reveals deep current chapters', await evaluate('document.querySelector("#book-nav").scrollTop > 0 && document.querySelector("#book-nav [aria-current=page]").checkVisibility()'));
  await cdp('Page.removeScriptToEvaluateOnNewDocument', { identifier });
  await navigate('/notes/essay/');
  await evaluate('document.querySelector("#theme-toggle").click()');
  await cdp('Emulation.setEmulatedMedia', { media: 'print' });
  check('print hides chrome and wraps long code', await evaluate('getComputedStyle(document.querySelector(".book-header")).display === "none" && getComputedStyle(document.querySelector("pre code")).whiteSpace === "pre-wrap"'));
  check('printing a dark page restores readable ink colors', await evaluate('getComputedStyle(document.querySelector(".prose h1 + p")).color === "rgb(0, 0, 0)" && getComputedStyle(document.querySelector(".prose h1")).color === "rgb(0, 0, 0)"'));
  await cdp('Emulation.setEmulatedMedia', { media: '', features: [{ name: 'prefers-color-scheme', value: 'light' }] });
  await viewport(1440);
  await navigate('/raw/');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelectorAll("svg").length === 2 && document.querySelectorAll(".code-block").length === 2');
  check('grammar diagram renders native raw content beside a graph-only component', await evaluate(`(() => {
    const component = document.querySelector('[notist-function*=grammar]');
    window.grammarSourceBlock = component.previousElementSibling;
    window.grammarSourceNode = grammarSourceBlock.querySelector('pre');
    window.grammarOriginalSource = component.getAttribute('notist-source');
    return grammarSourceBlock.matches('.code-block') &&
      grammarSourceNode.querySelector('code').textContent === grammarOriginalSource &&
      grammarSourceBlock.querySelectorAll('.copy-code').length === 1 &&
      !component.querySelector('pre') && !component.shadowRoot.querySelector('pre');
  })()`));
  const matchingRawStyles = `(() => {
    const native = document.querySelector('.prose > section pre') || document.querySelector('.prose pre');
    const component = window.grammarSourceNode;
    const equal = (left, right, properties) => properties.every(name => getComputedStyle(left)[name] === getComputedStyle(right)[name]);
    return equal(native, component, ['padding', 'margin', 'whiteSpace', 'tabSize', 'fontSize', 'lineHeight']) &&
      equal(native.querySelector('code'), component.querySelector('code'), ['fontFamily', 'fontSize', 'lineHeight', 'color']) &&
      equal(native.closest('.code-block'), component.closest('.code-block'), ['backgroundColor', 'border', 'borderRadius', 'margin']) &&
      equal(native.closest('.code-block').querySelector('.code-toolbar'), component.closest('.code-block').querySelector('.code-toolbar'), ['padding', 'fontSize', 'borderBottom']);
  })()`;
  check('native raw and grammar source share the exact light-theme presentation', await evaluate(matchingRawStyles));
  await screenshot('theme-unified-raw');
  await evaluate('window.copied = null; Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { window.copied = text; } } }); grammarSourceBlock.querySelector(".copy-code").click()');
  await waitFor('grammarSourceBlock.querySelector(".copy-code").textContent === "Copied"');
  check('grammar copy uses the same toolbar and preserves its exact source', await evaluate('window.copied === document.querySelector("[notist-function*=grammar]").getAttribute("notist-source")'));
  await evaluate('document.querySelector("#theme-toggle").click()');
  check('native raw and grammar source share the exact dark-theme presentation', await evaluate(matchingRawStyles));
  await screenshot('theme-unified-raw-dark');
  await viewport(390, 844);
  check('component source remains locally scrollable on mobile', await noOverflow());
  await evaluate('document.querySelector("#theme-toggle").click()');
  await screenshot('theme-unified-raw-mobile');
  await evaluate('document.querySelector("[notist-function*=grammar]").setAttribute("notist-source", "syntax Updated -> `new`;")');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector("svg title")?.textContent.trim() === "syntax Updated"');
  check('graph parameter updates redraw without rewriting host-rendered raw content', await evaluate('document.querySelector("[notist-function*=grammar]").previousElementSibling === grammarSourceBlock && grammarSourceBlock.querySelector("pre") === grammarSourceNode && grammarSourceNode.querySelector("code").textContent === grammarOriginalSource && grammarSourceBlock.querySelectorAll(".copy-code").length === 1 && !document.querySelector("[notist-function*=grammar]").querySelector("pre")'));
  await evaluate('const component = document.querySelector("[notist-function*=grammar]"); const parent = component.parentNode; const next = component.nextSibling; component.remove(); parent.insertBefore(component, next)');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector("svg")');
  check('graph reconnection leaves the native raw block and its toolbar intact', await evaluate('document.querySelector("[notist-function*=grammar]").previousElementSibling === grammarSourceBlock && grammarSourceBlock.querySelector("pre") === grammarSourceNode && grammarSourceBlock.querySelectorAll(".copy-code").length === 1'));
  await evaluate('document.querySelector("[notist-function*=grammar]").setAttribute("notist-source", "syntax Broken -> (")');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector(".error")');
  check('graph errors leave the existing native raw content readable and copyable', await evaluate('grammarSourceNode.querySelector("code").textContent === grammarOriginalSource && grammarSourceBlock.querySelector(".copy-code").checkVisibility()'));
  await evaluate('Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async () => { throw new Error("denied"); } } }); grammarSourceBlock.querySelector(".copy-code").click()');
  await waitFor('grammarSourceBlock.querySelector(".copy-code").textContent === "Select to copy"');
  check('grammar raw clipboard failure selects the host-rendered source', await evaluate('getSelection().getRangeAt(0).toString() === grammarOriginalSource'));
  await evaluate('document.querySelector("[notist-function*=grammar]").setAttribute("notist-source", "syntax Safe -> `<script>globalThis.grammarInjected = true</script>`;")');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector("svg title")?.textContent.trim() === "syntax Safe"');
  check('graph parameter updates keep HTML-like grammar inert', await evaluate('!globalThis.grammarInjected && !document.querySelector("[notist-function*=grammar] script") && grammarSourceNode.querySelector("code").textContent === grammarOriginalSource'));
  await evaluate(`(() => {
    const make = optIn => {
      const host = document.createElement('other-code-view');
      host.setAttribute('notist-protocol', '2');
      const pre = document.createElement('pre');
      if (optIn) pre.setAttribute('data-notist-code-block', '');
      const code = document.createElement('code');
      code.textContent = 'component-owned source';
      pre.append(code); host.append(pre); document.querySelector('.prose').append(host);
      return pre;
    };
    window.privateCode = make(false);
    window.sharedCode = make(true);
  })()`);
  await waitFor('window.sharedCode.closest(".code-block")');
  check('other components can opt into the same raw presentation', await evaluate('window.sharedCode.closest(".code-block").querySelectorAll(".copy-code").length === 1'));
  check('new component-owned code UI is not hijacked without opt-in', await evaluate('!window.privateCode.closest(".code-block")'));
  await navigate('/broken/');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector(".error")');
  check('an initially invalid diagram retains its exact native raw source and copy control', await evaluate(`(() => {
    const graph = document.querySelector('[notist-function*=grammar]');
    const raw = graph.previousElementSibling;
    return raw.querySelector('pre code').textContent === ${JSON.stringify(brokenGrammar)} &&
      raw.querySelector('.copy-code').checkVisibility();
  })()`));
  await navigate('/unsafe/');
  await waitFor('document.querySelector("[notist-function*=grammar]")?.shadowRoot?.querySelector("svg")');
  check('HTML-like source stays literal in both native raw content and the graph', await evaluate(`(() => {
    const graph = document.querySelector('[notist-function*=grammar]');
    return !globalThis.grammarInjected && !document.querySelector('.prose script') &&
      !graph.shadowRoot.querySelector('script') && graph.previousElementSibling.querySelector('pre code').textContent === ${JSON.stringify(unsafeGrammar)};
  })()`));
  await navigate('/raw/', false);
  check('grammar source is readable without JavaScript or WASM initialization', await evaluate(`(() => {
    const graph = document.querySelector('[notist-function*=grammar]');
    return !graph.shadowRoot && graph.previousElementSibling.matches('pre') &&
      graph.previousElementSibling.querySelector('code').textContent === graph.getAttribute('notist-source');
  })()`));
  check('theme JavaScript raised no uncaught exceptions', exceptions.length === 0);
  console.log(`${checks.length} Chromium theme checks passed:\n${checks.map(name => `  ✓ ${name}`).join('\n')}`);
} finally {
  socket?.close();
  if (browser && browser.exitCode === null) {
    const exited = new Promise(resolve => browser.once('exit', resolve));
    browser.kill();
    await exited;
  }
  if (server) await new Promise(resolve => server.close(resolve));
  await rm(temporary, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
