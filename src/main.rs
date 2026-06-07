#![feature(funnel_shifts)]

use criterion::Criterion;

mod utils;

mod divrem_by_recip;
mod divrem_builtin;

fn bench_by_recip(c: &mut Criterion) {
    divrem_by_recip::bench_udivrem128(c);
    divrem_by_recip::bench_udiv128(c);
    divrem_by_recip::bench_urem128(c);

    divrem_by_recip::bench_sdivrem128(c);
    divrem_by_recip::bench_sdiv128(c);
    divrem_by_recip::bench_srem128(c);

    divrem_by_recip::bench_divrem_with_loop_invariant_divisor(c);
}

fn bench_builtin(c: &mut Criterion) {
    divrem_builtin::bench_udivrem128(c);
    divrem_builtin::bench_udiv128(c);
    divrem_builtin::bench_urem128(c);

    divrem_builtin::bench_sdivrem128(c);
    divrem_builtin::bench_sdiv128(c);
    divrem_builtin::bench_srem128(c);

    divrem_builtin::bench_divrem_with_loop_invariant_divisor(c);
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_by_recip(&mut c);
    bench_builtin(&mut c);
    c.final_summary();
}
