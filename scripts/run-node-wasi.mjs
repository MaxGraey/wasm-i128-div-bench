/*
 * node:wasi loader for the criterion benchmark compiled to wasm32-wasip1.
 *
 * criterion runs unmodified: WASI supplies the monotonic clock that backs
 * std::time::Instant, argv for configure_from_args, and a preopened directory
 * for its result data. Arguments after the wasm path are forwarded verbatim to
 * criterion (filters, --sample-size, --save-baseline, ...).
 */
import { readFile } from 'node:fs/promises'
import { WASI } from 'node:wasi'
import { argv, exit, env } from 'node:process'

const wasmPath = argv[2]
const dataDir = env.WASI_BENCH_DIR ?? 'target/wasi'

const wasi = new WASI({
  version: 'preview1',
  args: ['bench', ...argv.slice(3)],
  env: { CARGO_TARGET_DIR: dataDir },
  preopens: { [dataDir]: dataDir },
  returnOnExit: true,
})

const module = await WebAssembly.compile(await readFile(wasmPath))
const instance = await WebAssembly.instantiate(module, wasi.getImportObject())

exit(wasi.start(instance))
