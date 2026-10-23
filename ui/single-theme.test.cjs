const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const core = require('./core.js');
const gate = fs.readFileSync(path.join(__dirname, 'style-gate.js'), 'utf8');

function boot(saved, pathname = '/index.html', blocked = false) {
  const values = new Map([['gamestrike-style', saved], ['gamestrike-sounds', 'off'], ['gamestrike-sort', 'title-asc']]);
  const navigations = [];
  const attributes = new Map();
  vm.runInNewContext(gate, {
    window: {
      localStorage: {
        getItem(key) { if (blocked) throw Error('unavailable'); return values.get(key) ?? null; },
        setItem(key, value) { if (blocked) throw Error('unavailable'); values.set(key, value); },
      },
      location: { pathname, replace(url) { navigations.push(url); } },
    },
    document: { documentElement: { setAttribute(key, value) { attributes.set(key, value); } } },
  });
  return { values, navigations, attributes };
}

test('PWDS Collection is the only selectable dashboard; all former choices resolve to it', () => {
  assert.deepEqual(core.STYLES.map(style => style.key), ['collection-orange']);
  for (const old of [null, '', 'collection-green', 'blades', 'original', 'pwds', 'unknown']) {
    assert.equal(core.styleFor(old).key, 'collection-orange');
    assert.equal(core.nextStyle(old).key, 'collection-orange');
  }
});

test('stale theme preferences migrate without changing sound or library preferences', () => {
  for (const old of ['collection-green', 'blades', 'original', 'pwds', 'unknown']) {
    const result = boot(old);
    assert.equal(result.values.get('gamestrike-style'), 'collection-orange');
    assert.equal(result.values.get('gamestrike-sounds'), 'off');
    assert.equal(result.values.get('gamestrike-sort'), 'title-asc');
    assert.deepEqual(result.navigations, []);
  }
});

test('direct legacy page returns to Collection even when optional storage is unavailable', () => {
  assert.deepEqual(boot('blades', '/classic.html').navigations, ['/index.html']);
  assert.deepEqual(boot('original', '/classic.html', true).navigations, ['/index.html']);
  assert.deepEqual(boot('pwds', '/index.html', true).navigations, []);
});

test('settings and legacy entry cannot expose the removed dashboards', () => {
  const html = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');
  const app = fs.readFileSync(path.join(__dirname, 'app.js'), 'utf8');
  const classic = fs.readFileSync(path.join(__dirname, 'classic.html'), 'utf8');
  assert.doesNotMatch(html, /id="settings-style"/);
  assert.doesNotMatch(app, /\$\("settings-style"\)|location\.replace\("\/classic\.html"\)/);
  assert.match(classic, /src="\/style-gate\.js"/);
  assert.doesNotMatch(classic, /src="\/classic\.js"/);
});
