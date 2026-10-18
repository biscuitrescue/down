//! End-to-end test against a real local EVM (anvil) and a real IPFS node (Kubo, offline mode).
//!
//! It deploys the contracts from the exported Foundry artifacts, starts the HTTP API in-process and drives
//! every stage through HTTP. Ignored by default because it needs the binaries; run it with
//! `scripts/e2e.sh` or `ANVIL_BIN=... IPFS_BIN=... cargo test --test e2e -- --ignored --nocapture`.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use oilschain::{
    chain::{BatchProvenanceNFT, ChainAction, ChainClient, FarmerCredential, OilseedValueChain},
    config::{
        AdvisoryConfig, ChainConfig, Config, Environment, IpfsBackupConfig, IpfsConfig,
        SecretsConfig,
    },
    db::{self, OpStatus},
    ipfs::IpfsClient,
    ops, reconciler, routes,
    state::AppState,
    verify,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

const ANVIL_KEY0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const FARMER_WALLET: &str = "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc"; // anvil account #5
const ADMIN_TOKEN: &str = "test-admin-token-0123456789abcdef0123456789";

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bin(env: &str, name: &str) -> PathBuf {
    std::env::var(env)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(name))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_until<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..150 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn start_anvil() -> (Proc, String) {
    let port = free_port();
    let child = Command::new(bin("ANVIL_BIN", "anvil"))
        .args(["--port", &port.to_string(), "--silent"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("anvil binary (set ANVIL_BIN)");
    let url = format!("http://127.0.0.1:{port}");
    let u = url.clone();
    wait_until("anvil", || {
        let u = u.clone();
        async move {
            let p = ProviderBuilder::new().connect_http(u.parse().unwrap());
            p.get_chain_id().await.is_ok()
        }
    })
    .await;
    (Proc(child), url)
}

async fn start_kubo(dir: &Path) -> (Proc, String) {
    let ipfs = bin("IPFS_BIN", "ipfs");
    let run = |args: &[&str]| {
        let out = Command::new(&ipfs)
            .args(args)
            .env("IPFS_PATH", dir)
            .output()
            .expect("ipfs binary (set IPFS_BIN)");
        assert!(
            out.status.success(),
            "ipfs {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "--profile=test"]);
    let api_port = free_port();
    run(&[
        "config",
        "Addresses.API",
        &format!("/ip4/127.0.0.1/tcp/{api_port}"),
    ]);
    run(&[
        "config",
        "Addresses.Gateway",
        &format!("/ip4/127.0.0.1/tcp/{}", free_port()),
    ]);
    let child = Command::new(&ipfs)
        .args(["daemon", "--offline"])
        .env("IPFS_PATH", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let url = format!("http://127.0.0.1:{api_port}");
    let u = url.clone();
    wait_until("kubo", || {
        let u = u.clone();
        async move {
            reqwest::Client::new()
                .post(format!("{u}/api/v0/version"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
        }
    })
    .await;
    (Proc(child), url)
}

async fn deploy(rpc: &str) -> (Address, Address, Address) {
    let signer: PrivateKeySigner = ANVIL_KEY0.parse().unwrap();
    let p = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(rpc.parse().unwrap());
    let reg = OilseedValueChain::deploy(p.clone()).await.unwrap();
    let prov = BatchProvenanceNFT::deploy(p.clone(), *reg.address())
        .await
        .unwrap();
    let cred = FarmerCredential::deploy(p.clone(), *reg.address())
        .await
        .unwrap();
    (*reg.address(), *prov.address(), *cred.address())
}

fn config(
    rpc: &str,
    ipfs: &str,
    contracts: (Address, Address, Address),
    db: PathBuf,
    advisory: Option<String>,
    backup: Option<&str>,
) -> Config {
    Config {
        port: 0,
        host: "127.0.0.1".into(),
        environment: Environment::Test,
        database_path: db,
        frontend_dir: None,
        cors_allowed_origins: vec![],
        chain: ChainConfig {
            rpc_url: rpc.into(),
            chain_id: 31337,
            registry: contracts.0,
            provenance_nft: contracts.1,
            credential_nft: contracts.2,
            deploy_block: 0,
            admin_private_key: ANVIL_KEY0.into(),
            participant_funding_wei: U256::from(10u64).pow(U256::from(19u64)), // 10 ETH
            confirm_timeout: Duration::from_secs(20),
            poll_interval: Duration::from_millis(100),
            drop_after: Duration::from_secs(600),
        },
        ipfs: IpfsConfig {
            api_url: ipfs.into(),
            gateway_url: "http://127.0.0.1:8080/ipfs/".into(),
            auth_header: None,
            timeout: Duration::from_secs(5),
            max_retries: 2,
        },
        ipfs_backup: IpfsBackupConfig {
            kubo: backup
                .into_iter()
                .map(|u| IpfsConfig {
                    api_url: u.into(),
                    gateway_url: "http://127.0.0.1:8080/ipfs/".into(),
                    auth_header: None,
                    timeout: Duration::from_secs(5),
                    max_retries: 1,
                })
                .collect(),
            pinning_service: None,
            max_attempts: 3,
        },
        secrets: SecretsConfig {
            admin_api_token: ADMIN_TOKEN.into(),
            key_encryption_key: [1u8; 32],
            pii_encryption_key: [2u8; 32],
        },
        reconcile_interval: Duration::from_secs(3600),
        ai_auto_reveal: true,
        advisory: AdvisoryConfig {
            base_url: advisory,
            model: "gemma3n:e2b".into(),
            languages: vec!["en".into(), "hi".into()],
            timeout: Duration::from_secs(5),
        },
        ranking_prior_strength: 4.0,
    }
}

/// Minimal Ollama-compatible stand-in (test boundary only): echoes the grounding facts it was given.
async fn fake_ollama() -> String {
    use axum::{routing::post, Json, Router};
    let app = Router::new().route(
        "/api/chat",
        post(|Json(body): Json<Value>| async move {
            let system = body["messages"][0]["content"].as_str().unwrap_or("").to_string();
            Json(json!({"message": {"role": "assistant", "content": format!("ECHO-FACTS: {}", system.lines().filter(|l| l.starts_with("Verified") || l.starts_with("Warehouse")).collect::<Vec<_>>().join(" | "))}}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        idem: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(k) = idem {
            req = req.header("Idempotency-Key", k);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
    async fn get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.call("GET", path, token, None, None).await
    }
    async fn post(&self, path: &str, token: &str, idem: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", path, Some(token), Some(idem), Some(body))
            .await
    }
}

fn ok(step: &str, (status, body): (StatusCode, Value)) -> Value {
    assert_eq!(status, StatusCode::OK, "{step}: {body:#}");
    if body.get("status").is_some_and(|s| s.is_string()) && body.get("operation_id").is_some() {
        assert_eq!(body["status"], "confirmed", "{step}: {body:#}");
    }
    body
}

async fn create_participant(api: &Api, name: &str, roles: &[&str]) -> (String, String, String) {
    let (status, body) = api
        .call(
            "POST",
            "/api/admin/participants",
            Some(ADMIN_TOKEN),
            None,
            Some(json!({"name": name, "roles": roles})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body:#}");
    for op in body["operations"].as_array().unwrap() {
        assert_eq!(op["status"], "confirmed", "{op:#}");
    }
    (
        body["participant"]["id"].as_str().unwrap().into(),
        body["api_token"].as_str().unwrap().into(),
        body["participant"]["address"].as_str().unwrap().into(),
    )
}

#[tokio::test]
#[ignore = "needs anvil and ipfs (Kubo) binaries; run scripts/e2e.sh"]
async fn full_supply_chain_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let (_anvil, rpc) = start_anvil().await;
    let (_kubo, ipfs_url) = start_kubo(&tmp.path().join("ipfs")).await;
    let (_kubo_backup, backup_url) = start_kubo(&tmp.path().join("ipfs-backup")).await;
    let contracts = deploy(&rpc).await;
    let ollama = fake_ollama().await;
    let db_path = tmp.path().join("e2e.db");

    let cfg = config(
        &rpc,
        &ipfs_url,
        contracts,
        db_path.clone(),
        Some(ollama),
        Some(&backup_url),
    );
    let state = AppState::initialize(cfg).await.expect("state");
    let app = axum::Router::new().merge(routes::configure_routes(state.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let api = Api {
        base,
        http: reqwest::Client::new(),
    };

    // ---- health & config
    let health = ok("health", api.get("/api/health", None).await);
    assert_eq!(health["status"], "ok");

    // ---- participants (each gets its own funded custodial wallet + on-chain role)
    let (fpo_id, fpo, fpo_addr) = create_participant(&api, "Shivneri FPO", &["fpo"]).await;
    let (wh_id, wh, _) = create_participant(&api, "Latur Warehouse", &["warehouse"]).await;
    let (lg_id, lg, _) = create_participant(&api, "Deccan Logistics", &["logistics"]).await;
    let (pr_id, pr, pr_addr) =
        create_participant(&api, "Sahyadri Oil Mill", &["processor", "packager"]).await;
    let (_or_id, oracle, _) = create_participant(&api, "Quality Oracle", &["ai_oracle"]).await;
    let (_rt_id, retailer, _) = create_participant(&api, "Pune Retail", &["retailer"]).await;
    let me = ok("me", api.get("/api/me", Some(&fpo)).await);
    assert_eq!(me["onchain_roles"], json!(["fpo"]));

    // ---- auth failures
    let (s, _) = api
        .call("POST", "/api/fpo/purchase", None, None, Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = api
        .call(
            "POST",
            "/api/farmer/register",
            Some("oc_bogus"),
            None,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // ---- farmer registration (encrypted on IPFS)
    let farmer_req = json!({
        "label": "SFPO member 0042",
        "region": {"state": "Maharashtra", "district": "Latur"},
        "primary_crop": "soybean",
        "wallet_address": FARMER_WALLET,
        "profile": {"name": "Asha Patil", "phone": "+91-9000000000", "village": "Ausa", "land_area_ha": 1.6, "crops": ["soybean", "groundnut"]}
    });
    let (s, _) = api
        .post("/api/farmer/register", &wh, "f-wh", farmer_req.clone())
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "warehouse cannot register farmers"
    );
    let reg = ok(
        "register farmer",
        api.post("/api/farmer/register", &fpo, "farmer-1", farmer_req.clone())
            .await,
    );
    let farmer_id = reg["result"]["farmer_id"].as_str().unwrap().to_string();
    assert!(reg["encrypted"].as_bool().unwrap());
    let cid = oilschain::cid::Cid::parse(reg["cid"].as_str().unwrap()).unwrap();
    let raw = state.ipfs_client.get_block(&cid).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&raw).contains("Asha"),
        "PII must not be readable on IPFS"
    );
    let fv = ok(
        "verify farmer",
        api.get(&format!("/api/farmers/{farmer_id}/verify"), None)
            .await,
    );
    assert_eq!(fv["status"], "verified", "{fv:#}");
    // Selective disclosure: public fields are readable and verified; private ones are encrypted and only
    // their names are reported.
    assert_eq!(fv["data"]["label"], "SFPO member 0042", "{fv:#}");
    assert_eq!(fv["data"]["crops"], json!(["soybean", "groundnut"]));
    assert!(fv["data"].get("profile").is_none() && fv["data"].get("wallet_address").is_none());
    assert_eq!(fv["private_fields"], json!(["profile", "wallet_address"]));
    assert_eq!(fv["private_verified"], true);
    let raw_text = String::from_utf8_lossy(&raw).to_string();
    assert!(
        raw_text.contains("SFPO member 0042")
            && !raw_text.contains("9000000000")
            && !raw_text.contains("Ausa")
    );

    // ---- procurement (creates batch) + idempotency
    let proc_req = json!({
        "farmer_id": farmer_id, "label": "LOT-2025-K-017", "crop": "soybean", "variety": "JS 335", "season": "kharif",
        "delivered_quantity_kg": 980.0, "committed_quantity_kg": 1000.0,
        "promised_delivery_date": "2025-10-20", "delivery_date": "2025-10-18",
        "price_per_quintal_inr": 4892.0, "moisture_pct": 10.5
    });
    let (s, _) = api
        .post("/api/fpo/purchase", &wh, "p-wh", proc_req.clone())
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let p1 = ok(
        "procurement",
        api.post("/api/fpo/purchase", &fpo, "proc-1", proc_req.clone())
            .await,
    );
    let p1_retry = ok(
        "procurement retry",
        api.post("/api/fpo/purchase", &fpo, "proc-1", proc_req.clone())
            .await,
    );
    assert_eq!(
        p1["operation_id"], p1_retry["operation_id"],
        "same key ⇒ same operation"
    );
    assert_eq!(p1["result"]["batch_id"], p1_retry["result"]["batch_id"]);
    let mut changed = proc_req.clone();
    changed["delivered_quantity_kg"] = json!(1.0);
    let (s, _) = api.post("/api/fpo/purchase", &fpo, "proc-1", changed).await;
    assert_eq!(s, StatusCode::CONFLICT, "same key, different body");
    let batch = p1["result"]["batch_id"].as_str().unwrap().to_string();
    let proc_tx = p1["tx_hash"].as_str().unwrap().to_string();

    // ---- provenance NFT minted from the procurement transaction
    let (s, body) = api
        .post(
            "/api/nft/mint-from-tx",
            &fpo,
            "m0",
            json!({"tx_hash": format!("0x{}", "ab".repeat(32))}),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "arbitrary tx hash is rejected: {body}"
    );
    let (s, _) = api
        .post(
            "/api/nft/mint-from-tx",
            &wh,
            "m1",
            json!({"tx_hash": proc_tx}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let reg_tx = reg["tx_hash"].as_str().unwrap();
    let (s, body) = api
        .post(
            "/api/nft/mint-from-tx",
            &fpo,
            "m2",
            json!({"tx_hash": reg_tx}),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "farmer registration is not a yield sale: {body}"
    );
    let mint = ok(
        "mint",
        api.post(
            "/api/nft/mint-from-tx",
            &fpo,
            "m3",
            json!({"tx_hash": proc_tx}),
        )
        .await,
    );
    assert!(mint["result"]["token_uri"]
        .as_str()
        .unwrap()
        .starts_with("ipfs://bafkrei"));
    let mint_again = ok(
        "mint again",
        api.post(
            "/api/nft/mint-from-tx",
            &fpo,
            "m4",
            json!({"tx_hash": proc_tx}),
        )
        .await,
    );
    assert_eq!(
        mint["operation_id"], mint_again["operation_id"],
        "no duplicate mint"
    );
    let token = ok("token", api.get("/api/nft/provenance/1", None).await);
    assert_eq!(token["owner"], fpo_addr.to_lowercase());
    assert_eq!(token["metadata_verified"], true, "{token:#}");
    assert_eq!(token["source_record"]["status"], "verified");
    assert_eq!(token["metadata"]["properties"]["batch_id"], batch);

    // ---- custody chain, warehouse, logistics
    let (s, body) = api
        .post(
            "/api/warehouse/inspection",
            &wh,
            "wi0",
            json!({
        "batch_id": batch, "warehouse_ref": "LTR-WH-3",
        "inspection": {"passed": true, "inspected_at": "2025-10-19T09:00:00Z"}}),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "warehouse is not custodian yet: {body}"
    );
    ok(
        "to warehouse",
        api.post(
            "/api/custody/transfer",
            &fpo,
            "c1",
            json!({"batch_id": batch, "to_participant_id": wh_id, "quantity_kg": 980.0}),
        )
        .await,
    );
    ok("inspection", api.post("/api/warehouse/inspection", &wh, "wi1", json!({
        "batch_id": batch, "warehouse_ref": "LTR-WH-3",
        "inspection": {"passed": true, "grade": "FAQ", "moisture_pct": 9.8, "foreign_matter_pct": 0.9, "oil_content_pct": 18.5, "inspected_at": "2025-10-19T09:00:00Z"},
        "storage": {"temperature_c": 27.5, "humidity_pct": 61.0}})).await);
    ok(
        "to logistics",
        api.post(
            "/api/custody/transfer",
            &wh,
            "c2",
            json!({"batch_id": batch, "to_participant_id": lg_id}),
        )
        .await,
    );
    let gps = ok(
        "gps upload",
        upload(
            &api,
            &lg,
            b"lat,lon,ts\n18.40,76.56,2025-10-20T18:00Z\n18.52,73.85,2025-10-21T06:30Z\n",
        )
        .await,
    );
    ok(
        "logistics",
        api.post(
            "/api/logistics/record",
            &lg,
            "l1",
            json!({
        "batch_id": batch, "shipment_ref": "SHP-77", "delivered": true,
        "checkpoint": {"name": "Sahyadri mill gate", "at": "2025-10-21T06:30:00Z"},
        "gps_trace_cid": gps["cid"], "distance_km": 312.0}),
        )
        .await,
    );
    ok(
        "to processor",
        api.post(
            "/api/custody/transfer",
            &lg,
            "c3",
            json!({"batch_id": batch, "to_participant_id": pr_id}),
        )
        .await,
    );

    // ---- processing + packaging
    let processed = ok("processing", api.post("/api/processing/batch", &pr, "pb1", json!({
        "input_batch_id": batch, "method": "expeller pressing + filtration",
        "outputs": [{"label": "OIL-K017", "product": "filtered_oil", "quantity_kg": 176.0}, {"label": "CAKE-K017", "product": "oilcake", "quantity_kg": 780.0}],
        "lab_report": {"oil_yield_pct": 18.0, "free_fatty_acid_pct": 0.8}})).await);
    let oil = processed["result"]["outputs"][0]["batch_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, _) = api.post("/api/logistics/record", &lg, "l2", json!({
        "batch_id": batch, "shipment_ref": "x", "delivered": false, "checkpoint": {"name": "x", "at": "2025-10-22T00:00:00Z"}})).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "consumed batch cannot take more records"
    );
    let sku_resp = ok(
        "packaging",
        api.post(
            "/api/packaging/sku",
            &pr,
            "sku1",
            json!({
        "batch_id": oil, "label": "Sahyadri Cold-Pressed Soybean Oil 1L",
        "unit_ids": ["U-0001", "U-0002", "U-0003"],
        "package": {"pack_size": "1 L", "material": "PET", "best_before": "2026-10-01"}}),
        )
        .await,
    );
    let sku = sku_resp["result"]["sku_id"].as_str().unwrap().to_string();

    // ---- consumer verification: full lineage, integrity, unit inclusion
    let v = ok(
        "verify sku",
        api.get(&format!("/api/skus/{sku}/verify?unit_id=U-0002"), None)
            .await,
    );
    assert_eq!(v["verified"], true, "{v:#}");
    assert_eq!(v["unit"]["included"], true);
    let lineage = v["provenance"]["lineage"].as_array().unwrap();
    assert_eq!(lineage.len(), 2);
    assert!(lineage.iter().all(|b| b["timeline_complete"] == true));
    let types: Vec<&str> = lineage[0]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["record_type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "procurement",
            "custody_transfer",
            "warehouse_inspection",
            "custody_transfer",
            "logistics",
            "custody_transfer",
            "processing"
        ]
    );
    assert_eq!(lineage[1]["records"][0]["record_type"], "packaging");
    assert_eq!(v["provenance"]["farmer"]["status"], "verified");
    let bad_unit = ok(
        "bad unit",
        api.get(&format!("/api/skus/{sku}/verify?unit_id=U-9999"), None)
            .await,
    );
    assert_eq!(bad_unit["unit"]["included"], false);
    assert_eq!(
        bad_unit["verified"], false,
        "a unit outside the SKU must not verify"
    );

    // ---- tampering is detected explicitly
    let log = state
        .chain_client
        .record_logs(oilschain::chain::parse_b256(&batch).unwrap())
        .await
        .unwrap()[0]
        .clone();
    let names = Default::default();
    let mut forged = log.clone();
    forged.content_hash = alloy::primitives::B256::repeat_byte(9);
    assert_eq!(
        verify::verify_log(&state, &forged, &names).await.status,
        verify::RecordStatus::DigestMismatch
    );
    let other = state
        .ipfs_client
        .put_block(b"{\"not\":\"the record\"}")
        .await
        .unwrap();
    let mut swapped = log.clone();
    swapped.cid = other.to_string();
    assert_eq!(
        verify::verify_log(&state, &swapped, &names).await.status,
        verify::RecordStatus::HashMismatch
    );
    let mut missing = log.clone();
    missing.cid = oilschain::cid::Cid::raw_sha256(b"never uploaded").to_string();
    assert_eq!(
        verify::verify_log(&state, &missing, &names).await.status,
        verify::RecordStatus::ContentUnavailable
    );

    // ---- fraud report + independent review (only upheld + farmer-attributed counts in rankings)
    let fr = ok("fraud report", api.post("/api/fraud/report", &retailer, "fr1", json!({
        "sku_id": sku, "category": "quantity_shortfall", "description": "Seal intact but 920 ml measured", "evidence_cids": []})).await);
    let report = fr["result"]["report_digest"].as_str().unwrap().to_string();
    let (s, _) = api.post("/api/fraud/resolve", &fpo, "rs0", json!({"report_digest": report, "upheld": true, "attribution": "packager", "notes": "x"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    ok("fraud resolve", api.post("/api/fraud/resolve", ADMIN_TOKEN, "rs1", json!({
        "report_digest": report, "upheld": true, "attribution": "packager", "notes": "Filling line under-calibrated."})).await);

    // ---- AI assessment: commit now, reveal only after the 1h window (chain time)
    let commit = ok(
        "ai commit",
        api.post(
            "/api/ai/commit",
            &oracle,
            "ai1",
            json!({"batch_id": oil, "assessment": {
        "model_id": "oil-quality-nir", "model_version": "0.3.1", "method": "NIR spectra regression",
        "outputs": {"predicted_ffa_pct": 0.82, "grade": "A"}}}),
        )
        .await,
    );
    assert_eq!(
        commit["cid"],
        Value::Null,
        "nothing is published before reveal"
    );
    let (s, body) = api
        .call(
            "POST",
            "/api/ai/reveal",
            Some(&oracle),
            None,
            Some(json!({"batch_id": oil})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "reveal before window: {body}");
    let p = state.chain_client.provider();
    let _: Value = p
        .raw_request("evm_increaseTime".into(), [3601])
        .await
        .unwrap();
    let _: Value = p.raw_request("evm_mine".into(), ()).await.unwrap();
    reconciler::run_once(&state).await.unwrap();
    let ai = ok(
        "ai status",
        api.get(&format!("/api/ai/{oil}"), Some(&oracle)).await,
    );
    assert_eq!(ai["status"], "revealed", "{ai:#}");
    let prov = ok(
        "provenance",
        api.get(&format!("/api/batches/{oil}/provenance"), None)
            .await,
    );
    assert_eq!(prov["verified"], true, "{prov:#}");
    let oil_types: Vec<&str> = prov["lineage"][1]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["record_type"].as_str().unwrap())
        .collect();
    assert_eq!(
        oil_types,
        [
            "packaging",
            "fraud_report",
            "fraud_resolution",
            "ai_assessment"
        ]
    );

    // ---- soulbound farmer credential
    let cred = ok(
        "credential",
        api.call(
            "POST",
            "/api/nft/credential",
            Some(&fpo),
            None,
            Some(json!({"farmer_id": farmer_id})),
        )
        .await,
    );
    assert_eq!(cred["status"], "confirmed");
    let ct = ok(
        "credential token",
        api.get("/api/nft/credential/1", None).await,
    );
    assert_eq!(ct["soulbound"], true);
    assert_eq!(ct["owner"], FARMER_WALLET);
    assert_eq!(ct["metadata_verified"], true, "{ct:#}");

    // ---- second, cold-start farmer and rankings
    let mut f2 = farmer_req.clone();
    f2["label"] = json!("SFPO member 0107");
    f2["wallet_address"] = Value::Null;
    f2.as_object_mut().unwrap().remove("wallet_address");
    let reg2 = ok(
        "register farmer 2",
        api.post("/api/farmer/register", &fpo, "farmer-2", f2).await,
    );
    let farmer2 = reg2["result"]["farmer_id"].as_str().unwrap().to_string();
    let ranks = ok(
        "rankings",
        api.get("/api/rankings/farmers", Some(&fpo)).await,
    );
    assert_eq!(ranks["total"], 2);
    let items = ranks["items"].as_array().unwrap();
    let cold = items.iter().find(|i| i["farmer_id"] == farmer2).unwrap();
    assert_eq!(cold["confidence"], "none");
    assert_eq!(cold["score"], ranks["population"]["baseline_score"]);
    let first = items.iter().find(|i| i["farmer_id"] == farmer_id).unwrap();
    assert_eq!(first["evidence_lots"], 1);
    let integrity = first["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "integrity_rate")
        .unwrap();
    assert_eq!(
        integrity["observed"]["successes"], 1.0,
        "packager-attributed fraud must not count against the farmer"
    );
    assert!(ranks["dataset"]["fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("0x"));
    let (s, _) = api.get("/api/rankings/farmers", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // ---- advisory through the (fake) local model, grounded in verified facts
    let adv = ok(
        "advisory",
        api.call(
            "POST",
            "/api/advisory",
            Some(&fpo),
            None,
            Some(json!({
        "question": "When should I sell my soybean?", "language": "en", "farmer_id": farmer_id})),
        )
        .await,
    );
    assert!(
        adv["answer"]
            .as_str()
            .unwrap()
            .contains("Verified lots on record: 1."),
        "{adv:#}"
    );
    let (s, _) = api
        .call(
            "POST",
            "/api/advisory",
            Some(&fpo),
            None,
            Some(json!({"question": "?", "language": "fr"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // ---- recovery: IPFS outage leaves the op `prepared`; a retry with the same key completes it
    let dead_cfg = config(
        &rpc,
        &format!("http://127.0.0.1:{}", free_port()),
        contracts,
        db_path.clone(),
        None,
        None,
    );
    let dead_state = AppState::from_parts(
        dead_cfg.clone(),
        state.db.clone(),
        ChainClient::connect(&dead_cfg.chain).await.unwrap(),
        IpfsClient::new(dead_cfg.ipfs.clone()).unwrap(),
    );
    let dead_api = {
        let app = axum::Router::new().merge(routes::configure_routes(dead_state));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        Api {
            base,
            http: reqwest::Client::new(),
        }
    };
    let mut proc2 = proc_req.clone();
    proc2["label"] = json!("LOT-2025-K-018");
    let (s, body) = dead_api
        .post("/api/fpo/purchase", &fpo, "proc-outage", proc2.clone())
        .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{body}");
    let pending: Vec<_> = state
        .db
        .call(|c| db::operations_with_status(c, &[OpStatus::Prepared]))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "op is journaled, not lost");
    let resumed = ok(
        "resume after outage",
        api.post("/api/fpo/purchase", &fpo, "proc-outage", proc2)
            .await,
    );
    assert_eq!(resumed["operation_id"].as_str().unwrap(), pending[0].id);

    // ---- recovery: transaction landed but the response was lost ⇒ no resend, recovered from chain logs
    let actor = ops::actor_for(&state, &fpo_id).await.unwrap();
    let rec = oilschain::records::prepare(
        &state,
        oilschain::canonical::RecordType::CustodyTransfer,
        oilschain::chain::parse_b256(&oil).unwrap(),
        actor.address,
        json!({"note": "direct"}),
    )
    .unwrap();
    // Processor holds `oil`; send the anchoring transaction directly, as if the API crashed afterwards.
    let pr_actor = ops::actor_for(&state, &pr_id).await.unwrap();
    state.ipfs_client.put_block(&rec.payload).await.unwrap();
    let action = ChainAction::TransferCustody {
        batch_id: oil.clone(),
        to: fpo_addr.clone(),
        content_hash: oilschain::chain::hex32(rec.content_hash),
        cid: rec.cid.to_string(),
    };
    state
        .chain_client
        .send(pr_actor.signer(&state).unwrap(), &action)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let op = ops::submit(
        &state,
        &pr_actor,
        ops::NewOperation {
            idem_key: "lost-response".into(),
            kind: "custody_transfer",
            request_hash: "lost".into(),
            record: Some(rec),
            action,
            context: json!({"result": {}}),
        },
    )
    .await
    .unwrap();
    assert_eq!(op.status, OpStatus::Confirmed);
    assert!(
        op.tx_hash.is_some(),
        "tx hash recovered from the RecordAnchored log"
    );
    assert_eq!(op.attempts, 0, "nothing was resent");
    assert_eq!(
        state
            .chain_client
            .registry()
            .custodianOf(oilschain::chain::parse_b256(&oil).unwrap())
            .call()
            .await
            .unwrap(),
        fpo_addr.parse::<Address>().unwrap()
    );
    let _ = pr_addr;

    // ---- orchestrated workflow: completes, and re-running with the same key is idempotent
    let wf = json!({
        "workflow_key": "wf-demo-1",
        "participants": {"fpo": fpo_id, "warehouse": wh_id, "logistics": lg_id, "processor": pr_id},
        "farmer_id": farmer2,
        "procurement": {"label": "LOT-2025-K-030", "crop": "groundnut", "delivered_quantity_kg": 500.0, "delivery_date": "2025-11-02"},
        "warehouse": {"warehouse_ref": "LTR-WH-3", "inspection": {"passed": false, "moisture_pct": 14.2, "inspected_at": "2025-11-03T08:00:00Z"}},
        "logistics": [{"shipment_ref": "SHP-90", "delivered": true, "checkpoint": {"name": "mill", "at": "2025-11-04T10:00:00Z"}}],
        "processing": {"method": "expeller", "outputs": [{"label": "GN-OIL-30", "product": "crude_oil", "quantity_kg": 200.0}]},
        "packaging": {"label": "GN oil 5L", "unit_ids": ["G-1", "G-2"]}
    });
    let (s, run1) = api
        .call(
            "POST",
            "/api/workflow/execute",
            Some(ADMIN_TOKEN),
            None,
            Some(wf.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{run1:#}");
    assert_eq!(run1["status"], "completed");
    let (_, run2) = api
        .call(
            "POST",
            "/api/workflow/execute",
            Some(ADMIN_TOKEN),
            None,
            Some(wf),
        )
        .await;
    assert_eq!(run1["ids"], run2["ids"], "resume/idempotency");
    let wf_sku = run1["ids"]["sku_id"].as_str().unwrap();
    let wv = ok(
        "verify workflow sku",
        api.get(&format!("/api/skus/{wf_sku}/verify"), None).await,
    );
    assert_eq!(wv["verified"], true);

    // Rankings now reflect the failed inspection of farmer 2.
    let ranks = ok(
        "rankings 2",
        api.get("/api/rankings/farmers?sort=lower_bound", Some(&fpo))
            .await,
    );
    let f2r = ranks["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["farmer_id"] == farmer2)
        .unwrap();
    let q = f2r["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "quality_pass_rate")
        .unwrap();
    assert_eq!(q["observed"], json!({"successes": 0.0, "trials": 1}));
    assert!(q["contribution"].as_f64().unwrap() < 0.0);
    // ---- backup pinning: every stored block is replicated to the second node and read back
    oilschain::pinning::drain(&state, 20).await.unwrap();
    let backup_client = IpfsClient::new(IpfsConfig {
        api_url: backup_url.clone(),
        gateway_url: String::new(),
        auth_header: None,
        timeout: Duration::from_secs(5),
        max_retries: 1,
    })
    .unwrap();
    let pending = state
        .db
        .call(|c| db::pin_jobs_with_status(c, Some("pending"), 1000))
        .await
        .unwrap();
    assert!(pending.is_empty(), "all pins processed: {pending:?}");
    let failed = state
        .db
        .call(|c| db::pin_jobs_with_status(c, Some("failed"), 1000))
        .await
        .unwrap();
    assert!(failed.is_empty(), "no failed pins: {failed:?}");
    let pinned = state
        .db
        .call(|c| db::pin_jobs_with_status(c, Some("pinned"), 1000))
        .await
        .unwrap();
    assert!(
        pinned.len() > 20,
        "records, metadata and attachments replicated: {}",
        pinned.len()
    );
    for j in &pinned {
        let cid = oilschain::cid::Cid::parse(&j.cid).unwrap();
        backup_client
            .get_block(&cid)
            .await
            .expect("block present on backup node");
    }
    let pv = ok(
        "pins",
        api.get(
            &format!("/api/ipfs/pins/{}", reg["cid"].as_str().unwrap()),
            None,
        )
        .await,
    );
    assert_eq!(pv["backups"][0]["status"], "pinned", "{pv:#}");
    let prov = ok(
        "provenance with replicas",
        api.get(&format!("/api/batches/{oil}/provenance"), None)
            .await,
    );
    assert_eq!(
        prov["lineage"][0]["records"][0]["replicas"][0]["status"],
        "pinned"
    );

    // ---- tampered confidential layouts anchored directly on-chain are caught
    let fpo_actor = ops::actor_for(&state, &fpo_id).await.unwrap();
    let domain = state.domain();
    let rt = oilschain::canonical::RecordType::FarmerRegistration;
    // (a) a private field written into the public part
    let fid = oilschain::chain::generate_id();
    let leaky = oilschain::canonical::build_record(
        domain,
        rt,
        fid,
        fpo_actor.address,
        json!({"label": "leak", "profile": {"name": "Exposed Name"}}),
    );
    let bytes = oilschain::canonical::canonical_json(&leaky).unwrap();
    let log = anchor_crafted(&state, &fpo_actor, fid, &bytes, &bytes).await;
    assert_eq!(
        verify::verify_log(&state, &log, &names).await.status,
        verify::RecordStatus::PrivateFieldExposed
    );
    // (b) ciphertext that does not match the committed private_commitment
    let fid = oilschain::chain::generate_id();
    let mut rec = oilschain::canonical::build_record(
        domain,
        rt,
        fid,
        fpo_actor.address,
        json!({"label": "swap"}),
    );
    let commitment = alloy::primitives::keccak256(b"{}");
    rec["private_commitment"] = json!(format!("{commitment:#x}"));
    let committed = oilschain::canonical::canonical_json(&rec).unwrap();
    let mut stored = rec.clone();
    stored["private_ciphertext"] = oilschain::crypto::encrypt_envelope_value(
        &[2u8; 32],
        br#"{"profile":{"name":"Someone Else"},"salt":"0x00"}"#,
        &oilschain::canonical::private_aad(rt, fid, commitment),
    );
    let payload = oilschain::canonical::canonical_json(&stored).unwrap();
    let log = anchor_crafted(&state, &fpo_actor, fid, &committed, &payload).await;
    assert_eq!(
        verify::verify_log(&state, &log, &names).await.status,
        verify::RecordStatus::PrivateCommitmentMismatch
    );

    println!(
        "e2e OK: {} rankings, sku {sku}, {} backup pins",
        ranks["total"],
        pinned.len()
    );
}

/// Store `payload` on IPFS and anchor `keccak256(committed)` as a farmer registration, bypassing the API's
/// record builder (simulates a buggy or malicious writer).
async fn anchor_crafted(
    state: &AppState,
    actor: &oilschain::auth::Actor,
    farmer_id: alloy::primitives::B256,
    committed: &[u8],
    payload: &[u8],
) -> oilschain::chain::AnchoredLog {
    let cid = state.ipfs_client.put_block(payload).await.unwrap();
    let action = ChainAction::RegisterFarmer {
        farmer_id: oilschain::chain::hex32(farmer_id),
        content_hash: oilschain::chain::hex32(alloy::primitives::keccak256(committed)),
        cid: cid.to_string(),
    };
    let tx = state
        .chain_client
        .send(actor.signer(state).unwrap(), &action)
        .await
        .unwrap();
    let r = state
        .chain_client
        .wait_for_receipt(tx, Duration::from_secs(10))
        .await
        .unwrap()
        .unwrap();
    assert!(r.status());
    state
        .chain_client
        .record_logs(farmer_id)
        .await
        .unwrap()
        .remove(0)
}

/// Upload an attachment (e.g. a GPS trace) as raw bytes.
async fn upload(api: &Api, token: &str, bytes: &[u8]) -> (StatusCode, Value) {
    let resp = api
        .http
        .post(format!("{}/api/ipfs/upload", api.base))
        .bearer_auth(token)
        .header("content-type", "text/csv")
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    (resp.status(), resp.json().await.unwrap())
}
