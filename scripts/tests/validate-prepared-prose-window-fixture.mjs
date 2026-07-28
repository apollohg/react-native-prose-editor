import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const corpus = JSON.parse(readFileSync(new URL('./viewer-performance-corpus.json', import.meta.url)));
const counts = {short: 900, medium: 80, image: 15, 'very-long': 5};
const category = (id) => id.startsWith('short-') ? 'short' : id.startsWith('medium-') ? 'medium' : id.startsWith('image-') ? 'image' : id.startsWith('very-long-') ? 'very-long' : 'unknown';

export function validateWindowContract(value) {
  const ids = value.documents.map(({id}) => id);
  assert.equal(ids.length, 1000); assert.equal(new Set(ids).size, 1000);
  assert.deepEqual(Object.fromEntries(Object.keys(counts).map((key) => [key, ids.filter((id) => category(id) === key).length])), counts);
  assert.equal(value.warmWindows.length, 27);
  const prime = [];
  for (const window of value.warmWindows) {
    const kind = category(window.primeIds[0]);
    const maximum = {short: 64, medium: 16, image: 8, 'very-long': 1}[kind];
    assert.ok(maximum != null && window.primeIds.length >= 1 && window.primeIds.length <= maximum, window.id);
    assert.ok(window.primeIds.every((id) => category(id) === kind), window.id);
    assert.deepEqual(window.warmIds, window.primeIds, window.id);
    prime.push(...window.primeIds);
  }
  assert.equal(new Set(prime).size, 1000);
  assert.deepEqual([...prime].sort(), [...ids].sort());
  assert.ok(value.warmWindows.filter((w) => category(w.primeIds[0]) === 'very-long').every((w) => w.primeIds.length === 1));
}

validateWindowContract(corpus);
for (const mutate of [
  (v) => v.warmWindows[0].primeIds.pop(),
  (v) => { v.warmWindows[1].primeIds[0] = v.warmWindows[0].primeIds[0]; v.warmWindows[1].warmIds[0] = v.warmWindows[0].primeIds[0]; },
  (v) => v.warmWindows[0].warmIds.reverse(),
  (v) => { const overflow = v.warmWindows[1].primeIds.slice(0, 5); v.warmWindows[0].primeIds.push(...overflow); v.warmWindows[0].warmIds.push(...overflow); },
  (v) => { v.warmWindows[22].primeIds = ['very-long-0001', 'very-long-0002']; v.warmWindows[22].warmIds = ['very-long-0001', 'very-long-0002']; },
]) { const candidate = structuredClone(corpus); mutate(candidate); assert.throws(() => validateWindowContract(candidate)); }
console.log('prepared-prose window fixture: PASS');
