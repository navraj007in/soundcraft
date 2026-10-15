use serde_json::{Value, json};
use soundcraft_automation::{Backend, Remote};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn disconnect_after_execution_replays_mutation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let applications = Arc::new(AtomicUsize::new(0));
    let counter = applications.clone();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut text = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut text).unwrap();
            let msg: Value = serde_json::from_str(&text).unwrap();
            counter.fetch_add(1, Ordering::SeqCst); // the mutation committed
            requests.push(msg.clone());
            if attempt == 1 {
                writeln!(stream, "{}", json!({"id":msg["id"],"ok":true,"result":{}})).unwrap();
            } // first connection drops after commit, before reply
        }
        requests
    });
    let mut client = Remote::new(&addr);
    client.call("engine.execute", json!({"command":"track.new","params":{"count":1}})).unwrap();
    let requests = server.join().unwrap();
    println!(
        "One successful Remote.call committed {} times (request ids {}, {})",
        applications.load(Ordering::SeqCst),
        requests[0]["id"],
        requests[1]["id"]
    );
    assert_eq!(applications.load(Ordering::SeqCst), 2);
}
