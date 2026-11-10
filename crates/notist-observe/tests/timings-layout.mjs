// Standalone browser regression check; requires Chromium, no npm dependencies.
// Run: node crates/notist-observe/tests/timings-layout.mjs
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const directory = mkdtempSync('/tmp/opencode/notist-timings-layout-');
const elapsed = 2_906_828_000;
const report = {
    elapsed_ns: elapsed,
    attributes: { trigger: 'layout regression fixture' },
    spans: Array.from({ length: 120 }, (_, id) => ({
        id, parent: id ? id - 1 : null, lane: 0,
        stage: id ? 'site.extensions.before_template.with.a.long.stage.name' : 'host.rebuild',
        label: id ? 'Analyze /a/very/long/source/path/'.repeat(8) + 'document.not' : 'Rebuild notist',
        start_ns: id * 1_000, end_ns: elapsed - id * 1_000,
        status: 'finished', attributes: { source: '/content/document.not' },
    })),
};
const template = readFileSync(new URL('../src/timings.html', import.meta.url), 'utf8');

for (const width of [1440, 1024, 800, 390]) {
    const check = `<script>
    document.querySelector('#expand').click();
    setTimeout(() => {
        const boxes = [...document.querySelectorAll('.scroll')];
        const axis = document.querySelector('.axis').getBoundingClientRect();
        const ticks = [...document.querySelectorAll('.axis span')].map(t => t.getBoundingClientRect());
        const name = [...document.querySelectorAll('.operation-name')].at(-1);
        const result = {
            viewport: innerWidth,
            pageWidth: document.documentElement.scrollWidth,
            overflow: boxes.map(box => box.scrollWidth > box.clientWidth + 1),
            ticksFit: ticks.every(t => t.left >= axis.left - 1 && t.right <= axis.right + 1),
            ticksSeparated: ticks.every((t, i) => !i || t.left >= ticks[i - 1].right + 4),
            tickCount: ticks.length,
            headersFit: [...document.querySelectorAll('.timeline-table th:not(#axis)')].every(t => t.scrollWidth <= t.clientWidth),
            truncated: name.scrollWidth > name.clientWidth,
            toggleVisible: name.previousElementSibling.getBoundingClientRect().width > 0,
            fullName: name.parentElement.parentElement.title === name.textContent,
        };
        const output = document.createElement('pre');
        output.id = 'layout-result';
        output.textContent = JSON.stringify(result);
        document.body.append(output);
    }, 100);
    </script>`;
    const file = join(directory, `report-${width}.html`);
    writeFileSync(file, template.replace('/*REPORT*/null', JSON.stringify(report)).replace('</html>', check + '</html>'));
    const result = spawnSync(process.env.CHROMIUM ?? 'chromium', [
        '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
        '--no-first-run', '--hide-scrollbars', '--allow-file-access-from-files',
        `--user-data-dir=${join(directory, `profile-${width}`)}`,
        `--window-size=${width},1000`, '--virtual-time-budget=1000', '--dump-dom',
        `--screenshot=${join(directory, `report-${width}.png`)}`, pathToFileURL(file).href,
    ], { encoding: 'utf8', timeout: 30_000, maxBuffer: 8 * 1024 * 1024 });
    assert.equal(result.status, 0, result.stderr);
    const match = result.stdout.match(/<pre id="layout-result">([^<]+)<\/pre>/);
    assert.ok(match, 'browser did not execute the layout check');
    const layout = JSON.parse(match[1]);
    assert.ok(layout.pageWidth <= layout.viewport, JSON.stringify(layout));
    assert.ok(layout.ticksFit && layout.ticksSeparated && layout.headersFit, JSON.stringify(layout));
    assert.ok(layout.truncated && layout.toggleVisible && layout.fullName, JSON.stringify(layout));
    if (width >= 800) assert.ok(layout.overflow.every(value => !value), JSON.stringify(layout));
    else assert.equal(layout.overflow[0], true, JSON.stringify(layout));
    console.log(`${width}px: no page overflow; ${layout.tickCount} separated ticks; table scroll ${layout.overflow[0]}`);
}
console.log(`Screenshots: ${directory}`);
