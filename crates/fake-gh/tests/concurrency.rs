//! Parallel invocations each get exactly one log line, with unique `seq` values.

mod common;

use common::{TempDir, gh, install_gh, read_log, write};

#[test]
fn twenty_parallel_invocations_log_twenty_lines() {
    let tmp = TempDir::new("concurrency");
    let bin = install_gh(tmp.path());
    let set = tmp.path().join("base");
    write(
        &set,
        "user/GET.json",
        r#"{"version":1,"request":{"method":"GET","path":"/user"},"response":{"status":200,"body":{"login":"parity-bot"}}}"#,
    );
    let log = tmp.path().join("fake-gh.log");

    let children: Vec<_> = (0..20)
        .map(|i| {
            // Mix hits and misses, as an app's startup burst would.
            let path = if i % 2 == 0 {
                "/user".to_owned()
            } else {
                format!("/miss/{i}")
            };
            gh(&bin, &[&set], Some(&log))
                .args(["api", "-i", &path])
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        child.wait().unwrap();
    }

    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 20);
    let entries = read_log(&log);
    let mut seqs: Vec<u64> = entries.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=20).collect::<Vec<_>>());
    assert_eq!(entries.iter().filter(|e| e["miss"] == true).count(), 10);
}
