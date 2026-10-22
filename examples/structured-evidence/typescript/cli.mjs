/** Ordinary command adapter; the host binds every file path and invocation ID. */
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

function parse(raw) {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(raw);
  const frames = [];
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (char === '"') {
      const start = i; let escaped = false;
      for (i++; i < text.length; i++) {
        if (!escaped && text[i] === '"') break;
        if (!escaped && text[i] === '\\') escaped = true;
        else escaped = false;
      }
      const value = JSON.parse(text.slice(start, i + 1));
      const frame = frames.at(-1);
      if (frame?.kind === 'object' && frame.key) {
        if (frame.names.has(value)) throw new Error();
        frame.names.add(value); frame.key = false;
      }
    } else if (char === '{') frames.push({ kind: 'object', names: new Set(), key: true });
    else if (char === '[') frames.push({ kind: 'array' });
    else if (char === '}' || char === ']') frames.pop();
    else if (char === ',' && frames.at(-1)?.kind === 'object') frames.at(-1).key = true;
    else if (char === '-' || /[0-9]/.test(char)) {
      const start = i;
      while (i + 1 < text.length && /[0-9eE+.\-]/.test(text[i + 1])) i++;
      const token = text.slice(start, i + 1);
      if (!/^-?(0|[1-9][0-9]*)$/.test(token) || !Number.isSafeInteger(Number(token))) throw new Error();
    }
  }
  return JSON.parse(text);
}

try {
  const args = process.argv.slice(2);
  if (args.length !== 6 || args[0] !== '--module' || args[2] !== '--fixture' || args[4] !== '--invocation') throw new Error();
  const chunks = []; let total = 0;
  for await (const chunk of process.stdin) {
    total += chunk.length;
    if (total > 8192) throw new Error();
    chunks.push(chunk);
  }
  const fixture = await readFile(args[3]);
  if (fixture.length > 8192) throw new Error();
  const implementation = await import(pathToFileURL(args[1]).href);
  const result = implementation.collect(parse(Buffer.concat(chunks)), parse(fixture), args[5]);
  const encoded = JSON.stringify(result);
  if (Buffer.byteLength(encoded) > 8192) throw new Error();
  process.stdout.write(encoded + '\n');
} catch {
  process.stderr.write('structured-evidence-rejected\n');
  process.exitCode = 2;
}
