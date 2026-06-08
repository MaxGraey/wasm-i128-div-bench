/*
 * Shared seed and benchmark input setup for both division backends.
 */

use num_traits::AsPrimitive;
use rand::{RngExt, SeedableRng, rngs::SmallRng};

/* Fixed seed (golden-ratio) for random tests and benches. */
pub(crate) const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/* Iteration count for the loop-invariant-divisor benchmark. */
pub(crate) const BENCH_ITERS: usize = 1000;

/* Random dividend and a random divisor, rejecting only 0 (the divide-by-zero
 * trap) and the i128::MIN bit pattern. The divisor is no longer pinned to the
 * 3-by-2 kernel - it now exercises whichever path it selects (2-by-1, the Y > X
 * and full-top-limb shortcuts, or the general 3-by-2). Both backends draw the
 * same seeded operands. The concrete type is chosen at the call site, u128 as_()
 * to T is the identity for u128 and a bit reinterpret for i128. */
pub(crate) fn rand_operands<T>() -> (T, T)
where
    T: Copy + 'static,
    u128: AsPrimitive<T>,
{
    let mut rng = SmallRng::seed_from_u64(SEED);
    let x: u128 = rng.random();

    let mut y: u128 = rng.random();
    while y == 0 || y == (1u128 << 127) {
        y = rng.random();
    }

    (x.as_(), y.as_())
}
