/*
 * Native 128-bit division baseline.
 *
 * Mirrors the public API of divrem_by_recip, but every divide goes through the
 * native u128 / i128 operators (the compiler-rt __udivti3 / __umodti3 libcall on
 * wasm). This is the baseline the reciprocal path is measured against.
 */

pub fn udivrem128(x: u128, y: u128) -> (u128, u128) {
    (x / y, x % y)
}

pub fn udiv128(x: u128, y: u128) -> u128 {
    x / y
}

pub fn urem128(x: u128, y: u128) -> u128 {
    x % y
}

pub fn sdivrem128(x: i128, y: i128) -> (i128, i128) {
    (x / y, x % y)
}

pub fn sdiv128(x: i128, y: i128) -> i128 {
    x / y
}

pub fn srem128(x: i128, y: i128) -> i128 {
    x % y
}

pub fn divrem_with_loop_invariant_divisor(x: (u64, u64), iters: usize) -> (u64, u64) {
    let d_hi = (x.0 & 0x7fff_ffff_ffff_ffff) | 1;
    let d = ((d_hi as u128) << 64) | x.1 as u128;

    let mut acc = (0u64, 0u64);
    let mut i = 0usize;

    while i < iters {
        let num = (((x.0 ^ (i as u64)) as u128) << 64) | x.1 as u128;
        let q = num / d;
        let r = num % d;

        acc.0 |= q as u64;
        acc.1 |= (r as u64) | ((r >> 64) as u64);

        i += 1;
    }

    acc
}

/* Criterion micro-benchmarks for the public API. Same seeded operands as the
 * reciprocal backend, so the two can be compared label for label. */
use crate::utils::*;

use criterion::Criterion;
use rand::{RngExt, SeedableRng, rngs::SmallRng};
use std::hint::black_box;

pub fn bench_udivrem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("builtin/udivrem128", |b| {
        b.iter(|| udivrem128(black_box(x), black_box(y)))
    });
}

pub fn bench_udiv128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("builtin/udiv128", |b| {
        b.iter(|| udiv128(black_box(x), black_box(y)))
    });
}

pub fn bench_urem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("builtin/urem128", |b| {
        b.iter(|| urem128(black_box(x), black_box(y)))
    });
}

pub fn bench_sdivrem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("builtin/sdivrem128", |b| {
        b.iter(|| sdivrem128(black_box(x), black_box(y)))
    });
}

pub fn bench_sdiv128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("builtin/sdiv128", |b| {
        b.iter(|| sdiv128(black_box(x), black_box(y)))
    });
}

pub fn bench_srem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("builtin/srem128", |b| {
        b.iter(|| srem128(black_box(x), black_box(y)))
    });
}

pub fn bench_divrem_with_loop_invariant_divisor(c: &mut Criterion) {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let x = (rng.random::<u64>(), rng.random::<u64>());
    c.bench_function("builtin/divrem_loop_invariant", |b| {
        b.iter(|| divrem_with_loop_invariant_divisor(black_box(x), black_box(LOOP_INVAR_ITERS)))
    });
}
