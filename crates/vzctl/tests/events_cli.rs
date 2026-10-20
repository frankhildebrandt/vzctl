use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn event_stream_preserves_fragmented_ndjson_and_additive_types() {
    let path = std::path::PathBuf::from("/private/tmp").join(format!(
        "vze-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    let listener = UnixListener::bind(path.join("vz.sock")).unwrap();
    let first = json!({"v": 1, "ts": "2026-10-20T12:00:00Z", "type": "vm.state", "data": {"vm_id": "demo/vm", "state": "running"}});
    let second = json!({"v": 1, "ts": "2026-10-20T12:00:01Z", "type": "future.additive", "data": {"new": true}, "optional": 42});
    let expected = vec![first.clone(), second.clone()];
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "events.subscribe");
        assert_eq!(request["params"]["filter"], "vm.*,future.*");
        writeln!(stream, "{}", json!({"result": {"ok": true, "v": 1}})).unwrap();
        let event = format!("{first}\n");
        stream.write_all(&event.as_bytes()[..10]).unwrap();
        // Deliberately cross the client's 250ms read timeout inside a frame.
        thread::sleep(Duration::from_millis(400));
        stream.write_all(&event.as_bytes()[10..]).unwrap();
        writeln!(stream, "{second}").unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_vzctl"))
        .args(["events", "subscribe", "--filter", "vm.*,future.*"])
        .env("VZCTL_STATE_DIR", &path)
        .output()
        .unwrap();
    server.join().unwrap();
    fs::remove_dir_all(path).unwrap();
    assert_eq!(output.status.code(), Some(10)); // EOF is an interrupted stream.
    let actual: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(actual, expected);
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("closed the event stream"));
}
