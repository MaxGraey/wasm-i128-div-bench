use criterion::Criterion;

fn main() {
    let mut bench = Criterion::default().configure_from_args();

    bench.final_summary();
}
