# wasm-i128-div-bench

Microbenchmarks for 128-bit integer division: a reciprocal-based implementation
(stays on `u64` limbs, no compiler-rt libcall) vs the native `u128` / `i128`
operators (`__udivti3` / `__umodti3` on wasm). Both run through criterion.

## Requirements

- **Rust Nightly** with `wasm32-wasip1` target
- **Node JS** >= 22

## Run the benchmarks

Native:

```bash
npm run bench:native
```

WebAssembly under Node (builds to `wasm32-wasip1`, runs the same criterion
benchmark via `node:wasi`):

```bash
npm run bench:node
```

The same wasm under wasmtime:

```bash
npm run bench:wasmtime
```

Filter by name or forward any criterion flag with `--`:

```bash
npm run bench:node -- udiv128
npm run bench:wasmtime -- --sample-size 50 reciprocal
```

## Test

```bash
cargo test
```

## Extended wide arithmetic proposal

A sketch of WebAssembly instructions that would let an engine expose 128-bit
division natively, extending the [wide-arithmetic] proposal - which adds
`i64.add128`, `i64.sub128`, `i64.mul_wide_u` but leaves out division.
`i64.recip128` prepares the divisor with the Moeller-Granlund reciprocal kernel
measured here - 2-by-1 for a 64-bit divisor, 3-by-2 for a full-width one;
`i64.divrem_recip128` consumes that prep to give the quotient and remainder
together. Splitting prep from divide is the point: for a loop-invariant divisor
the reciprocal is computed once and hoisted out of the loop. `i64.div_recip128`
and `i64.rem_recip128` are narrowing projections of `i64.divrem_recip128` - the
reciprocal kernel produces both results jointly (its correction steps need the
remainder to fix the quotient), so they drop a result word, not run a cheaper path.

[wide-arithmetic]: https://github.com/WebAssembly/wide-arithmetic

Conventions: each 128-bit value is a `(low, high)` pair of `i64`s on the stack
(as in `i64.add128`); for a value `V` the halves satisfy `V = V_hi*2^64 + V_lo`.
The family is unsigned. `D` denotes the normalized divisor `d_hi*2^64 + d_lo`.

`i64.recip128` - divisor prep (normalize + reciprocal):

```
i64.recip128 : [i64 i64] -> [i64 i64 i64 i64]

  operands  y_lo y_hi
  results   d_lo d_hi rcp lsh

  y_hi != 0  (full-width divisor, 3-by-2):
    lsh = clz(y_hi)
    D = Y << lsh                   ; normalized, bit 127 set
    rcp = floor((2^192 - 1) / D) - 2^64

  y_hi == 0  (64-bit divisor, 2-by-1):
    lsh = clz(y_lo)
    d_lo = y_lo << lsh,  d_hi = 0  ; normalized, bit 63 set
    rcp = floor((2^128 - 1) / d_lo) - 2^64

  traps if y == 0
```

`i64.divrem_recip128` - quotient and remainder:

```
i64.divrem_recip128 : [i64 i64 i64 i64 i64 i64] -> [i64 i64 i64 i64]

  operands  x_lo x_hi d_lo d_hi rcp lsh   ; from i64.recip128 of Y
  results   q_lo q_hi r_lo r_hi
    q_hi*2^64 + q_lo = floor(X / Y),  Y = D >> lsh
    r_hi*2^64 + r_lo = X mod Y
```

It picks the kernel from `d_hi` - 2-by-1 when `d_hi == 0` (64-bit divisor,
quotient up to 128 bits), 3-by-2 otherwise (then `q_hi == 0`). It does not trap;
the divisor check, including divide-by-zero, lives in `i64.recip128`.

`i64.div_recip128` and `i64.rem_recip128` narrow it to one result - same operands
and work, one limb pair dropped:

```
  i64.div_recip128 : [i64 i64 i64 i64 i64 i64] -> [i64 i64]   ; results q_lo q_hi
  i64.rem_recip128 : [i64 i64 i64 i64 i64 i64] -> [i64 i64]   ; results r_lo r_hi
```

A full unsigned 128-bit divide needs no branch - the kernel choice is inside the
divide:

```
divrem128_u(x_lo, x_hi, y_lo, y_hi) -> (q_lo, q_hi, r_lo, r_hi)

  d_lo, d_hi, rcp, lsh   = i64.recip128(y_lo, y_hi)
  q_lo, q_hi, r_lo, r_hi = i64.divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh)
  -> (q_lo, q_hi, r_lo, r_hi)
```

The same in wat - `recip128` once, then one `divrem_recip128`; the four result
words land on the stack, no result locals:

```wasm
(func $divrem128_u (param $x_lo i64) (param $x_hi i64)
                   (param $y_lo i64) (param $y_hi i64)
                   (result i64 i64 i64 i64)  ;; q_lo q_hi r_lo r_hi
  (local $d_lo i64)
  (local $d_hi i64)
  (local $rcp i64)
  (local $lsh i64)

  ;; reciprocal divisor precomp and can be utilized in cse/licm
  local.get $y_lo
  local.get $y_hi
  i64.recip128          ;; -> d_lo d_hi rcp lsh
  local.set $lsh
  local.set $rcp
  local.set $d_hi
  local.set $d_lo

  ;; q, r = divmod(x, y)
  local.get $x_lo
  local.get $x_hi
  local.get $d_lo
  local.get $d_hi
  local.get $rcp
  local.get $lsh
  i64.divrem_recip128   ;; -> q_lo q_hi r_lo r_hi
)
```

Signed division reuses these: `abs` both operands, run the unsigned kernel, then
negate `q` when the operand signs differ and `r` when the dividend is negative.
