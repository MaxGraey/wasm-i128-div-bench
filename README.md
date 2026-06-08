# WebAssembly 128-bit Division Benchmarks

Microbenchmarks for 128-bit integer division. A reciprocal-based implementation
(stays on `u64` limbs, no compiler-rt libcall) vs the native `u128` / `i128`
operators (`__udivti3` / `__umodti3` on wasm). Both run through criterion.

## Requirements

- **Rust Nightly** with `wasm32-wasip1` target
- **Node JS** >= 22

## Run the benchmarks

Aggregate the runs into [markdown files in report folder](report/).

```bash
npm run bench
```

or

```bash
scripts/bench.sh
```

Pass `--features native-wide-mul` to swap `mul128` from the `u64`-limb synthesis
to a single native `u128` multiply (`i64.mul_wide_u`) across every backend.

```bash
scripts/bench.sh --features native-wide-mul
```

## Test

```bash
cargo test
```

## Extended wide arithmetic proposal

A sketch of WebAssembly instructions that would let an engine expose 128-bit
division natively, extending the [wide-arithmetic] proposal, which adds
`i64.add128`, `i64.sub128`, `i64.mul_wide_u` but leaves out division.
`i64.recip128` prepares the divisor with the Moeller-Granlund reciprocal kernel
measured here, 2-by-1 for a 64-bit divisor and 3-by-2 for a full-width one.
`i64.divrem_recip128` consumes that prep to give the quotient and remainder
together. Splitting prep from divide is the point. For a loop-invariant divisor
the reciprocal is computed once and hoisted out of the loop. `i64.div_recip128`
and `i64.rem_recip128` are narrowing projections of `i64.divrem_recip128`. The
reciprocal kernel produces both results jointly (its correction steps need the
remainder to fix the quotient), so they drop a result word, not run a cheaper path.

[wide-arithmetic]: https://github.com/WebAssembly/wide-arithmetic

Conventions. Each 128-bit value is a `(low, high)` pair of `i64`s on the stack
(as in `i64.add128`). For a value `V` the halves satisfy `V = V_hi*2^64 + V_lo`.
The family is unsigned. `D` denotes the normalized divisor `d_hi*2^64 + d_lo`.

`i64.recip128` performs divisor prep (normalize + reciprocal).

```
i64.recip128 : [i64 i64] -> [i32 i64 i64 i64]

  operands  y_lo y_hi
  results   lsh rcp d_lo d_hi

  y_hi != 0: ; (full-width divisor, 3-by-2)
    lsh = clz(y_hi)
    D   = Y << lsh                          ; normalized, bit 127 set
    rcp = floor((2^192 - 1) / D) - 2^64

  y_hi == 0: ; (64-bit divisor, 2-by-1)
    lsh  = clz(y_lo)
    d_lo = y_lo << lsh,  d_hi = 0           ; normalized, bit 63 set
    rcp  = floor((2^128 - 1) / d_lo) - 2^64

  traps if y == 0
```

`i64.divrem_recip128` returns the quotient and remainder.

```
i64.divrem_recip128 : [i32 i64 i64 i64 i64 i64] -> [i64 i64 i64 i64]

  operands  lsh rcp d_lo d_hi x_lo x_hi   ; prep from i64.recip128(Y), then dividend X
  results   q_lo q_hi r_lo r_hi
    q_hi*2^64 + q_lo = floor(X / Y),  Y = D >> lsh
    r_hi*2^64 + r_lo = X mod Y
```

It picks the kernel from `d_hi`, 2-by-1 when `d_hi == 0` (64-bit divisor,
quotient up to 128 bits) and 3-by-2 otherwise (then `q_hi == 0`). It does not trap.
The divisor check, including divide-by-zero, lives in `i64.recip128`.

A full unsigned 128-bit divide needs no branch. The kernel choice is inside the
divide.

```
divrem128_u(x_lo, x_hi, y_lo, y_hi) -> (q_lo, q_hi, r_lo, r_hi)

  lsh, rcp, d_lo, d_hi   = i64.recip128(y_lo, y_hi)
  q_lo, q_hi, r_lo, r_hi = i64.divrem_recip128(lsh, rcp, d_lo, d_hi, x_lo, x_hi)
```

The same in wat. `recip128` runs once, then one `divrem_recip128`. The prep tuple
stays on the stack between them, so the body needs no locals at all.

```wasm
(func $divrem128_u (param $x_lo i64) (param $x_hi i64)
                   (param $y_lo i64) (param $y_hi i64)
                        ;; q_lo q_hi r_lo r_hi
                   (result  i64  i64  i64  i64)
  local.get $y_lo
  local.get $y_hi
  i64.recip128
  ;; -> lsh rcp d_lo d_hi

  local.get $x_lo
  local.get $x_hi
  ;; <- lsh rcp d_lo d_hi x_lo x_hi
  i64.divrem_recip128
  ;; -> q_lo q_hi r_lo r_hi
)
```

Signed division reuses these. `abs` both operands, run the unsigned kernel, then
negate `q` when the operand signs differ and `r` when the dividend is negative.
