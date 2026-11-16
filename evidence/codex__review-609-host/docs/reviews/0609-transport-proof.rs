// Included as a review-only child module of the real tunnel crate.
use super::*;
use agora_hub::ca::Ca;
use std::sync::atomic::{AtomicUsize, Ordering};

const ACK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

struct Recorder {
    ca: Ca,
    addr: SocketAddr,
    count: Arc<AtomicUsize>,
    length: Arc<AtomicUsize>,
    received: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn recorder() -> Recorder {
    let ca = Ca::generate("review fixture").unwrap();
    let (cert, key) = ca.issue_server(&["localhost".into()]).unwrap();
    let tls = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let length = Arc::new(AtomicUsize::new(0));
    let (tx, received) = tokio::sync::mpsc::unbounded_channel();
    let c = count.clone();
    let n = length.clone();
    let task = tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            c.fetch_add(1, Ordering::SeqCst);
            let expected = n.load(Ordering::SeqCst);
            let tls = tls.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut s = tls.accept(s).await.unwrap();
                let mut bytes = vec![0; expected];
                tokio::time::timeout(Duration::from_secs(3), s.read_exact(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
                tx.send(bytes).unwrap();
                s.write_all(ACK).await.unwrap();
                s.shutdown().await.unwrap();
            });
        }
    });
    Recorder {
        ca,
        addr,
        count,
        length,
        received,
        task,
    }
}

async fn exchange(local: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut client = TcpStream::connect(local).await.unwrap();
    let _ = client.write_all(bytes).await;
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(4), client.read_to_end(&mut reply))
        .await
        .unwrap();
    reply
}

#[tokio::test]
async fn review609_rejections_never_dial_and_valid_control_still_works() {
    let mut hub = recorder().await;
    let tunnel = open(&hub.addr.to_string(), hub.ca.cert_pem(), 0)
        .await
        .unwrap();
    let p = tunnel.local.port();
    let good = format!("127.0.0.1:{p}");
    let requests = [
        format!("GET / HTTP/1.1\r\nHost: rebind.example:{p}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nHost: localhost.:{p}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.2:{p}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nHost: [::1]:{p}\r\n\r\n"),
        format!(
            "GET / HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
            p.wrapping_add(1)
        ),
        "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n".into(),
        "GET / HTTP/1.1\r\n\r\n".into(),
        format!("GET / HTTP/1.1\r\nHost: {good}\r\nHOST: {good}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nHost: rebind.example:{p}\r\nHost: {good}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nHost : {good}\r\n\r\n"),
        format!("GET / HTTP/1.1\r\nX: a\r\n Host: {good}\r\n\r\n"),
        format!("GET http://rebind.example:{p}/ HTTP/1.1\r\nHost: {good}\r\n\r\n"),
        format!("CONNECT rebind.example:443 HTTP/1.1\r\nHost: {good}\r\n\r\n"),
    ];
    for (i, request) in requests.iter().enumerate() {
        assert_eq!(
            exchange(tunnel.local, request.as_bytes()).await,
            MISDIRECTED,
            "case {i}"
        );
    }
    let mut bad_utf8 = format!("GET / HTTP/1.1\r\nHost: {good}").into_bytes();
    bad_utf8.extend_from_slice(b"\xff\r\n\r\n");
    assert_eq!(exchange(tunnel.local, &bad_utf8).await, MISDIRECTED);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        hub.count.load(Ordering::SeqCst),
        0,
        "14 rejected heads never reach TCP/TLS"
    );
    let valid = format!("GET /control HTTP/1.1\r\nHost: {good}\r\n\r\n");
    hub.length.store(valid.len(), Ordering::SeqCst);
    assert_eq!(exchange(tunnel.local, valid.as_bytes()).await, ACK);
    assert_eq!(hub.received.recv().await.unwrap(), valid.as_bytes());
    assert_eq!(
        hub.count.load(Ordering::SeqCst),
        1,
        "positive control proves listener is live"
    );
    println!(
        "14 rejected heads: 421, hub TCP count 0; valid TLS control: ACK and exact bytes, count 1"
    );
}

#[tokio::test]
async fn review609_body_limit_and_documented_pipeline_are_byte_exact() {
    let mut hub = recorder().await;
    let tunnel = open(&hub.addr.to_string(), hub.ca.cert_pem(), 0)
        .await
        .unwrap();
    let host = tunnel.local;
    let mut binary =
        format!("POST /body HTTP/1.1\r\nHost: {host}\r\nContent-Length: 6\r\n\r\n").into_bytes();
    binary.extend_from_slice(b"\xff\x00\r\n\r\n");
    let prefix = format!("GET /limit HTTP/1.1\r\nHost: {host}\r\nX-Pad: ");
    let exact = format!(
        "{prefix}{}\r\n\r\n",
        "a".repeat(MAX_HEAD - prefix.len() - 4)
    )
    .into_bytes();
    assert_eq!(exact.len(), MAX_HEAD);
    let pipeline = format!("GET /first HTTP/1.1\r\nHost: {host}\r\n\r\nGET /second HTTP/1.1\r\nHost: rebind.example:{}\r\n\r\n", host.port()).into_bytes();
    for request in [&binary, &exact, &pipeline] {
        hub.length.store(request.len(), Ordering::SeqCst);
        assert_eq!(exchange(host, request).await, ACK);
        assert_eq!(&hub.received.recv().await.unwrap(), request);
    }
    let mut oversized = exact.clone();
    oversized.insert(oversized.len() - 4, b'a');
    assert_eq!(oversized.len(), MAX_HEAD + 1);
    assert_eq!(exchange(host, &oversized).await, b"");
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        hub.count.load(Ordering::SeqCst),
        3,
        "oversized head never dials"
    );
    println!("Binary body, exact 16KiB head and local-program pipeline preserved; 16KiB+1 head closed before dialing");
}

#[tokio::test]
async fn review609_partial_head_expires_without_blocking_other_connections() {
    let mut hub = recorder().await;
    let tunnel = open(&hub.addr.to_string(), hub.ca.cert_pem(), 0)
        .await
        .unwrap();
    let mut slow = TcpStream::connect(tunnel.local).await.unwrap();
    slow.write_all(b"GET / HTTP/1.1\r\nHost: ").await.unwrap();
    let began = tokio::time::Instant::now();
    let request = format!("GET /parallel HTTP/1.1\r\nHost: {}\r\n\r\n", tunnel.local);
    hub.length.store(request.len(), Ordering::SeqCst);
    assert_eq!(exchange(tunnel.local, request.as_bytes()).await, ACK);
    assert!(began.elapsed() < Duration::from_secs(3));
    assert_eq!(hub.received.recv().await.unwrap(), request.as_bytes());
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(12), slow.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(reply.is_empty());
    assert!(began.elapsed() >= Duration::from_secs(9));
    assert_eq!(
        hub.count.load(Ordering::SeqCst),
        1,
        "only valid parallel request dials"
    );
    println!("Valid parallel request finished <3s; incomplete head expired near 10s with no response/no extra hub connection");
}
