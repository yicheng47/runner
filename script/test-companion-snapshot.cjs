const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Terminal } = require(process.argv[2] || '@xterm/headless');

const fixtures = path.join(__dirname, '../crates/runner-core/src/companion/fixtures');
const load = name => JSON.parse(fs.readFileSync(path.join(fixtures, `${name}.json`)));
const write = (term, bytes) => new Promise(resolve => term.write(bytes, resolve));

// xterm exposes CSI hooks, but printing inside a hook currently needs its internal InputHandler.
function hostRep(term) {
  const input = term._core._inputHandler;
  const print = input.print.bind(input);
  let preceding = null;
  input.print = (data, start, end) => {
    if (end > start) preceding = data[end - 1];
    print(data, start, end);
  };
  term.parser.registerCsiHandler({ final: 'b' }, params => {
    if (preceding !== null) {
      const data = new Uint32Array(params[0] || 1).fill(preceding);
      input.print(data, 0, data.length);
    }
    return true;
  });
  return char => { preceding = char === null ? null : char.codePointAt(0); };
}

async function replay(snapshot, output, continuation = true) {
  const term = new Terminal({ cols: snapshot.cols, rows: snapshot.rows, allowProposedApi: true });
  try {
    const restore = continuation ? hostRep(term) : () => {};
    const bytes = Buffer.from(snapshot.data, 'base64');
    const split = bytes.length - (snapshot.unfinishedLen || 0);
    assert(split >= 0, 'unfinishedLen exceeds the assembled snapshot');
    await write(term, bytes.subarray(0, split));
    restore(snapshot.precedingChar ?? null);
    await write(term, bytes.subarray(split));
    await write(term, output);
    return term.buffer.active.getLine(0).translateToString(true).trimEnd();
  } finally {
    term.dispose();
  }
}

async function main() {
  const snapshot = load('snapshot_rep');
  const unfinished = load('snapshot_rep_unfinished');
  assert.equal(await replay(snapshot, '\x1b[3b', false), 'abc', 'byte-only reproduction');
  for (let resync = 0; resync < 2; resync++) {
    assert.equal(await replay(snapshot, '\x1b[3b'), 'abcccc');
    assert.equal(await replay(unfinished, 'b'), 'abcccc');
  }
  assert.equal(await replay(snapshot, '\x1b[1G\x1b[3b'), 'ccc');
  assert.equal(await replay({ ...snapshot, precedingChar: null }, '\x1b[3b'), 'abc');
  const bytes = Buffer.from(snapshot.data, 'base64');
  const utf8 = Buffer.from('界');
  const partial = { ...snapshot, data: Buffer.concat([bytes, utf8.subarray(0, 2)]).toString('base64'), unfinishedLen: 2 };
  assert.equal(await replay(partial, Buffer.concat([utf8.subarray(2), Buffer.from('\x1b[2b')])), 'abc界界界');
  const legacy = { ...snapshot };
  delete legacy.precedingChar;
  delete legacy.unfinishedLen;
  assert.equal(await replay(legacy, 'd'), 'abcd');
  console.log('PASS: xterm companion attach/resync REP, unfinished CSI/UTF-8, cursor movement, null and legacy continuation');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
