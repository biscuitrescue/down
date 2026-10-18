//! IPFS client behaviour against a scripted fake Kubo API (test boundary only): CID verification,
//! bounded retries, timeouts and integrity checks on reads.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Router};
use oilschain::{
    cid::Cid,
    config::IpfsConfig,
    ipfs::{IpfsClient, IpfsError},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Clone)]
struct Fake {
    calls: Arc<AtomicUsize>,
    /// Number of initial calls answered with 503.
    fail_first: usize,
    /// CID to answer block/put with (None = the correct one for "hello").
    put_key: Option<String>,
    /// Bytes to answer block/get with.
    get_body: Vec<u8>,
    delay: Duration,
}

async fn put(State(f): State<Fake>) -> impl IntoResponse {
    let n = f.calls.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(f.delay).await;
    if n < f.fail_first {
        return (StatusCode::SERVICE_UNAVAILABLE, "busy".to_string()).into_response();
    }
    let key = f
        .put_key
        .clone()
        .unwrap_or_else(|| Cid::raw_sha256(b"hello").to_string());
    axum::Json(serde_json::json!({"Key": key, "Size": 5})).into_response()
}

async fn get(State(f): State<Fake>) -> impl IntoResponse {
    f.calls.fetch_add(1, Ordering::SeqCst);
    f.get_body.clone()
}

async fn serve(f: Fake) -> String {
    let app = Router::new()
        .route("/api/v0/block/put", post(put))
        .route("/api/v0/block/get", post(get))
        .with_state(f);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

fn fake() -> Fake {
    Fake {
        calls: Arc::new(AtomicUsize::new(0)),
        fail_first: 0,
        put_key: None,
        get_body: b"hello".to_vec(),
        delay: Duration::ZERO,
    }
}

fn client(url: String, timeout: Duration, retries: u32) -> IpfsClient {
    IpfsClient::new(IpfsConfig {
        api_url: url,
        gateway_url: "http://gw/ipfs/".into(),
        auth_header: None,
        timeout,
        max_retries: retries,
    })
    .unwrap()
}

#[tokio::test]
async fn put_returns_locally_computed_cid() {
    let f = fake();
    let c = client(serve(f.clone()).await, Duration::from_secs(2), 3);
    let cid = c.put_block(b"hello").await.unwrap();
    assert_eq!(cid, Cid::raw_sha256(b"hello"));
    assert_eq!(
        c.gateway_url(&cid.to_string()),
        format!("http://gw/ipfs/{cid}")
    );
}

#[tokio::test]
async fn put_rejects_a_node_that_returns_another_cid() {
    let mut f = fake();
    f.put_key = Some(Cid::raw_sha256(b"something else").to_string());
    let c = client(serve(f).await, Duration::from_secs(2), 1);
    assert!(matches!(
        c.put_block(b"hello").await,
        Err(IpfsError::CidMismatch { .. })
    ));
}

#[tokio::test]
async fn transient_errors_are_retried_a_bounded_number_of_times() {
    let mut f = fake();
    f.fail_first = 2;
    let url = serve(f.clone()).await;
    let ok = client(url.clone(), Duration::from_secs(2), 3)
        .put_block(b"hello")
        .await;
    assert!(ok.is_ok(), "{ok:?}");
    assert_eq!(f.calls.load(Ordering::SeqCst), 3);

    let mut g = fake();
    g.fail_first = 100;
    let c = client(serve(g.clone()).await, Duration::from_secs(2), 2);
    assert!(matches!(
        c.put_block(b"hello").await,
        Err(IpfsError::Unavailable(_))
    ));
    assert_eq!(
        g.calls.load(Ordering::SeqCst),
        2,
        "gives up after max_retries"
    );
}

#[tokio::test]
async fn slow_node_times_out() {
    let mut f = fake();
    f.delay = Duration::from_millis(1500);
    let c = client(serve(f).await, Duration::from_millis(300), 1);
    assert!(matches!(
        c.put_block(b"hello").await,
        Err(IpfsError::Unavailable(_))
    ));
}

#[tokio::test]
async fn get_verifies_content_against_cid() {
    let mut f = fake();
    f.get_body = b"tampered".to_vec();
    let c = client(serve(f).await, Duration::from_secs(2), 1);
    let cid = Cid::raw_sha256(b"hello");
    assert!(matches!(
        c.get_block(&cid).await,
        Err(IpfsError::IntegrityFailure { .. })
    ));

    let good = client(serve(fake()).await, Duration::from_secs(2), 1);
    assert_eq!(good.get_block(&cid).await.unwrap(), b"hello");
}

#[tokio::test]
async fn oversized_payloads_are_refused_before_upload() {
    let f = fake();
    let c = client(serve(f.clone()).await, Duration::from_secs(2), 1);
    let big = vec![b'x'; oilschain::ipfs::MAX_BLOCK_SIZE + 1];
    assert!(matches!(
        c.put_block(&big).await,
        Err(IpfsError::TooLarge(_))
    ));
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unreachable_node_is_reported_as_unavailable() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let c = client(
        format!("http://127.0.0.1:{port}"),
        Duration::from_secs(1),
        2,
    );
    assert!(matches!(
        c.put_block(b"hello").await,
        Err(IpfsError::Unavailable(_))
    ));
}

// ---------------- Pinning Service API client (spec: ipfs.github.io/pinning-services-api-spec) ----------------

#[tokio::test]
async fn pinning_service_add_then_poll_until_pinned() {
    use axum::{extract::Path, http::HeaderMap, routing::get, Json};
    use oilschain::{config::PinningServiceConfig, pinning::PinningServiceClient};
    let polls = Arc::new(AtomicUsize::new(0));
    let p2 = polls.clone();
    let app = Router::new()
        .route(
            "/pins",
            post(
                |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer secret-token");
                    assert!(body["cid"].as_str().unwrap().starts_with("bafkrei"));
                    (
                        StatusCode::ACCEPTED,
                        Json(serde_json::json!({"requestid": "r1", "status": "queued"})),
                    )
                },
            ),
        )
        .route(
            "/pins/{id}",
            get(move |Path(id): Path<String>| {
                let p = p2.clone();
                async move {
                    let n = p.fetch_add(1, Ordering::SeqCst);
                    let status = if n == 0 { "pinning" } else { "pinned" };
                    Json(serde_json::json!({"requestid": id, "status": status}))
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let c = PinningServiceClient::new(PinningServiceConfig {
        url,
        token: "secret-token".into(),
        timeout: Duration::from_secs(2),
    })
    .unwrap();
    let cid = Cid::raw_sha256(b"hello").to_string();
    let added = c.add(&cid, "oilschain-test").await.unwrap();
    assert_eq!(
        (added.requestid.as_str(), added.status.as_str()),
        ("r1", "queued")
    );
    assert_eq!(c.status("r1").await.unwrap().status, "pinning");
    assert_eq!(c.status("r1").await.unwrap().status, "pinned");
}

#[tokio::test]
async fn pinning_service_auth_failure_is_an_api_error() {
    use axum::{http::StatusCode as S, routing::post};
    use oilschain::{config::PinningServiceConfig, pinning::PinningServiceClient};
    let app = Router::new().route("/pins", post(|| async { (S::UNAUTHORIZED, "bad token") }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let c = PinningServiceClient::new(PinningServiceConfig {
        url,
        token: "x".into(),
        timeout: Duration::from_secs(2),
    })
    .unwrap();
    assert!(matches!(
        c.add("bafkreix", "n").await,
        Err(IpfsError::Api { status: 401, .. })
    ));
}
