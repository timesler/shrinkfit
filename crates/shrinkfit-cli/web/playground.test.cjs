// Dependency-free UI behavior tests. No browser, API key, or network required.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');
const html = readFileSync(join(__dirname, 'index.html'), 'utf8');
const code = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const manifest = JSON.parse(readFileSync(join(__dirname, 'twg/manifest.json'), 'utf8'));
const flush = () => new Promise(resolve => setImmediate(resolve));

async function setup(fetchOverride) {
  const nodes = new Map();
  const node = () => ({ value: '', checked: true, textContent: '', children: [], listeners: {},
    append(child) { this.children.push(child); },
    addEventListener(name, cb) { (this.listeners[name] ||= []).push(cb); },
  });
  const get = id => { if (!nodes.has(id)) nodes.set(id, node()); return nodes.get(id); };
  const requests = [];
  const context = vm.createContext({
    document: { getElementById: get, querySelectorAll: () => [], createElement: node },
    localStorage: { getItem: () => null, setItem() {} }, navigator: {},
    setTimeout: () => 1, clearTimeout() {},
    EXAMPLES: [() => ({ name: 'Generated', desc: 'Generated example', text: '{}', max_chars: 100 })],
    fetch: async url => {
      requests.push(url);
      const overridden = fetchOverride?.(url);
      if (overridden) return overridden;
      if (url === '/twg/manifest.json') return { ok: true, json: async () => manifest };
      return { ok: true, text: async () => readFileSync(join(__dirname, url.slice(1)), 'utf8') };
    },
  });
  vm.runInContext(code, context);
  await flush();
  return { get, requests };
}

test('catalog loads lazily; all examples set input, budget, and provenance', async () => {
  const { get, requests } = await setup();
  assert.deepEqual(requests, ['/twg/manifest.json']);
  const buttons = get('twg-examples').children;
  assert.equal(buttons.length, manifest.length);
  for (let i = 0; i < buttons.length; i++) {
    get('jev').checked = false;
    get('arguments').value = 'stale arguments';
    await buttons[i].onclick();
    assert.equal(get('input').value.length, manifest[i].chars);
    assert.equal(get('max_chars').value, manifest[i].max_chars);
    assert.equal(get('array_strategy').value, 'condense');
    assert.equal(get('input_format').value, 'json');
    assert.equal(get('format').value, 'json');
    assert.equal(get('arguments').value, '');
    assert.equal(get('jev').checked, false);
    assert.ok(get('example-info').textContent.includes(manifest[i].command));
    assert.ok(get('example-info').textContent.includes('synthetic'));
    assert.equal(JSON.parse(get('input').value).apiVersion, 'v2');
  }
});

test('failed fixture request preserves current input and displays an error', async () => {
  const { get } = await setup(url => url.endsWith('boards.json') && { ok: false, status: 503 });
  get('input').value = 'keep me';
  await get('twg-examples').children.at(-1).onclick();
  assert.equal(get('input').value, 'keep me');
  assert.match(get('example-info').textContent, /HTTP 503/);
});

test('failed catalog displays an error without breaking generated examples', async () => {
  const { get } = await setup(() => ({ ok: false, status: 500 }));
  assert.match(get('twg-examples').textContent, /HTTP 500/);
  get('examples').children[0].onclick();
  assert.equal(get('input').value, '{}');
});

test('late fixture responses cannot overwrite a newer selection or manual edit', async () => {
  let finish;
  const { get } = await setup(url => url.endsWith('jira-search.json') && new Promise(resolve => { finish = resolve; }));
  const old = get('twg-examples').children[0].onclick();
  await get('twg-examples').children[1].onclick();
  const selected = get('input').value;
  finish({ ok: true, text: async () => 'stale' });
  await old;
  assert.equal(get('input').value, selected);

  const edited = get('twg-examples').children[0].onclick();
  get('input').value = 'manual edit';
  for (const cb of get('input').listeners.input) cb();
  finish({ ok: true, text: async () => 'stale' });
  await edited;
  assert.equal(get('input').value, 'manual edit');

  const generated = get('twg-examples').children[0].onclick();
  get('examples').children[0].onclick();
  finish({ ok: true, text: async () => 'stale' });
  await generated;
  assert.equal(get('input').value, '{}');
});
