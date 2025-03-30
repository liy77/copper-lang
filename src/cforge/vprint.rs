#[macro_export]
macro_rules! vprint {
    ($($arg:tt)*) => {
        if std::env::var_os("CFORGE_VERBOSE").map(|s| s.to_string_lossy() == "1").unwrap_or(false) {
            println!($($arg)*);
        }
    };
}