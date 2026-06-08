/*
 * Reciprocal-based 128-bit division.
 *
 * Based on Algorithm 2 of Moeller & Granlund
 * "Improved division by invariant integers".
 *
 * The whole point of this path is to replace the compiler-rt 128-bit division
 * libcall (__udivti3 / __umodti3 on wasm) with a multiply-heavy sequence that
 * stays on u64 limbs. Everything below operates on explicit (hi, lo) limbs with
 * manual carry / borrow, so no native u128 arithmetic is emitted - native u128
 * appears only at the public API boundary, where the split / join lower to plain
 * register moves.
 */

use crate::utils::*;
use std::hint::{likely, unlikely};

/* Reciprocal lookup table. Entry table[i] = floor(0x7fd00 / (i + 256)).
 * 256 x u16 = 512 bytes */
const RECIPROCAL_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = (0x7fd00 / (i as u32 + 256)) as u16;
        i += 1;
    }
    table
};


#[inline(always)]
fn split(x: u128) -> (u64, u64) {
    ((x >> 64) as u64, x as u64)
}

#[inline(always)]
fn join(v: (u64, u64)) -> u128 {
    ((v.0 as u128) << 64) | (v.1 as u128)
}

#[inline(always)]
fn add128(a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    let (lo, carry) = a.1.overflowing_add(b.1);
    let hi = a.0.wrapping_add(b.0).wrapping_add(carry as u64);
    (hi, lo)
}

#[inline(always)]
fn add128_u64(a: (u64, u64), b: u64) -> (u64, u64) {
    let (lo, carry) = a.1.overflowing_add(b);
    (a.0.wrapping_add(carry as u64), lo)
}

#[inline(always)]
fn sub128(a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    let (lo, borrow) = a.1.overflowing_sub(b.1);
    let hi = a.0.wrapping_sub(b.0).wrapping_sub(borrow as u64);
    (hi, lo)
}

/* 128-bit logical shifts over the (hi, lo) limb pair, n in 0..=63, as plain
 * shifts (stable Rust, no funnel intrinsic). The "<< 1" preshift with "63 - n"
 * zeroes the cross term at n == 0, where a bare "hi << (64 - n)" would wrap to
 * "hi << 0" since AArch64/wasm mask the shift count mod 64:
 *
 *   shr  lo' = (lo >> n) | ((hi << 1) << (63 - n))   -> (lo >> n) | (hi << (64-n)), = lo at n==0
 *   shl  hi' = (hi << n) | ((lo >> 1) >> (63 - n))
 *
 * x86 folds each pair to shrd, ARM/wasm synthesize. See https://godbolt.org/z/Y44bfKedh */
#[inline(always)]
fn shl128(v: (u64, u64), n: u32) -> (u64, u64) {
    ((v.0 << n) | ((v.1 >> 1) >> (63 - n)), v.1 << n)
}

#[inline(always)]
fn shr128(v: (u64, u64), n: u32) -> (u64, u64) {
    (v.0 >> n, (v.1 >> n) | ((v.0 << 1) << (63 - n)))
}

#[inline(always)]
fn shl128_wide(v: (u64, u64), n: u32) -> (u64, u64, u64) {
    let (hi, lo) = shl128(v, n);
    ((v.0 >> 1) >> (63 - n), hi, lo)
}

#[inline(always)]
fn mul128(a: u64, b: u64) -> (u64, u64) {
    let al = a & 0xffff_ffff;
    let ah = a >> 32;

    let bl = b & 0xffff_ffff;
    let bh = b >> 32;

    let ll = al * bl;
    let lh = al * bh;
    let hl = ah * bl;
    let hh = ah * bh;

    let cs = (ll >> 32) + (lh & 0xffff_ffff) + (hl & 0xffff_ffff);
    let lo = (cs << 32) | (ll & 0xffff_ffff);
    let hi = hh + (lh >> 32) + (hl >> 32) + (cs >> 32);

    (hi, lo)
}

/* Computes floor((2^128 - 1) / d) - 2^64 for normalized d (top bit set). */
#[inline(always)]
fn reciprocal_2by1(d: u64) -> u64 {
    debug_assert!(d & 0x8000_0000_0000_0000 != 0, "d must be normalized");

    let v0 = RECIPROCAL_TABLE[((d >> 55) - 256) as usize] as u32;

    let d40  = (d >> 24) + 1;
    let v0sq = v0.wrapping_mul(v0) as u64;
    let term = (v0sq.wrapping_mul(d40) >> 40) as u32;
    let v1   = (v0 << 11).wrapping_sub(term).wrapping_sub(1) as u64;

    let v2 = (v1 << 13).wrapping_add(
        v1.wrapping_mul(
            0x1000_0000_0000_0000u64.wrapping_sub(v1.wrapping_mul(d40))
        ) >> 47,
    );

    let d0  = d & 1;
    let d63 = (d >> 1).wrapping_add(d0); // ceil(d/2)
    let e   = ((v2 >> 1) & 0u64.wrapping_sub(d0)).wrapping_sub(v2.wrapping_mul(d63));
    let v3  = (mul128(v2, e).0 >> 1).wrapping_add(v2 << 31);

    v3.wrapping_sub(add128_u64(mul128(v3, d), d).0).wrapping_sub(d)
}

/* Reciprocal for a normalized 3-by-2 divisor d (high limb top bit set). */
#[inline]
fn reciprocal_3by2(d: (u64, u64)) -> u64 {
    let mut v = reciprocal_2by1(d.0);
    let mut p = d.0.wrapping_mul(v);
    p = p.wrapping_add(d.1);

    if p < d.1 {
        v = v.wrapping_sub(1);
        if p >= d.0 {
            v = v.wrapping_sub(1);
            p = p.wrapping_sub(d.0);
        }
        p = p.wrapping_sub(d.0);
    }

    let t = mul128(v, d.1);
    p = p.wrapping_add(t.0);

    if p < t.0 {
        v = v.wrapping_sub(1);
        if p >= d.0 && (p > d.0 || t.1 >= d.1) {
            v = v.wrapping_sub(1);
        }
    }

    v
}

/* Divides the 128-bit u by the normalized 64-bit d with precomputed v,
 * returns (quotient, remainder). */
#[inline]
fn udivrem_2by1(u: (u64, u64), d: u64, v: u64) -> (u64, u64) {
    let q = add128(mul128(v, u.0), u);

    let q_lo = q.1;
    let mut q_hi = q.0.wrapping_add(1);
    let mut r = u.1.wrapping_sub(q_hi.wrapping_mul(d));

    if r > q_lo {
        q_hi = q_hi.wrapping_sub(1);
        r = r.wrapping_add(d);
    }

    if r >= d {
        q_hi = q_hi.wrapping_add(1);
        r = r.wrapping_sub(d);
    }

    (q_hi, r)
}

/* Divides the 192-bit (u2, u1, u0) by the normalized 128-bit d with precomputed
 * v, returns (quotient, remainder limb pair). */
#[inline]
fn udivrem_3by2(
    u2: u64,
    u1: u64,
    u0: u64,
    d: (u64, u64),
    v: u64,
) -> (u64, (u64, u64)) {
    let q = add128(mul128(v, u2), (u2, u1));

    let q_lo = q.1;
    let mut q_hi = q.0;

    let r1 = u1.wrapping_sub(q_hi.wrapping_mul(d.0));
    let t = mul128(d.1, q_hi);

    let mut r = sub128(sub128((r1, u0), t), d);
    let r1 = r.0;

    q_hi = q_hi.wrapping_add(1);

    if r1 >= q_lo {
        q_hi = q_hi.wrapping_sub(1);
        r = add128(r, d);
    }

    if r >= d {
        q_hi = q_hi.wrapping_add(1);
        r = sub128(r, d);
    }

    (q_hi, r)
}

/* Division by zero is fatal. Messages mirror the wasm integer trap text V8
 * reports for i64.div_u / i64.rem_u by zero ("divide by zero" / "remainder by
 * zero"), so the failure reads identically to the native u64/u64 path. */
#[cold]
#[inline(never)]
fn divide_by_zero_trap() -> ! {
    panic!("divide by zero")
}

// #[cold]
// #[inline(never)]
// fn remainder_by_zero_trap() -> ! {
//     panic!("remainder by zero")
// }

/* i128::MIN / -1 overflows the signed quotient. Mirrors the wasm i64.div_s trap
 * ("integer overflow"); i64.rem_s leaves this case untrapped (remainder 0). */
#[cold]
#[inline(never)]
fn integer_overflow_trap() -> ! {
    panic!("integer overflow")
}

/* The proposed wide-arithmetic instructions, modeled on u64 limbs. Operands and
 * results use the proposal's low-word-first order (y_lo, y_hi), while the kernels
 * above stay on the file's (hi, lo) limb pairs. i64.recip128 prepares the divisor
 * once so a loop-invariant reciprocal hoists out of the loop, i64.divrem_recip128
 * then divides. The reciprocal kernel computes quotient and remainder jointly, so
 * a caller needing only one drops the other word - there is no cheaper narrow form. */

/* i64.recip128 : [y_lo y_hi] -> [d_lo d_hi rcp lsh]
 * Normalize the divisor and precompute its reciprocal. y_hi == 0 takes the
 * 2-by-1 kernel (d_hi = 0), else 3-by-2. Traps on y == 0. */
fn recip128(y_lo: u64, y_hi: u64) -> (u64, u64, u64, u64) {
    if likely(y_hi != 0) {
        let lsh = y_hi.leading_zeros();
        let (d_hi, d_lo) = shl128((y_hi, y_lo), lsh);
        return (d_lo, d_hi, reciprocal_3by2((d_hi, d_lo)), lsh as u64);
    }

    // 64-bit divisor (y_hi == 0): 2-by-1 kernel.
    if unlikely(y_lo == 0) {
        divide_by_zero_trap();
    }

    let lsh = y_lo.leading_zeros();
    let d_lo = y_lo << lsh;
    (d_lo, 0, reciprocal_2by1(d_lo), lsh as u64)
}

/* i64.divrem_recip128 : [x_lo x_hi d_lo d_hi rcp lsh] -> [q_lo q_hi r_lo r_hi]
 * Joint quotient and remainder. d_hi == 0 selects the 2-by-1 kernel (quotient up
 * to 128 bits), else 3-by-2 (q_hi == 0). The Y > X and lsh == 0 shortcuts skip
 * the divide multiplies, the reciprocal having already been spent in
 * i64.recip128. q and r come out together - the correction steps need r to fix
 * the quotient, so a divide-only or modulo-only caller just drops a result word. */
fn divrem_recip128(
    x_lo: u64,
    x_hi: u64,
    d_lo: u64,
    d_hi: u64,
    rcp: u64,
    lsh: u64,
) -> (u64, u64, u64, u64) {
    if unlikely(d_hi == 0) {
        let lsh = lsh as u32;
        let (xn_ex, xn_hi, xn_lo) = shl128_wide((x_hi, x_lo), lsh);

        let (q1, r1) = udivrem_2by1((xn_ex, xn_hi), d_lo, rcp);
        let (q0, r0) = udivrem_2by1((r1, xn_lo), d_lo, rcp);

        return (q0, q1, r0 >> lsh, 0);
    }

    if (d_hi >> lsh) > x_hi {
        return (0, 0, x_lo, x_hi);
    }

    if unlikely(lsh == 0) {
        return if (d_hi < x_hi) || (d_hi == x_hi && d_lo <= x_lo) {
            let (r_hi, r_lo) = sub128((x_hi, x_lo), (d_hi, d_lo));
            (1, 0, r_lo, r_hi)
        } else {
            (0, 0, x_lo, x_hi)
        };
    }

    let lsh = lsh as u32;
    let (xn_ex, xn_hi, xn_lo) = shl128_wide((x_hi, x_lo), lsh);

    let (q, r) = udivrem_3by2(xn_ex, xn_hi, xn_lo, (d_hi, d_lo), rcp);
    let (r_hi, r_lo) = shr128(r, lsh);
    (q, 0, r_lo, r_hi)
}


/* Unsigned 128-bit divide-and-remainder. Traps on division by zero. */
pub fn udivrem128(x: u128, y: u128) -> (u128, u128) {
    // if unlikely(y == 0) {
    //     divide_by_zero_trap();
    // }

    let (x_hi, x_lo) = split(x);
    let (y_hi, y_lo) = split(y);

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (q_lo, q_hi, r_lo, r_hi) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    (join((q_hi, q_lo)), join((r_hi, r_lo)))
}

pub fn udiv128(x: u128, y: u128) -> u128 {
    // if unlikely(y == 0) {
    //     divide_by_zero_trap();
    // }

    let (x_hi, x_lo) = split(x);
    let (y_hi, y_lo) = split(y);

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (q_lo, q_hi, _, _) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    join((q_hi, q_lo))
}

pub fn urem128(x: u128, y: u128) -> u128 {
    // if unlikely(y == 0) {
    //     remainder_by_zero_trap();
    // }

    let (x_hi, x_lo) = split(x);
    let (y_hi, y_lo) = split(y);

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (_, _, r_lo, r_hi) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    join((r_hi, r_lo))
}

/* Signed wrappers divide the magnitudes through i64.recip128 + i64.divrem_recip128,
 * then truncate toward zero by sign - negate the quotient when the operand signs
 * differ, the remainder when the dividend is negative. INT_MIN / -1 traps in
 * sdiv128 / sdivrem128 (integer overflow, like i64.div_s); srem128 returns 0,
 * matching i64.rem_s. */
pub fn sdivrem128(x: i128, y: i128) -> (i128, i128) {
    // if unlikely(y == 0) {
    //     divide_by_zero_trap();
    // }

    if unlikely(x == i128::MIN && y == -1) {
        integer_overflow_trap();
    }

    let (x_hi, x_lo) = split(x.unsigned_abs());
    let (y_hi, y_lo) = split(y.unsigned_abs());

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (q_lo, q_hi, r_lo, r_hi) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    let q = join((q_hi, q_lo)) as i128;
    let r = join((r_hi, r_lo)) as i128;

    let q = if (x < 0) != (y < 0) {
        q.wrapping_neg()
    } else {
        q
    };

    let r = if x < 0 {
        r.wrapping_neg()
    } else {
        r
    };

    (q, r)
}

pub fn sdiv128(x: i128, y: i128) -> i128 {
    // if unlikely(y == 0) {
    //     divide_by_zero_trap();
    // }

    if unlikely(x == i128::MIN && y == -1) {
        integer_overflow_trap();
    }

    let (x_hi, x_lo) = split(x.unsigned_abs());
    let (y_hi, y_lo) = split(y.unsigned_abs());

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (q_lo, q_hi, _, _) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    let q = join((q_hi, q_lo)) as i128;

    if (x < 0) != (y < 0) {
        q.wrapping_neg()
    } else {
        q
    }
}

pub fn srem128(x: i128, y: i128) -> i128 {
    // if unlikely(y == 0) {
    //     remainder_by_zero_trap();
    // }

    let (x_hi, x_lo) = split(x.unsigned_abs());
    let (y_hi, y_lo) = split(y.unsigned_abs());

    let (d_lo, d_hi, rcp, lsh) = recip128(y_lo, y_hi);
    let (_, _, r_lo, r_hi) = divrem_recip128(x_lo, x_hi, d_lo, d_hi, rcp, lsh);

    let r = join((r_hi, r_lo)) as i128;

    if x < 0 {
        r.wrapping_neg()
    } else {
        r
    }
}

/* Benchmark scaffold for the invariant-divisor pattern, and a faithful u128
 * division. The divisor is fixed before the loop, so i64.recip128 runs once and
 * only the per-step i64.divrem_recip128 stays in the loop. Each iteration
 * computes num / d and num % d for a varying u128 num, bit-identical to
 * the native operators (see divrem_with_loop_invariant_divisor_matches_native).
 * Results fold into acc with |= so the loop cannot be optimized away. */
pub fn divrem_with_loop_invariant_divisor(x: (u64, u64), iters: usize) -> (u64, u64) {
    // Invariant divisor in [2^64, 2^127) - high limb nonzero with top bit clear,
    // so the 3-by-2 path runs with a real normalization shift (lsh in 1..=63).
    let d = ((x.0 & 0x7fff_ffff_ffff_ffff) | 1, x.1);

    // i64.recip128 hoisted - normalized divisor and reciprocal computed once.
    let (d_lo, d_hi, rcp, lsh) = recip128(d.1, d.0);

    let mut acc = (0u64, 0u64);
    let mut i = 0usize;
    while i < iters {
        // Varying dividend; i64.divrem_recip128 reuses the hoisted prep.
        let num = (x.0 ^ (i as u64), x.1);
        let (q_lo, q_hi, r_lo, r_hi) = divrem_recip128(num.1, num.0, d_lo, d_hi, rcp, lsh);

        acc.0 |= q_lo | q_hi;
        acc.1 |= r_lo | r_hi;

        i += 1;
    }

    acc
}

/* Criterion micro-benchmarks for the public API. Inputs come from a seeded
 * SmallRng so this backend and divrem_builtin see identical operands. */
use criterion::Criterion;
use rand::{RngExt, SeedableRng, rngs::SmallRng};
use std::hint::black_box;

pub fn bench_udivrem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("reciprocal/udivrem128", |b| {
        b.iter(|| udivrem128(black_box(x), black_box(y)))
    });
}

pub fn bench_udiv128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("reciprocal/udiv128", |b| {
        b.iter(|| udiv128(black_box(x), black_box(y)))
    });
}

pub fn bench_urem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<u128>();
    c.bench_function("reciprocal/urem128", |b| {
        b.iter(|| urem128(black_box(x), black_box(y)))
    });
}

pub fn bench_sdivrem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("reciprocal/sdivrem128", |b| {
        b.iter(|| sdivrem128(black_box(x), black_box(y)))
    });
}

pub fn bench_sdiv128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("reciprocal/sdiv128", |b| {
        b.iter(|| sdiv128(black_box(x), black_box(y)))
    });
}

pub fn bench_srem128(c: &mut Criterion) {
    let (x, y) = rand_operands::<i128>();
    c.bench_function("reciprocal/srem128", |b| {
        b.iter(|| srem128(black_box(x), black_box(y)))
    });
}

pub fn bench_divrem_with_loop_invariant_divisor(c: &mut Criterion) {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let x = (rng.random::<u64>(), rng.random::<u64>());
    c.bench_function("reciprocal/divrem_loop_invariant", |b| {
        b.iter(|| divrem_with_loop_invariant_divisor(black_box(x), black_box(LOOP_INVAR_ITERS)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::SEED;
    use rand::{RngExt, SeedableRng, rngs::SmallRng};

    #[test]
    fn umul_matches_native() {
        let mut rng = SmallRng::seed_from_u64(SEED);
        for _ in 0..100_000 {
            let a: u64 = rng.random();
            let b: u64 = rng.random();
            let got = mul128(a, b);
            let want = (a as u128) * (b as u128);
            assert_eq!(((got.0 as u128) << 64) | got.1 as u128, want, "{a} * {b}");
        }
    }

    #[test]
    fn check_shl128() {
        // sh == 0 is identity
        assert_eq!(shl128((1, 2), 0), (1, 2));
        assert_eq!(shl128((u64::MAX, u64::MAX), 0), (u64::MAX, u64::MAX));

        // low-limb top bit crosses into the high limb
        assert_eq!(shl128((0, 0x8000_0000_0000_0000), 1), (1, 0));
        assert_eq!(shl128((1, 0), 1), (2, 0));

        // sh == 63, the maximal in-limb shift
        assert_eq!(shl128((0, 1), 63), (0, 0x8000_0000_0000_0000));
        assert_eq!(shl128((0, 2), 63), (1, 0));

        // all ones, partial cross
        assert_eq!(shl128((u64::MAX, u64::MAX), 4), (u64::MAX, 0xffff_ffff_ffff_fff0));
    }

    #[test]
    fn check_shr128() {
        // sh == 0 is identity
        assert_eq!(shr128((1, 2), 0), (1, 2));
        assert_eq!(shr128((u64::MAX, u64::MAX), 0), (u64::MAX, u64::MAX));

        // high-limb low bit crosses into the low limb
        assert_eq!(shr128((1, 0), 1), (0, 0x8000_0000_0000_0000));
        assert_eq!(shr128((0, 2), 1), (0, 1));

        // sh == 63, the maximal in-limb shift
        assert_eq!(shr128((1, 0), 63), (0, 2));
        assert_eq!(shr128((0x8000_0000_0000_0000, 0), 63), (1, 0));

        // all ones, partial cross
        assert_eq!(shr128((u64::MAX, u64::MAX), 4), (0x0fff_ffff_ffff_ffff, u64::MAX));
    }

    #[test]
    fn divrem_with_loop_invariant_divisor_matches_native() {
        // Mirror the divisor and dividend the function derives, then divide
        // natively, so a mismatch means the reciprocal path diverged.
        let reference = |x: (u64, u64), iters: usize| -> (u64, u64) {
            let d = join(((x.0 & 0x7fff_ffff_ffff_ffff) | 1, x.1));
            let mut acc = (0u64, 0u64);
            for i in 0..iters {
                let num = join((x.0 ^ (i as u64), x.1));
                acc.0 |= (num / d) as u64;
                let r = num % d;
                acc.1 |= (r as u64) | ((r >> 64) as u64);
            }
            acc
        };

        let mut rng = SmallRng::seed_from_u64(SEED);
        let mut cases = vec![
            ((0u64, 0u64), 0usize),
            ((0, 0), 1),
            ((u64::MAX, u64::MAX), 1),
            ((u64::MAX, u64::MAX), 64),
            ((1, 0), 10),
            ((0, 1), 10),
        ];
        for _ in 0..2_000 {
            let x = (rng.random::<u64>(), rng.random::<u64>());
            cases.push((x, rng.random_range(0..300usize)));
        }

        for &(x, iters) in &cases {
            assert_eq!(
                divrem_with_loop_invariant_divisor(x, iters),
                reference(x, iters),
                "x={x:?} iters={iters}",
            );
        }
    }

    #[test]
    fn unsigned_random() {
        let mut rng = SmallRng::seed_from_u64(SEED);
        for _ in 0..500_000 {
            let x: u128 = rng.random();
            let mut y: u128 = rng.random();
            // Bias toward small divisors too, exercising the 2-by-1 path.
            if rng.random::<u64>() & 1 == 0 {
                y = (rng.random::<u64>() >> (rng.random::<u64>() % 64)) as u128;
            }
            if y == 0 {
                continue;
            }
            assert_eq!(udivrem128(x, y), (x / y, x % y), "x={x} y={y}");
        }
    }

    #[test]
    fn unsigned_edges() {
        let vals = [
            0u128,
            1,
            2,
            u64::MAX as u128,
            (u64::MAX as u128) + 1,
            u128::MAX,
            u128::MAX - 1,
            0x8000_0000_0000_0000_0000_0000_0000_0000,
            // 2^127 + k: lsh==0 path, q=1 with a cross-limb borrow (d_lo != 0).
            (1u128 << 127) + 1,
            (1u128 << 127) + 3,
            0xffff_ffff_ffff_ffff_0000_0000_0000_0000,
            0x0000_0000_0000_0001_0000_0000_0000_0000,
            // hi=3, lo=1: forces the 3-by-2 kernel with a nonzero normalized low limb.
            0x0000_0000_0000_0003_0000_0000_0000_0001,
        ];
        for &x in &vals {
            for &y in &vals {
                if y == 0 {
                    continue;
                }
                assert_eq!(udivrem128(x, y), (x / y, x % y), "x={x} y={y}");
            }
        }
    }

    #[test]
    fn signed_random() {
        let mut rng = SmallRng::seed_from_u64(SEED);
        for _ in 0..500_000 {
            let x = rng.random::<u128>() as i128;
            let y = rng.random::<u128>() as i128;
            if y == 0 {
                continue;
            }
            // INT_MIN / -1 traps sdiv128 / sdivrem128 now, srem128 stays 0.
            if x == i128::MIN && y == -1 {
                assert_eq!(srem128(x, y), 0);
                continue;
            }
            assert_eq!(sdivrem128(x, y), (x / y, x % y), "x={x} y={y}");
        }
    }

    #[test]
    fn signed_edges() {
        let vals = [
            0i128,
            1,
            -1,
            2,
            -2,
            i128::MAX,
            i128::MIN,
            i128::MIN + 1,
            i64::MAX as i128,
            i64::MIN as i128
        ];
        for &x in &vals {
            for &y in &vals {
                if y == 0 || (x == i128::MIN && y == -1) {
                    continue;
                }
                assert_eq!(sdivrem128(x, y), (x / y, x % y), "x={x} y={y}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "integer overflow")]
    fn sdiv_overflow_panics() {
        let _ = sdiv128(i128::MIN, -1);
    }

    #[test]
    #[should_panic(expected = "integer overflow")]
    fn sdivrem_overflow_panics() {
        let _ = sdivrem128(i128::MIN, -1);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn udivrem128_by_zero_panics() {
        let _ = udivrem128(42, 0);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn udiv_by_zero_panics() {
        let _ = udiv128(42, 0);
    }

    // #[test]
    // #[should_panic(expected = "remainder by zero")]
    // fn urem_by_zero_panics() {
    //     let _ = urem128(42, 0);
    // }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn urem_by_zero_panics() {
        let _ = urem128(42, 0);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn div_by_zero_panics() {
        let _ = sdiv128(42, 0);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn rem_by_zero_panics() {
        let _ = srem128(42, 0);
    }

    // #[test]
    // #[should_panic(expected = "remainder by zero")]
    // fn rem_by_zero_panics() {
    //     let _ = srem128(42, 0);
    // }
}
