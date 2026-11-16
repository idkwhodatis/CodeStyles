import fs from 'node:fs';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Run the bounded suite in a disposable process with a hard wall-clock timeout.
const args = process.argv.slice(2);
const options = {};
for (let i = 0; i < args.length; i += 2) {
  assert.ok(['--wasm', '--sha256', '--oracle', '--report', '--timeout', '--child'].includes(args[i]), 'Unknown option');
  assert.ok(args[i + 1], 'Each option requires a value');
  options[args[i].slice(2)] = args[i + 1];
}
assert.ok(options.wasm && options.sha256 && options.oracle, 'Required: --wasm PATH --sha256 HEX --oracle PATH [--report PATH] [--timeout SECONDS]');
const seconds = Number(options.timeout ?? 120);
assert.ok(Number.isFinite(seconds) && seconds > 0 && seconds <= 300, 'Timeout must be 0 < seconds <= 300');
if (!options.child) {
  const result = spawnSync(process.execPath, [fileURLToPath(import.meta.url), ...args, '--child', 'true'],
    { encoding: 'utf8', timeout: seconds * 1000, maxBuffer: 1024 * 1024 });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, result.stderr || `Child failed: ${result.signal}`);
  const report = JSON.parse(result.stdout);
  if (options.report) fs.writeFileSync(options.report, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
  process.exit(0);
}
const wasm = fs.readFileSync(options.wasm);
assert.equal(crypto.createHash('sha256').update(wasm).digest('hex'), options.sha256, 'WASM checksum mismatch');
const module = new WebAssembly.Module(wasm);
const imports = WebAssembly.Module.imports(module);
assert.deepEqual(imports,[], 'Must not expose files, network, env, WASI or host callbacks');
const {exports:e}=new WebAssembly.Instance(module,{});
const encode = new TextEncoder(); const decode = new TextDecoder('utf-8',{fatal:true,ignoreBOM:true});
function send(data) { if(typeof data==='string')data=encode.encode(data); const p=e.clear_shared_bytes(data.length); new Uint8Array(e.memory.buffer,p,data.length).set(data); }
function read(size) { return decode.decode(new Uint8Array(e.memory.buffer,e.get_shared_bytes_ptr(),size)); }
function format(input,id=1){send(input);const status=e.format(id);return {status,text:status===0?input:read(status===1?e.get_formatted_text():e.get_error_text())};}
send('{"plugin":{},"global":{"useTabs":false,"indentWidth":8}}');e.register_config(1);
assert.equal(read(e.get_config_diagnostics(1)),'[]');
assert.equal(read(e.get_resolved_config(1)),'{}');
const cases=JSON.parse(fs.readFileSync(options.oracle));
assert.ok(cases.length > 0 && cases.length <= 1000);
for(const row of cases){assert.ok(encode.encode(row.input).length <= 65536 && encode.encode(row.expect).length <= 8 * 1024 * 1024, 'Fixture exceeds bounded suite size');const r=format(row.input);assert.notEqual(r.status,2,row.description);assert.equal(r.text,row.expect,row.description);assert.equal(r.status,row.input===row.expect?0:1,row.description);}
const growth=[];const sample='localhost {\n'+Array(100).fill('respond "hello {😀} \\"quoted\\""\n').join('')+'}\n';
const expected=format(sample).text;
for(let i=0;i<10000;i++){
 assert.equal(format(sample).text,expected);
 if(i%500===0)growth.push({iteration:i,bytes:e.memory.buffer.byteLength});
}
const steady=growth.slice(-10).map(x=>x.bytes);
assert.ok(Math.max(...steady)-Math.min(...steady)<8*1024*1024,'Repeated use should plateau within an 8 MiB margin');
send('{"plugin":{"indentWidth":8},"global":{}}');e.register_config(2);
assert.notEqual(read(e.get_config_diagnostics(2)),'[]');assert.equal(format('localhost {}',2).status,2);e.release_config(2);
send('{"unexpected":true}');e.set_override_config();assert.equal(format('localhost {}').status,2);assert.notEqual(format('localhost {}').status,2);
assert.equal(format(Uint8Array.of(255)).status,2);assert.notEqual(format('localhost {}').status,2);
send('localhost {}');assert.equal(e.format_range(1,0,5),2);assert.match(read(e.get_error_text()),/range/);assert.notEqual(format('localhost {}').status,2);
for(let i=10;i<1010;i++){send('{"plugin":{},"global":{}}');e.register_config(i);e.release_config(i);}
// Bounded nested-brace probes exercise upstream indentation saturation and recovery.
const nested = [];
for (const depth of [64, 256, 1000, 2000]) {
  const input = 'a {\n'.repeat(depth) + 'x\n' + '}\n'.repeat(depth);
  const before = e.memory.buffer.byteLength;
  const result = format(input);
  assert.notEqual(result.status, 2);
  assert.ok(encode.encode(result.text).length <= 8 * 1024 * 1024);
  assert.equal(format(result.text).text, result.text);
  nested.push({depth, input_bytes: encode.encode(input).length,
    output_bytes: encode.encode(result.text).length, memory_before: before,
    memory_after: e.memory.buffer.byteLength});
}
const limit=32*1024*1024;
assert.equal(format(' '.repeat(limit)).text,'\n');
assert.equal(format(' '.repeat(limit+1)).status,2);
assert.equal(format('x'.repeat(limit)).status,2);
assert.notEqual(format('localhost {}').status,2);
assert.throws(()=>e.clear_shared_bytes(limit+2),WebAssembly.RuntimeError);
const report={imports,exports:WebAssembly.Module.exports(module).map(x=>x.name),upstream_cases:cases.filter(x=>x.origin==='upstream').length,differential_cases:cases.length,repeated_calls:10000,memory_growth:growth,boundaries:['32 MiB accepted','32 MiB + 1 rejected','output over 32 MiB rejected','oversized host transfer traps'],error_recovery:['unknown config','bad override','invalid UTF-8','unsupported range'],nested_braces:nested, resource_limit_note:'Whole suite has a subprocess timeout. Inputs/depth are bounded; observed linear memory is not a worst-case bound or a host-enforced memory ceiling.', final_memory_bytes:e.memory.buffer.byteLength,status:'passed'};
console.log(JSON.stringify(report,null,2));
