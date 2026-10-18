//! Farmer supply-reliability ranking (model `farmer-reliability`, version `bayes-shrinkage-v1`).
//!
//! What it ranks: farmers, by how reliably their lots met agreed terms and passed inspection, measured only from
//! records anchored on-chain by *other* participants (FPOs, warehouses, the fraud reviewer). It is a deterministic
//! statistical model, not a trained ML model, and it is not a credit or insurance score.
//!
//! Each feature is a rate with a Beta prior centred on the population rate (empirical Bayes):
//!   estimate = (successes + k·m) / (trials + k),  var = estimate·(1−estimate) / (trials + k + 1)
//! (m is the population rate clamped to [0.05, 0.95]) so a farmer with no history scores exactly the baseline (cold start is neither rewarded nor
//! penalised) and estimates move toward the farmer's own record as evidence accumulates.
//!   score = 100 · Σ wᵢ·estimateᵢ,  contributionᵢ = 100 · wᵢ·(estimateᵢ − mᵢ),  sd = 100 · √(Σ wᵢ²·varᵢ)
//! Volume, farm size, price and transaction count are deliberately not features. Each lot counts once per
//! feature, so repeating events for one lot cannot inflate a score. Specification: docs/RANKING.md.

use crate::auth::Actor;
use crate::canonical;
use crate::chain::hex32;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MODEL_NAME: &str = "farmer-reliability";
pub const MODEL_VERSION: &str = "bayes-shrinkage-v1";
pub const Z90: f64 = 1.645;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    QualityPassRate,
    FulfilmentRate,
    OnTimeRate,
    IntegrityRate,
}

impl Feature {
    pub const ALL: [Feature; 4] = [
        Feature::QualityPassRate,
        Feature::FulfilmentRate,
        Feature::OnTimeRate,
        Feature::IntegrityRate,
    ];

    pub fn weight(self) -> f64 {
        match self {
            Feature::QualityPassRate => 0.35,
            Feature::FulfilmentRate => 0.25,
            Feature::OnTimeRate => 0.20,
            Feature::IntegrityRate => 0.20,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Feature::QualityPassRate => "quality_pass_rate",
            Feature::FulfilmentRate => "fulfilment_rate",
            Feature::OnTimeRate => "on_time_rate",
            Feature::IntegrityRate => "integrity_rate",
        }
    }
}

/// Per-lot facts extracted from verified records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LotFacts {
    pub batch_id: String,
    pub crop: Option<String>,
    /// Latest warehouse inspection outcome, if any.
    pub inspection_passed: Option<bool>,
    /// min(delivered / committed, 1) when a commitment was recorded.
    pub fulfilment: Option<f64>,
    /// delivery_date <= promised_delivery_date when a promise was recorded.
    pub on_time: Option<bool>,
    /// An upheld fraud finding attributed to the farmer exists for this lot (or its products).
    pub upheld_farmer_fraud: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FarmerFacts {
    pub farmer_id: String,
    pub label: String,
    pub primary_crop: Option<String>,
    pub lots: Vec<LotFacts>,
}

#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub farmers: Vec<FarmerFacts>,
    pub records_considered: usize,
    pub records_excluded: usize,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Observed {
    pub successes: f64,
    pub trials: u32,
}

impl Observed {
    fn of(lots: &[LotFacts], f: Feature) -> Self {
        let mut o = Observed::default();
        for l in lots {
            let s = match f {
                Feature::QualityPassRate => l.inspection_passed.map(|p| p as u8 as f64),
                Feature::FulfilmentRate => l.fulfilment,
                Feature::OnTimeRate => l.on_time.map(|p| p as u8 as f64),
                Feature::IntegrityRate => Some(if l.upheld_farmer_fraud { 0.0 } else { 1.0 }),
            };
            if let Some(s) = s {
                o.successes += s;
                o.trials += 1;
            }
        }
        o
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureResult {
    pub name: &'static str,
    pub weight: f64,
    /// Observed facts (from verified records).
    pub observed: Observed,
    /// Population rate used as the prior mean.
    pub baseline: f64,
    /// Posterior estimate (a prediction, not an observation).
    pub estimate: f64,
    pub std_dev: f64,
    /// Points added to / removed from the cold-start baseline score.
    pub contribution: f64,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedFarmer {
    pub rank: usize,
    pub farmer_id: String,
    pub label: String,
    pub primary_crop: Option<String>,
    pub score: f64,
    pub std_dev: f64,
    pub interval_90: [f64; 2],
    pub confidence: &'static str,
    pub evidence_lots: usize,
    pub features: Vec<FeatureResult>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub prior_strength: f64,
    pub weights: BTreeMap<&'static str, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Population {
    pub baseline_score: f64,
    pub rates: BTreeMap<&'static str, f64>,
    pub farmers: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankingResponse {
    pub model: ModelInfo,
    pub intended_use: &'static str,
    pub not_for: [&'static str; 3],
    pub generated_at: String,
    pub dataset: DatasetInfo,
    pub population: Population,
    pub sort: &'static str,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub items: Vec<RankedFarmer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetInfo {
    pub records_considered: usize,
    pub records_excluded: usize,
    pub fingerprint: String,
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn explain(f: Feature, o: Observed, baseline: f64, contribution: f64) -> String {
    let effect = if contribution.abs() < 0.05 {
        "no effect".to_string()
    } else {
        format!("{:+.1} points", contribution)
    };
    if o.trials == 0 {
        return format!(
            "No data yet; uses the population rate {} ({effect}).",
            pct(baseline)
        );
    }
    match f {
        Feature::QualityPassRate => format!(
            "{} of {} inspected lots passed warehouse inspection vs population {} ({effect}).",
            o.successes as u32,
            o.trials,
            pct(baseline)
        ),
        Feature::FulfilmentRate => format!(
            "Delivered {} of the committed quantity on average across {} lots vs population {} ({effect}).",
            pct(o.successes / o.trials as f64),
            o.trials,
            pct(baseline)
        ),
        Feature::OnTimeRate => format!(
            "{} of {} lots delivered on or before the promised date vs population {} ({effect}).",
            o.successes as u32,
            o.trials,
            pct(baseline)
        ),
        Feature::IntegrityRate => {
            let upheld = o.trials - o.successes as u32;
            format!(
                "{upheld} upheld fraud finding(s) attributed to the farmer across {} lots ({effect}). Open or dismissed reports are not counted.",
                o.trials
            )
        }
    }
}

/// Pure scoring: same dataset + prior strength ⇒ same output (apart from `generated_at`).
pub fn score_all(dataset: &Dataset, prior_strength: f64) -> (Population, Vec<RankedFarmer>) {
    let k = prior_strength.max(0.0);
    let mut rates = BTreeMap::new();
    let mut baselines = HashMap::new();
    for f in Feature::ALL {
        let (mut s, mut n) = (0.0, 0u32);
        for farmer in &dataset.farmers {
            let o = Observed::of(&farmer.lots, f);
            s += o.successes;
            n += o.trials;
        }
        // Clamp so a degenerate population (all 0% or all 100%) cannot collapse the prior's variance.
        let m = if n == 0 {
            0.5
        } else {
            (s / n as f64).clamp(0.05, 0.95)
        };
        baselines.insert(f, m);
        rates.insert(f.name(), round(m, 4));
    }
    let baseline_score = 100.0
        * Feature::ALL
            .iter()
            .map(|f| f.weight() * baselines[f])
            .sum::<f64>();

    let mut ranked: Vec<RankedFarmer> = dataset
        .farmers
        .iter()
        .map(|farmer| {
            let mut features = Vec::new();
            let mut score = 0.0;
            let mut var = 0.0;
            for f in Feature::ALL {
                let o = Observed::of(&farmer.lots, f);
                let m = baselines[&f];
                let denom = o.trials as f64 + k;
                let est = if denom == 0.0 { m } else { (o.successes + k * m) / denom };
                let v = est * (1.0 - est) / (denom + 1.0);
                let w = f.weight();
                let contribution = 100.0 * w * (est - m);
                score += 100.0 * w * est;
                var += (100.0 * w).powi(2) * v;
                features.push(FeatureResult {
                    name: f.name(),
                    weight: w,
                    observed: Observed {
                        successes: round(o.successes, 4),
                        trials: o.trials,
                    },
                    baseline: round(m, 4),
                    estimate: round(est, 4),
                    std_dev: round(v.sqrt(), 4),
                    contribution: round(contribution, 2),
                    explanation: explain(f, o, m, contribution),
                });
            }
            let sd = var.sqrt();
            let evidence = farmer.lots.len();
            // Needs both enough lots and a narrow interval.
            let confidence = if evidence == 0 {
                "none"
            } else if evidence < 3 || sd >= 7.0 {
                "low"
            } else if evidence < 10 || sd >= 4.0 {
                "medium"
            } else {
                "high"
            };
            let mut sorted: Vec<&FeatureResult> = features.iter().collect();
            sorted.sort_by(|a, b| b.contribution.abs().total_cmp(&a.contribution.abs()));
            let explanation = if evidence == 0 {
                format!(
                    "No verified lots yet: score equals the population baseline {:.1}. This is a cold-start placeholder, not a judgement.",
                    baseline_score
                )
            } else if sorted[0].contribution.abs() < 0.05 {
                format!(
                    "Score {:.1} equals the cold-start baseline: {} verified lot(s) do not yet differ from the population. 90% interval {:.1}–{:.1}.",
                    score,
                    evidence,
                    (score - Z90 * sd).max(0.0),
                    (score + Z90 * sd).min(100.0)
                )
            } else {
                let top = sorted[0];
                format!(
                    "Score {:.1} vs cold-start baseline {:.1} from {} verified lot(s); largest factor: {} ({:+.1}). 90% interval {:.1}–{:.1}.",
                    score,
                    baseline_score,
                    evidence,
                    top.name,
                    top.contribution,
                    (score - Z90 * sd).max(0.0),
                    (score + Z90 * sd).min(100.0)
                )
            };
            RankedFarmer {
                rank: 0,
                farmer_id: farmer.farmer_id.clone(),
                label: farmer.label.clone(),
                primary_crop: farmer.primary_crop.clone(),
                score: round(score, 2),
                std_dev: round(sd, 2),
                interval_90: [round((score - Z90 * sd).max(0.0), 2), round((score + Z90 * sd).min(100.0), 2)],
                confidence,
                evidence_lots: evidence,
                features,
                explanation,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.farmer_id.cmp(&b.farmer_id))
    });
    for (i, r) in ranked.iter_mut().enumerate() {
        r.rank = i + 1;
    }
    (
        Population {
            baseline_score: round(baseline_score, 2),
            rates,
            farmers: dataset.farmers.len(),
        },
        ranked,
    )
}

/// Build the dataset from confirmed records in the database. Each cached record is re-hashed against its
/// anchored content hash; mismatches are excluded and counted.
pub async fn load_dataset(state: &AppState) -> Result<Dataset, ApiError> {
    let farmers = state.db.call(db::list_farmers).await?;
    let batches = state.db.call(|c| db::list_batches(c, None)).await?;
    let records = state.db.call(db::all_records).await?;
    let fraud = state.db.call(db::all_fraud).await?;

    let mut parent_of: HashMap<String, String> = HashMap::new();
    let mut raw_by_farmer: HashMap<String, Vec<&db::BatchRow>> = HashMap::new();
    for b in &batches {
        match &b.parent_batch_id {
            Some(p) => {
                parent_of.insert(b.batch_id.clone(), p.clone());
            }
            None => raw_by_farmer
                .entry(b.farmer_id.clone())
                .or_default()
                .push(b),
        }
    }
    let root_of = |mut id: String| -> String {
        for _ in 0..8 {
            match parent_of.get(&id) {
                Some(p) => id = p.clone(),
                None => break,
            }
        }
        id
    };

    let mut considered = 0;
    let mut excluded = 0;
    let mut fingerprint_items: Vec<String> = Vec::new();
    let mut lots: HashMap<String, LotFacts> = HashMap::new();
    let mut latest_inspection: HashMap<String, (u64, String, bool)> = HashMap::new();

    for r in &records {
        let rt = r.record_type;
        if rt != canonical::RecordType::Procurement.as_u8()
            && rt != canonical::RecordType::WarehouseInspection.as_u8()
        {
            continue;
        }
        let Some(json) = &r.public_json else { continue };
        if hex32(canonical::content_hash(json.as_bytes())) != r.content_hash {
            excluded += 1;
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(json) else {
            excluded += 1;
            continue;
        };
        considered += 1;
        fingerprint_items.push(r.record_digest.clone());
        let data = &v["data"];
        let lot = lots
            .entry(r.subject_id.clone())
            .or_insert_with(|| LotFacts {
                batch_id: r.subject_id.clone(),
                ..Default::default()
            });
        if rt == canonical::RecordType::Procurement.as_u8() {
            lot.crop = data["crop"].as_str().map(str::to_string);
            let delivered = data["delivered_quantity_kg"].as_f64();
            if let (Some(d), Some(c)) = (delivered, data["committed_quantity_kg"].as_f64()) {
                if c > 0.0 {
                    lot.fulfilment = Some((d / c).min(1.0));
                }
            }
            if let (Some(d), Some(p)) = (
                data["delivery_date"].as_str(),
                data["promised_delivery_date"].as_str(),
            ) {
                // ISO dates compare lexicographically.
                lot.on_time = Some(d <= p);
            }
        } else if let Some(passed) = data["inspection"]["passed"].as_bool() {
            let key = (r.block_number.unwrap_or(0), r.created_at.clone(), passed);
            let entry = latest_inspection
                .entry(r.subject_id.clone())
                .or_insert(key.clone());
            if (key.0, &key.1) >= (entry.0, &entry.1) {
                *entry = key;
            }
        }
    }
    for (batch, (_, _, passed)) in latest_inspection {
        if let Some(lot) = lots.get_mut(&batch) {
            lot.inspection_passed = Some(passed);
        }
    }
    let mut fraud_lots: HashSet<String> = HashSet::new();
    for f in &fraud {
        fingerprint_items.push(format!(
            "{}:{}:{}",
            f.report_digest,
            f.status,
            f.attribution.as_deref().unwrap_or("")
        ));
        if f.status == "upheld" && f.attribution.as_deref() == Some("farmer") {
            fraud_lots.insert(root_of(f.batch_id.clone()));
        }
    }

    let mut out = Vec::new();
    for f in farmers {
        let mut farmer_lots: Vec<LotFacts> = raw_by_farmer
            .get(&f.farmer_id)
            .map(|bs| {
                bs.iter()
                    .filter_map(|b| lots.get(&b.batch_id).cloned())
                    .map(|mut l| {
                        l.upheld_farmer_fraud = fraud_lots.contains(&l.batch_id);
                        l
                    })
                    .collect()
            })
            .unwrap_or_default();
        farmer_lots.sort_by(|a, b| a.batch_id.cmp(&b.batch_id));
        out.push(FarmerFacts {
            farmer_id: f.farmer_id,
            label: f.label,
            primary_crop: f.primary_crop,
            lots: farmer_lots,
        });
    }
    out.sort_by(|a, b| a.farmer_id.cmp(&b.farmer_id));
    fingerprint_items.sort();
    let fingerprint = format!(
        "0x{}",
        hex::encode(crate::crypto::sha256(
            format!("{MODEL_VERSION}|{}", fingerprint_items.join(",")).as_bytes()
        ))
    );
    Ok(Dataset {
        farmers: out,
        records_considered: considered,
        records_excluded: excluded,
        fingerprint,
    })
}

#[derive(Debug, Deserialize)]
pub struct RankingQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub crop: Option<String>,
    /// `score` (default) or `lower_bound` (rank by the 90% interval's lower end — conservative).
    pub sort: Option<String>,
    pub min_evidence: Option<usize>,
}

fn model_info(prior_strength: f64) -> ModelInfo {
    ModelInfo {
        name: MODEL_NAME,
        version: MODEL_VERSION,
        kind: "deterministic empirical-Bayes weighted scoring (not a trained ML model)",
        prior_strength,
        weights: Feature::ALL
            .iter()
            .map(|f| (f.name(), f.weight()))
            .collect(),
    }
}

const INTENDED_USE: &str =
    "Operational supply reliability of farmers, for FPO procurement planning and support prioritisation.";
const NOT_FOR: [&str; 3] = [
    "credit or loan decisions",
    "insurance eligibility or pricing",
    "excluding farmers without human review",
];

/// GET /api/rankings/farmers (participants)
pub async fn rank_farmers(
    State(state): State<AppState>,
    actor: Actor,
    Query(q): Query<RankingQuery>,
) -> Result<Json<RankingResponse>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let limit = q.limit.unwrap_or(20);
    if limit == 0 || limit > 100 {
        return Err(ApiError::bad_request("limit must be 1-100"));
    }
    let offset = q.offset.unwrap_or(0);
    let sort = match q.sort.as_deref() {
        None | Some("score") => "score",
        Some("lower_bound") => "lower_bound",
        Some(_) => {
            return Err(ApiError::bad_request(
                "sort must be `score` or `lower_bound`",
            ))
        }
    };
    let crop = q.crop.map(|c| c.to_lowercase());
    let dataset = load_dataset(&state).await?;
    let k = state.config.ranking_prior_strength;
    let (population, mut ranked) = score_all(&dataset, k);

    let crops: HashMap<String, HashSet<String>> = dataset
        .farmers
        .iter()
        .map(|f| {
            let mut s: HashSet<String> = f.lots.iter().filter_map(|l| l.crop.clone()).collect();
            if let Some(c) = &f.primary_crop {
                s.insert(c.clone());
            }
            (f.farmer_id.clone(), s)
        })
        .collect();
    ranked.retain(|r| {
        crop.as_ref()
            .is_none_or(|c| crops.get(&r.farmer_id).is_some_and(|s| s.contains(c)))
            && r.evidence_lots >= q.min_evidence.unwrap_or(0)
    });
    if sort == "lower_bound" {
        ranked.sort_by(|a, b| {
            b.interval_90[0]
                .total_cmp(&a.interval_90[0])
                .then_with(|| a.farmer_id.cmp(&b.farmer_id))
        });
    }
    for (i, r) in ranked.iter_mut().enumerate() {
        r.rank = i + 1;
    }
    let total = ranked.len();
    let items = ranked.into_iter().skip(offset).take(limit).collect();
    Ok(Json(RankingResponse {
        model: model_info(k),
        intended_use: INTENDED_USE,
        not_for: NOT_FOR,
        generated_at: db::now(),
        dataset: DatasetInfo {
            records_considered: dataset.records_considered,
            records_excluded: dataset.records_excluded,
            fingerprint: dataset.fingerprint,
        },
        population,
        sort,
        total,
        offset,
        limit,
        items,
    }))
}

#[derive(Debug, Serialize)]
pub struct FarmerRankingResponse {
    pub model: ModelInfo,
    pub intended_use: &'static str,
    pub not_for: [&'static str; 3],
    pub generated_at: String,
    pub dataset: DatasetInfo,
    pub population: Population,
    pub farmer: RankedFarmer,
    pub out_of: usize,
}

/// GET /api/rankings/farmers/{id}
pub async fn farmer_ranking(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<String>,
) -> Result<Json<FarmerRankingResponse>, ApiError> {
    if !actor.is_admin {
        actor.require_any_role()?;
    }
    let farmer_id = hex32(crate::supply_chain_handlers::parse_id("farmer_id", &id)?);
    let dataset = load_dataset(&state).await?;
    let k = state.config.ranking_prior_strength;
    let (population, ranked) = score_all(&dataset, k);
    let out_of = ranked.len();
    let farmer = ranked
        .into_iter()
        .find(|r| r.farmer_id == farmer_id)
        .ok_or_else(|| ApiError::NotFound("farmer not found".into()))?;
    Ok(Json(FarmerRankingResponse {
        model: model_info(k),
        intended_use: INTENDED_USE,
        not_for: NOT_FOR,
        generated_at: db::now(),
        dataset: DatasetInfo {
            records_considered: dataset.records_considered,
            records_excluded: dataset.records_excluded,
            fingerprint: dataset.fingerprint,
        },
        population,
        farmer,
        out_of,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(
        id: &str,
        passed: Option<bool>,
        fulfil: Option<f64>,
        on_time: Option<bool>,
        fraud: bool,
    ) -> LotFacts {
        LotFacts {
            batch_id: id.into(),
            crop: Some("groundnut".into()),
            inspection_passed: passed,
            fulfilment: fulfil,
            on_time,
            upheld_farmer_fraud: fraud,
        }
    }

    fn farmer(id: &str, lots: Vec<LotFacts>) -> FarmerFacts {
        FarmerFacts {
            farmer_id: id.into(),
            label: id.into(),
            primary_crop: Some("groundnut".into()),
            lots,
        }
    }

    fn dataset(farmers: Vec<FarmerFacts>) -> Dataset {
        Dataset {
            farmers,
            ..Default::default()
        }
    }

    #[test]
    fn cold_start_farmer_scores_exactly_the_baseline() {
        let d = dataset(vec![
            farmer(
                "0xa",
                vec![lot("1", Some(true), Some(1.0), Some(true), false)],
            ),
            farmer("0xnew", vec![]),
        ]);
        let (pop, ranked) = score_all(&d, 4.0);
        let new = ranked.iter().find(|r| r.farmer_id == "0xnew").unwrap();
        assert_eq!(new.score, pop.baseline_score);
        assert_eq!(new.confidence, "none");
        assert!(new
            .features
            .iter()
            .all(|f| f.contribution == 0.0 && f.observed.trials == 0));
    }

    #[test]
    fn more_evidence_moves_further_from_baseline_and_narrows_uncertainty() {
        let good = |n: usize, prefix: &str| {
            (0..n)
                .map(|i| {
                    lot(
                        &format!("{prefix}{i}"),
                        Some(true),
                        Some(1.0),
                        Some(true),
                        false,
                    )
                })
                .collect::<Vec<_>>()
        };
        let bad = (0..10)
            .map(|i| lot(&format!("b{i}"), Some(false), Some(0.5), Some(false), false))
            .collect::<Vec<_>>();
        let d = dataset(vec![
            farmer("0x1", good(1, "x")),
            farmer("0x2", good(10, "y")),
            farmer("0x3", bad),
        ]);
        let (pop, ranked) = score_all(&d, 4.0);
        let one = ranked.iter().find(|r| r.farmer_id == "0x1").unwrap();
        let ten = ranked.iter().find(|r| r.farmer_id == "0x2").unwrap();
        assert!(ten.score > one.score && one.score > pop.baseline_score);
        assert!(ten.std_dev < one.std_dev);
        assert_eq!(ranked[0].farmer_id, "0x2");
        assert_eq!(ranked.last().unwrap().farmer_id, "0x3");
    }

    #[test]
    fn missing_features_fall_back_to_population_rate() {
        // Farmer 0x1 has inspections but no commitments or promises.
        let d = dataset(vec![
            farmer("0x1", vec![lot("1", Some(true), None, None, false)]),
            farmer(
                "0x2",
                vec![lot("2", Some(false), Some(0.8), Some(true), false)],
            ),
        ]);
        let (_, ranked) = score_all(&d, 4.0);
        let f1 = ranked.iter().find(|r| r.farmer_id == "0x1").unwrap();
        let fulfil = f1
            .features
            .iter()
            .find(|f| f.name == "fulfilment_rate")
            .unwrap();
        assert_eq!(fulfil.observed.trials, 0);
        assert_eq!(fulfil.estimate, fulfil.baseline);
        assert_eq!(fulfil.contribution, 0.0);
        assert!(fulfil.explanation.starts_with("No data yet"));
    }

    #[test]
    fn ties_break_stably_by_farmer_id() {
        let same = || vec![lot("l", Some(true), None, None, false)];
        let d = dataset(vec![
            farmer("0xc", same()),
            farmer("0xa", same()),
            farmer("0xb", same()),
        ]);
        let (_, ranked) = score_all(&d, 4.0);
        let order: Vec<_> = ranked.iter().map(|r| r.farmer_id.as_str()).collect();
        assert_eq!(order, ["0xa", "0xb", "0xc"]);
        assert_eq!(ranked.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn upheld_fraud_lowers_integrity_and_score() {
        let d = dataset(vec![
            farmer(
                "0x1",
                vec![
                    lot("1", Some(true), None, None, false),
                    lot("2", Some(true), None, None, false),
                ],
            ),
            farmer(
                "0x2",
                vec![
                    lot("3", Some(true), None, None, true),
                    lot("4", Some(true), None, None, false),
                ],
            ),
        ]);
        let (_, ranked) = score_all(&d, 4.0);
        assert_eq!(ranked[0].farmer_id, "0x1");
        let integ = ranked[1]
            .features
            .iter()
            .find(|f| f.name == "integrity_rate")
            .unwrap();
        assert_eq!(integ.observed.successes, 1.0);
        assert!(integ.contribution < 0.0);
    }

    #[test]
    fn one_lot_is_never_high_confidence_even_with_degenerate_population() {
        let d = dataset(vec![farmer(
            "0x1",
            vec![lot("1", Some(true), Some(1.0), Some(true), false)],
        )]);
        let (pop, ranked) = score_all(&d, 4.0);
        assert_eq!(ranked[0].confidence, "low");
        assert!(ranked[0].std_dev > 0.0, "prior variance must not collapse");
        assert!(pop.rates.values().all(|r| *r <= 0.95));
    }

    #[test]
    fn scoring_is_deterministic() {
        let d = dataset(vec![
            farmer(
                "0x1",
                vec![lot("1", Some(true), Some(0.9), Some(false), false)],
            ),
            farmer(
                "0x2",
                vec![lot("2", Some(false), Some(1.0), Some(true), false)],
            ),
        ]);
        let a = serde_json::to_string(&score_all(&d, 4.0).1).unwrap();
        let b = serde_json::to_string(&score_all(&d, 4.0).1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn contributions_sum_to_score_minus_baseline() {
        let d = dataset(vec![
            farmer(
                "0x1",
                vec![
                    lot("1", Some(true), Some(0.7), Some(false), false),
                    lot("2", Some(false), Some(1.0), Some(true), true),
                ],
            ),
            farmer(
                "0x2",
                vec![lot("3", Some(true), Some(1.0), Some(true), false)],
            ),
        ]);
        let (pop, ranked) = score_all(&d, 4.0);
        for r in ranked {
            let sum: f64 = r.features.iter().map(|f| f.contribution).sum();
            assert!(
                (r.score - pop.baseline_score - sum).abs() < 0.05,
                "{} vs {}",
                r.score - pop.baseline_score,
                sum
            );
        }
    }
}
