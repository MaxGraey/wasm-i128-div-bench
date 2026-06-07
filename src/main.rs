#![feature(funnel_shifts)]

use criterion::Criterion;

#[allow(dead_code)]
mod divrem_by_recip;

fn main() {
    let mut bench = Criterion::default().configure_from_args();

    bench.final_summary();
}
