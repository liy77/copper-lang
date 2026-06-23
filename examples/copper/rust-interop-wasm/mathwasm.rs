// Rust executed by Alloy via embedded wasm (compiled to wasm32-unknown-unknown,
// cached, run by wasmi). Prototype: i64 scalars exported as `extern "C"`.
#[no_mangle]
pub extern "C" fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[no_mangle]
pub extern "C" fn fib(n: i64) -> i64 {
    if n < 2 { return n; }
    let mut a: i64 = 0;
    let mut b: i64 = 1;
    let mut i: i64 = 2;
    while i <= n {
        let c = a + b;
        a = b;
        b = c;
        i += 1;
    }
    b
}
