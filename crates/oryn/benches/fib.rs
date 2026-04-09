use criterion::{Criterion, criterion_group, criterion_main};

const FIB_SRC: &str = "\
fn fib(n: i32) -> i32 {
    if n <= 1 { rn n }
    rn fib(n - 1) + fib(n - 2)
}

print(fib(28))
";

fn fib_28(c: &mut Criterion) {
    // Compile once, run many times. This benchmarks the VM, not the compiler.
    let chunk = oryn::Chunk::compile(FIB_SRC).expect("compile error");
    let mut vm = oryn::VM::new();
    let mut sink = std::io::sink();

    c.bench_function("fib(28)", |b| {
        b.iter(|| {
            vm.run_with_writer(&chunk, &mut sink).unwrap();
        });
    });
}

fn compile_fib(c: &mut Criterion) {
    // Benchmark the compiler separately.
    c.bench_function("compile fib", |b| {
        b.iter(|| {
            oryn::Chunk::compile(FIB_SRC).unwrap();
        });
    });
}

criterion_group!(benches, fib_28, compile_fib);
criterion_main!(benches);
