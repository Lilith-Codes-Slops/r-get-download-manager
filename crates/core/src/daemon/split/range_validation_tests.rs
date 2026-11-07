use crate::daemon::range::{validate, validate_bounded, validate_probe, ResumeInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_request(stream: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
        if !matches!(stream.read(&mut byte).await, Ok(1)) { break; }
        request.push(byte[0]);
    }
}

async fn serve(response: &'static str) -> (reqwest::Response, ResumeInfo) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        let _ = stream.write_all(response.as_bytes()).await;
    });
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    task.await.unwrap();
    let info = ResumeInfo { etag: "\"v1\"".into(), total: 8, url: reqwest::Url::parse(&url).unwrap() };
    (resp, info)
}

#[tokio::test]
async fn accepts_only_exact_bounded_range_and_valid_offsets() {
    let (resp, info) = serve("HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/8\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nabcd").await;
    assert!(validate_bounded(&resp, &info, 0, 4).is_ok());
    assert!(validate_bounded(&resp, &info, 0, 0).is_err());
    assert!(validate_bounded(&resp, &info, 4, 9).is_err());
    assert!(validate_bounded(&resp, &info, 1, 4).is_err());
    assert!(validate(&resp, &info, 9).is_err());
    assert!(validate(&resp, &info, 8).is_err());
}

#[tokio::test]
async fn rejects_redirect_to_another_resolved_url() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        read_request(&mut first).await;
        first.write_all(b"HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        let (mut second, _) = listener.accept().await.unwrap();
        read_request(&mut second).await;
        second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/8\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nabcd").await.unwrap();
    });
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    let info = ResumeInfo { etag: "\"v1\"".into(), total: 8, url: reqwest::Url::parse(&url).unwrap() };
    assert!(validate_bounded(&resp, &info, 0, 4).is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn probe_requires_valid_strong_validator_and_nonempty_resource() {
    let (resp, mut info) = serve("HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/8\r\nETag: \"v1\"\r\nConnection: close\r\n\r\na").await;
    assert!(validate_probe(&resp, &info).is_ok());
    info.etag = "W/\"v1\"".into();
    assert!(validate_probe(&resp, &info).is_err());
    info.etag = "\"v1\"".into();
    info.total = 0;
    assert!(validate_probe(&resp, &info).is_err());
}
