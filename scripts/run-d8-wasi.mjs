/*
 * d8 / V8 loader for the criterion benchmark compiled to wasm32-wasip1.
 *
 * run:
 *   v8 --experimental-wasm-wide-arithmetic --module scripts/run-d8-wasi.mjs \
 *      -- <wasm> --bench <criterion-args>
 */

import {
  WASI,
  File,
  OpenFile,
  ConsoleStdout,
  PreopenDirectory,
} from "../node_modules/@bjorn3/browser_wasi_shim/dist/index.js";

// The shim reaches for the global `self`; d8 only defines `globalThis`.
globalThis.self = globalThis;

// d8 ships no TextEncoder / TextDecoder; the shim needs UTF-8 both ways
// (arg encoding, and decoding criterion's output, which contains "µs").
if (typeof globalThis.TextEncoder === "undefined") {
  globalThis.TextEncoder = class {
    encode(str) {
      const out = [];
      for (let i = 0; i < str.length; i++) {
        let cp = str.codePointAt(i);
        if (cp > 0xffff) i++;
        if (cp < 0x80) {
          out.push(cp);
        } else if (cp < 0x800) {
          out.push(
            0xc0 | (cp >>> 6),
            0x80 | (cp & 0x3f)
          );
        } else if (cp < 0x10000) {
          out.push(
            0xe0 | (cp >>> 12),
            0x80 | ((cp >>> 6) & 0x3f),
            0x80 | (cp & 0x3f)
          );
        } else {
          out.push(
            0xf0 | (cp >>> 18),
            0x80 | ((cp >>> 12) & 0x3f),
            0x80 | ((cp >>> 6) & 0x3f),
            0x80 | (cp & 0x3f),
          );
        }
      }
      return new Uint8Array(out);
    }
  };
}
if (typeof globalThis.TextDecoder === "undefined") {
  globalThis.TextDecoder = class {
    constructor() {
      this.pending = [];
    }
    decode(input, options) {
      const stream = options && options.stream;
      const bytes = this.pending.concat(Array.from(input || []));

      this.pending = [];
      let out = "";
      let i = 0;

      while (i < bytes.length) {
        const b = bytes[i];
        let cp, len;
        if (b < 0x80) {
          cp = b;
          len = 1;
        }
        else if ((b & 0xe0) === 0xc0) { cp = b & 0x1f; len = 2; }
        else if ((b & 0xf0) === 0xe0) { cp = b & 0x0f; len = 3; }
        else if ((b & 0xf8) === 0xf0) { cp = b & 0x07; len = 4; }
        else {
          i++;
          out += "�";
          continue;
        }
        if (i + len > bytes.length) {
          if (stream) {
            this.pending = bytes.slice(i);
          }
          else out += "�";
          break;
        }
        for (let k = 1; k < len; k++) {
          cp = (cp << 6) | (bytes[i + k] & 0x3f);
        }
        out += String.fromCodePoint(cp);
        i += len;
      }
      return out;
    }
  };
}

const wasmPath = arguments[0];
const benchArgs = arguments.slice(1);

const fds = [
  new OpenFile(new File([])),                  // fd 0: stdin
  ConsoleStdout.lineBuffered((s) => print(s)), // fd 1: stdout
  ConsoleStdout.lineBuffered((s) => print(s)), // fd 2: stderr
  new PreopenDirectory(".", new Map()),        // fd 3: writable cwd for criterion data
];

const wasi = new WASI(["bench", ...benchArgs], ["CARGO_TARGET_DIR=."], fds);

const module = new WebAssembly.Module(readbuffer(wasmPath));
const instance = new WebAssembly.Instance(module, {
  wasi_snapshot_preview1: wasi.wasiImport,
});

wasi.start(instance);

// criterion wrote its estimates into the in-memory preopen; emit them on one
// marked line so bench-d8.sh can persist them to disk for the report tool
// (d8 has no real filesystem to write through).
const estimates = {};
(function walk(dir, prefix) {
  for (const [name, entry] of dir.contents) {
    const path = prefix ? `${prefix}/${name}` : name;
    if (entry.contents !== undefined) walk(entry, path);
    else if (path.endsWith("estimates.json"))
      estimates[path] = new TextDecoder().decode(entry.data);
  }
})(fds[3].dir, "");
print("===CRITERION-DATA===" + JSON.stringify(estimates));
