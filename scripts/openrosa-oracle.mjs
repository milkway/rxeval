// Ask the reference implementation what each expression means, and write
// the answers down.
//
//   node scripts/openrosa-oracle.mjs [oracle-dir] > .../expected.json
//
// The package has to be installed somewhere node can find from this file,
// which is why the directory is an argument: run the copy that sits beside
// node_modules and point it at the repo.
//
// The reference is Enketo's openrosa-xpath-evaluator — the engine that runs
// in every Enketo web form. It wraps the browser's own XPath, so this needs
// a DOM; jsdom supplies one, and the package needs a handful of globals it
// expects to find on `window`.
//
// The output is committed, so the Rust tests compare against it without
// needing Node. Regenerate it when the corpus changes, and read the diff:
// a line that moves is either a bug fixed upstream or a case where the two
// implementations were never going to agree.
//
//   npm install openrosa-xpath-evaluator jsdom

import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html>');
for (const name of [
  'window',
  'document',
  'Node',
  'XPathResult',
  'XPathEvaluator',
  'DOMParser',
  'XMLSerializer',
]) {
  globalThis[name] = dom.window[name];
}

const { default: makeEvaluator } = await import('openrosa-xpath-evaluator');
const evaluator = makeEvaluator();
const R = dom.window.XPathResult;

const here = process.argv[2]
  ? new URL(`file://${process.argv[2].replace(/\/?$/, '/')}`)
  : new URL('../rxeval/tests/oracle/', import.meta.url);
const instanceXml = readFileSync(new URL('instance.xml', here), 'utf8');
const corpus = readFileSync(new URL('corpus.txt', here), 'utf8');

const instance = new dom.window.DOMParser().parseFromString(instanceXml, 'text/xml');
const context = instance.documentElement;

/** Read one XPathResult into a comparable shape. */
function describe(result) {
  switch (result.resultType) {
    case R.STRING_TYPE:
      return { type: 'string', value: result.stringValue };
    case R.NUMBER_TYPE:
      return { type: 'number', value: describeNumber(result.numberValue) };
    case R.BOOLEAN_TYPE:
      return { type: 'boolean', value: result.booleanValue };
    default: {
      // A node-set is compared by the string values it holds, in document
      // order: node identity means nothing across two implementations, and
      // the values are what a form goes on to use.
      const values = [];
      let node;
      while ((node = result.iterateNext())) values.push(node.textContent);
      return { type: 'nodeset', value: values };
    }
  }
}

// Which build answered, so a fixture that moves can be explained.
const pkg = JSON.parse(
  readFileSync(
    new URL('./node_modules/openrosa-xpath-evaluator/package.json', import.meta.url),
    'utf8'
  )
);
const results = {
  $meta: { engine: 'openrosa-xpath-evaluator', version: pkg.version },
};
for (const line of corpus.split('\n')) {
  const expression = line.trim();
  if (!expression || expression.startsWith('#')) continue;
  try {
    const result = evaluator.evaluate(expression, context, null, R.ANY_TYPE, null);
    results[expression] = { ...describe(result), via: 'openrosa' };
  } catch (error) {
    // The OpenRosa layer adds functions on top of the browser's XPath and
    // delegates the rest. Outside a browser it refuses some plain
    // expressions that the underlying engine handles perfectly well — so
    // fall back to it, and record which one answered. Provenance stays in
    // the file: a fixture that hides where a value came from is not
    // evidence of anything.
    try {
      const native = instance.evaluate(expression, context, null, R.ANY_TYPE, null);
      results[expression] = { ...describe(native), via: 'native-xpath' };
    } catch (nativeError) {
      // Both refused. That is an answer too, and the interesting kind: if
      // one engine refuses where the other invents a value, that is the
      // divergence most worth knowing about.
      results[expression] = {
        type: 'error',
        value: String(error.message || error),
        native: String(nativeError.message || nativeError),
        via: 'both-refused',
      };
    }
  }
}

/** JSON has one number type; NaN and the infinities need names. */
function describeNumber(n) {
  if (Number.isNaN(n)) return 'NaN';
  if (n === Infinity) return 'Infinity';
  if (n === -Infinity) return '-Infinity';
  return n;
}

process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
