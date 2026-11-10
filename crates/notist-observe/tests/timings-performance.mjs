// Browser regression for large, interleaved timing trees. No npm dependencies.
// Run: node crates/notist-observe/tests/timings-performance.mjs
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const directory = mkdtempSync('/tmp/opencode/notist-timings-performance-');
const template = readFileSync(new URL('../src/timings.html', import.meta.url), 'utf8');
const elapsed = 1_000_000_000;
const spans = [];
function span(parent, label, lane = 0, attributes = {}) {
    const id = spans.length;
    spans.push({ id, parent, label, lane, stage: 'site.build', start_ns: id,
        end_ns: elapsed - id, status: 'finished', attributes });
    return id;
}
const root = span(null, 'Build');
const workers = [span(root, 'Worker A'), span(root, 'Worker B')];
const documents = [];
for (let i = 0; i < 6000; i++) documents.push(span(workers[i % 2], `Document ${i}`, i % 2));
// Descendants arrive after other workers' siblings, not in depth-first order.
for (const id of documents) {
    span(id, `Analyze ${id}`, id % 2, { source: `document-${id}.not` });
    span(id, `Render ${id}`, id % 2);
}
const report = { elapsed_ns: elapsed, attributes: {}, spans };
const check = `<script>
const checks=[];
const verify=(condition,message)=>{if(!condition)throw Error(message);checks.push(message)};
const rows=()=>[...document.querySelectorAll('#timeline tr:not(.spacer)')];
const names=()=>rows().map(r=>r.querySelector('.operation-name').textContent);
const pause=()=>new Promise(resolve=>setTimeout(resolve,80));
(async()=>{
    const timings={initial:performance.now()-started};
    verify(rows().length<60,'initial DOM is bounded');
    verify(visible.length===6003,'default collapse hides document children');
    verify(rows().every(r=>r.getBoundingClientRect().height===38),'virtual row heights match');
    verify(names().slice(0,4).join('|')==='Build|Worker A|Document 0|Document 2','workers form contiguous subtrees');
    let start=performance.now();
    document.querySelector('#expand').click();
    timings.expand=performance.now()-start;
    verify(visible.length===report.spans.length&&rows().length<60,'expand all keeps DOM bounded');
    viewport.scrollTop=viewport.scrollHeight;
    await pause();
    drawTimeline();
    verify(names().at(-1)==='Render 6002','scroll reaches final span');
    verify(Math.abs(rows().at(-1).getBoundingClientRect().bottom-viewport.getBoundingClientRect().bottom)<3,'last row aligns with viewport');
    // Collapse a branch at the bottom, where the old scroll offset exceeds the new height.
    document.querySelector('#timeline .toggle:not(:disabled)')?.click();
    document.querySelector('#collapse').click();
    verify(visible.length===1&&names()[0]==='Build','collapse all resets scrolled viewport');
    start=performance.now();
    document.querySelector('#timeline .toggle').click();
    timings.toggle=performance.now()-start;
    verify(visible.length===3&&rows().length===3,'toggle exposes only immediate children');
    document.querySelector('#filter').value='document-6002.not';
    document.querySelector('#filter').dispatchEvent(new Event('input'));
    await new Promise(resolve=>setTimeout(resolve,180));
    verify(visible.length===1&&names()[0]==='Analyze 6002','filter searches attributes inside collapsed branches');
    document.querySelector('#filter').value='missing-result';
    document.querySelector('#filter').dispatchEvent(new Event('input'));
    await new Promise(resolve=>setTimeout(resolve,180));
    verify(visible.length===0&&rows().length===0,'empty search has no stale rows');
    document.querySelector('#filter').value='';
    document.querySelector('#filter').dispatchEvent(new Event('input'));
    await new Promise(resolve=>setTimeout(resolve,180));
    verify(visible.length===3,'clearing filter preserves collapsed state');
    return {checks,timings,spans:report.spans.length};
})().then(result=>{
    const output=document.createElement('pre');output.id='performance-result';
    output.textContent=JSON.stringify(result);document.body.append(output);
}).catch(error=>{
    const output=document.createElement('pre');output.id='performance-result';
    output.textContent=JSON.stringify({error:error.message,checks});document.body.append(output);
});
</script>`;
const file = join(directory, 'report.html');
writeFileSync(file, template.replace("'use strict';", "'use strict';const started=performance.now();")
    .replace('/*REPORT*/null', JSON.stringify(report)).replace('</html>', check + '</html>'));
const result = spawnSync(process.env.CHROMIUM ?? 'chromium', [
    '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
    '--no-first-run', `--user-data-dir=${join(directory, 'profile')}`,
    '--window-size=1440,1000', '--virtual-time-budget=3000', '--dump-dom', pathToFileURL(file).href,
], { encoding: 'utf8', timeout: 60_000, maxBuffer: 20 * 1024 * 1024 });
assert.equal(result.status, 0, result.stderr);
const match = result.stdout.match(/<pre id="performance-result">([^<]+)<\/pre>/);
assert.ok(match, 'browser did not complete performance checks');
const outcome = JSON.parse(match[1]);
assert.equal(outcome.error, undefined, JSON.stringify(outcome));
console.log(JSON.stringify(outcome, null, 2));
console.log(`Fixture: ${file}`);
