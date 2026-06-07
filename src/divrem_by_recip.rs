/*
 * Reciprocal-based 128-bit division.
 *
 * Ported from intx (https://github.com/chfast/intx, Apache-2.0). Based on
 * Algorithm 2 of Moeller & Granlund, "Improved division by invariant integers".
 *
 * The whole point of this path is to replace the compiler-rt 128-bit division
 * libcall (__udivti3 / __umodti3 on wasm) with a multiply-heavy sequence that
 * stays on u64 limbs. Everything below operates on explicit (hi, lo) limbs with
 * manual carry / borrow, so no native u128 arithmetic is emitted - native u128
 * appears only at the public API boundary, where the split / join lower to plain
 * register moves.
 */


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
fn split128(x: u128) -> (u64, u64) {
    ((x >> 64) as u64, x as u64)
}

#[inline(always)]
fn join128(v: (u64, u64)) -> u128 {
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

#[inline(always)]
fn shr128(v: (u64, u64), sh: u32) -> (u64, u64) {
    (v.0 >> sh, (v.1 >> sh) | (v.0 << (64 - sh)))
}

#[inline]
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
fn reciprocal_2by1(d: u64) -> u64 {
    debug_assert!(d & 0x8000_0000_0000_0000 != 0, "d must be normalized");

    let d9 = d >> 55;
    let v0 = RECIPROCAL_TABLE[(d9 - 256) as usize] as u32;

    let d40  = (d >> 24) + 1;
    let v0v0 = v0.wrapping_mul(v0);
    let term = ((v0v0 as u64).wrapping_mul(d40) >> 40) as u32;
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

/* Core 128-by-128 division on limbs, returns (quotient, remainder).
 * Caller guarantees y != 0. */
#[inline]
fn core_udivrem128(x: (u64, u64), y: (u64, u64)) -> ((u64, u64), (u64, u64)) {
    // fast-path
    if y.0 == 0 {
        // Divisor fits in 64 bits, normalize and run two 2-by-1 steps.
        let lsh = y.1.leading_zeros();
        let rsh = (64 - lsh) & 63;
        let rsh_mask = (if lsh == 0 {
            1u64
        } else {
            0u64
        }).wrapping_sub(1);

        let yn    = y.1 << lsh;
        let xn_lo = x.1 << lsh;
        let xn_hi = (x.0 << lsh) | ((x.1 >> rsh) & rsh_mask);
        let xn_ex = (x.0 >> rsh) & rsh_mask;

        let v        = reciprocal_2by1(yn);
        let (q1, r1) = udivrem_2by1((xn_ex, xn_hi), yn, v);
        let (q0, r0) = udivrem_2by1((r1, xn_lo), yn, v);

        return ((q1, q0), (0, r0 >> lsh));
    }

    // fast-path
    if y.0 > x.0 {
        return ((0, 0), x);
    }

    // fast-path
    let lsh = y.0.leading_zeros();
    if lsh == 0 {
        // Divisor already uses the top limb fully. Quotient is 0 or 1.
        let q = (y.0 < x.0) || (y.1 <= x.1);
        let rem = if q {
            sub128(x, y)
        } else {
            x
        };
        return ((0, q as u64), rem);
    }

    let rsh = 64 - lsh;

    let yn_lo = y.1 << lsh;
    let yn_hi = (y.0 << lsh) | (y.1 >> rsh);
    let xn_lo = x.1 << lsh;
    let xn_hi = (x.0 << lsh) | (x.1 >> rsh);
    let xn_ex = x.0 >> rsh;

    let d = (yn_hi, yn_lo);
    let v = reciprocal_3by2(d);
    let (q, r) = udivrem_3by2(xn_ex, xn_hi, xn_lo, d, v);

    ((0, q), shr128(r, lsh))
}

/* Division by zero is fatal. Messages mirror the wasm integer trap text V8
 * reports for i64.div_u / i64.rem_u by zero ("divide by zero" / "remainder by
 * zero"), so the failure reads identically to the native u64/u64 path. */
#[cold]
#[inline(never)]
fn divide_by_zero() -> ! {
    panic!("divide by zero")
}

#[cold]
#[inline(never)]
fn remainder_by_zero() -> ! {
    panic!("remainder by zero")
}

/* Signed core, truncating toward zero like the native i128 operators. Assumes
 * y != 0. The i128::MIN / -1 overflow yields the wrapping result (MIN, 0). */
#[inline]
fn core_sdivrem128(x: i128, y: i128) -> (i128, i128) {
    let (q, r) = core_udivrem128(
        split128(x.unsigned_abs()),
        split128(y.unsigned_abs()),
    );

    let q = join128(q) as i128;
    let r = join128(r) as i128;

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


/* Unsigned 128-bit divide-and-remainder. Traps on division by zero. */
pub fn udivrem128(x: u128, y: u128) -> (u128, u128) {
    if y == 0 {
        divide_by_zero();
    }

    let (q, r) = core_udivrem128(split128(x), split128(y));
    (join128(q), join128(r))
}

pub fn udiv128(x: u128, y: u128) -> u128 {
    if y == 0 {
        divide_by_zero();
    }

    join128(core_udivrem128(split128(x), split128(y)).0)
}

pub fn urem128(x: u128, y: u128) -> u128 {
    if y == 0 {
        remainder_by_zero();
    }

    join128(core_udivrem128(split128(x), split128(y)).1)
}

pub fn sdivrem128(x: i128, y: i128) -> (i128, i128) {
    if y == 0 {
        divide_by_zero();
    }

    core_sdivrem128(x, y)
}

pub fn sdiv128(x: i128, y: i128) -> i128 {
    if y == 0 {
        divide_by_zero();
    }

    core_sdivrem128(x, y).0
}

pub fn srem128(x: i128, y: i128) -> i128 {
    if y == 0 {
        remainder_by_zero();
    }

    core_sdivrem128(x, y).1
}

#[cfg(test)]
mod tests {
    use super::*;

    /* Deterministic xorshift so tests stay reproducible without a dep. */
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn u128(&mut self) -> u128 {
            ((self.next() as u128) << 64) | self.next() as u128
        }
    }

    #[test]
    fn umul_matches_native() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for _ in 0..100_000 {
            let a = rng.next();
            let b = rng.next();
            let got = mul128(a, b);
            let want = (a as u128) * (b as u128);
            assert_eq!(((got.0 as u128) << 64) | got.1 as u128, want, "{a} * {b}");
        }
    }

    #[test]
    fn unsigned_random() {
        let mut rng = Rng(0xdead_beef_cafe_babe);
        for _ in 0..500_000 {
            let x = rng.u128();
            let mut y = rng.u128();
            // Bias toward small divisors too, exercising the 2-by-1 path.
            if rng.next() & 1 == 0 {
                y = (rng.next() >> (rng.next() % 64)) as u128;
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
            0xffff_ffff_ffff_ffff_0000_0000_0000_0000,
            0x0000_0000_0000_0001_0000_0000_0000_0000,
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
        let mut rng = Rng(0x0bad_f00d_1337_c0de);
        for _ in 0..500_000 {
            let x = rng.u128() as i128;
            let y = rng.u128() as i128;
            if y == 0 {
                continue;
            }
            // Skip native overflow case, documented to differ.
            if x == i128::MIN && y == -1 {
                assert_eq!(sdiv128(x, y), i128::MIN);
                assert_eq!(srem128(x, y), 0);
                continue;
            }
            assert_eq!(sdivrem128(x, y), (x / y, x % y), "x={x} y={y}");
        }
    }

    #[test]
    fn signed_edges() {
        let vals = [0i128, 1, -1, 2, -2, i128::MAX, i128::MIN, i128::MIN + 1, i64::MAX as i128, i64::MIN as i128];
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
    #[should_panic(expected = "divide by zero")]
    fn udivrem128_by_zero_panics() {
        let _ = udivrem128(42, 0);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn udiv_by_zero_panics() {
        let _ = udiv128(42, 0);
    }

    #[test]
    #[should_panic(expected = "remainder by zero")]
    fn urem_by_zero_panics() {
        let _ = urem128(42, 0);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn div_by_zero_panics() {
        let _ = sdiv128(42, 0);
    }

    #[test]
    #[should_panic(expected = "remainder by zero")]
    fn rem_by_zero_panics() {
        let _ = srem128(42, 0);
    }
}
