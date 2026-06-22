use std::io::Write;
use std::process::Command;

/// Roda o binário `alloy run` sobre um fonte temporário e captura stdout.
fn alloy_run(src: &str) -> (String, bool) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "alloy_test_{}_{}.crs",
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed)
    ));
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
    }
    let exe = env!("CARGO_BIN_EXE_alloy");
    let out = Command::new(exe).arg("run").arg(&path).output().unwrap();
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

#[test]
fn hello_world_prints() {
    let (stdout, ok) = alloy_run("println(\"olá, alloy\")");
    assert!(ok, "alloy run falhou");
    assert_eq!(stdout.trim_end(), "olá, alloy");
}

#[test]
fn loop_and_function() {
    let src = "func int dobro(n: int) { return n * 2 }\nfor i in 1..4 { println(dobro(i)) }";
    let (stdout, ok) = alloy_run(src);
    assert!(ok);
    assert_eq!(stdout, "2\n4\n6\n");
}
