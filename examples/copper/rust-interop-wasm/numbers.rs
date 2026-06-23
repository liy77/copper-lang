// Real Rust number-crunching, executed by Alloy via embedded wasm.
// Prototype ABI: i64 scalars, exported as `#[no_mangle] pub extern "C"`.

#[no_mangle]
pub extern "C" fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.abs()
}

#[no_mangle]
pub extern "C" fn is_prime(n: i64) -> i64 {
    if n < 2 {
        return 0;
    }
    let mut i: i64 = 2;
    while i * i <= n {
        if n % i == 0 {
            return 0;
        }
        i += 1;
    }
    1
}

#[no_mangle]
pub extern "C" fn pow_mod(mut base: i64, mut exp: i64, modulus: i64) -> i64 {
    if modulus == 1 {
        return 0;
    }
    let mut result: i64 = 1;
    base %= modulus;
    while exp > 0 {
        if exp % 2 == 1 {
            result = result * base % modulus;
        }
        exp /= 2;
        base = base * base % modulus;
    }
    result
}
