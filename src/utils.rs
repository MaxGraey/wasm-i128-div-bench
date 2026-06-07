/*
 * Shared seed and benchmark input setup for both division backends.
 */

use num_traits::AsPrimitive;
use rand::{RngExt, SeedableRng, rngs::SmallRng};

/* Fixed seed (golden-ratio) for random tests and benches. */
pub(crate) const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/* Iteration count for the loop-invariant-divisor benchmark. */
pub(crate) const BENCH_ITERS: usize = 1000;

/* Random dividend and a divisor in [2^64, 2^127) - top bit clear and nonzero,
 * so the reciprocal path runs its 3-by-2 kernel. Both backends draw the same
 * seeded operands. The concrete type is chosen at the call site, u128 as_() to T
 * is the identity for u128 and a bit reinterpret for i128. */
pub(crate) fn rand_operands<T>() -> (T, T)
where
    T: Copy + 'static,
    u128: AsPrimitive<T>,
{
    let mut rng = SmallRng::seed_from_u64(SEED);
    let x: u128 = rng.random();
    let y = (rng.random::<u128>() >> 1) | (1u128 << 64);
    (x.as_(), y.as_())
}
