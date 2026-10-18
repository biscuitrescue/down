# Farmer supply-reliability ranking (`farmer-reliability` / `bayes-shrinkage-v1`)

Implemented in `backend/src/ranking.rs`. Served at `GET /api/rankings/farmers` and
`GET /api/rankings/farmers/{farmer_id}`, which require a participant token.

## What it ranks and what it is not

* **Target:** farmers, ranked by how reliably their lots met agreed terms and passed inspection.
* **Intended use:** FPO procurement planning and deciding which farmers to prioritise for support.
* **Not for:** credit decisions, insurance eligibility or pricing, or excluding anyone without human review.
  This is operational reputation, not credit risk. The API repeats this in `intended_use` / `not_for`.
* **Kind:** a deterministic empirical-Bayes scoring model. It is **not** a trained machine-learning model.
  The repository has no labelled outcome data that could train or evaluate one, so no accuracy figure exists
  or is claimed. No LLM is involved, and explanations are generated from the numbers by templates.

## Evidence used

Only records that are confirmed on-chain and were submitted by *other* participants count. Each cached
record is re-hashed against its anchored `contentHash`; mismatches are excluded and counted in
`dataset.records_excluded`. Each raw lot (a farmer's procurement batch) counts **once** per feature, so
repeating events on one lot cannot inflate a score.

| Feature | Per lot | Weight |
|---|---|---|
| `quality_pass_rate` | latest warehouse inspection `passed` (lots with no inspection: no observation) | 0.35 |
| `fulfilment_rate` | `min(delivered_quantity_kg / committed_quantity_kg, 1)` when a commitment was recorded | 0.25 |
| `on_time_rate` | `delivery_date <= promised_delivery_date` when a promise was recorded | 0.20 |
| `integrity_rate` | 0 if an **upheld** fraud finding **attributed to the farmer** exists for the lot or its products, else 1 | 0.20 |

These are deliberately **not** features: volume, farm size, price, wealth, number of transactions, region,
and any personal attribute. Open or dismissed fraud reports are ignored. Upheld reports attributed to
another stage (packager, processor and so on) do not affect the farmer. An admin who is not the reporter
must resolve a report (enforced by the contract).

## Formula

For each feature *f*, with population rate `m_f = Σ successes / Σ trials` over all farmers (0.5 when there
is no data), clamped to [0.05, 0.95], and prior strength `k` (`RANKING_PRIOR_STRENGTH`, default 4):

```
estimate_f     = (s_f + k·m_f) / (n_f + k)
variance_f     = estimate_f·(1 − estimate_f) / (n_f + k + 1)
score          = 100 · Σ w_f · estimate_f
baseline       = 100 · Σ w_f · m_f              (score of a farmer with no history)
contribution_f = 100 · w_f · (estimate_f − m_f)  (Σ contributions = score − baseline)
sd             = 100 · √(Σ w_f² · variance_f)    (features treated as independent)
interval_90    = clamp(score ± 1.645·sd, 0, 100)
```

* **Cold start:** a new farmer scores exactly `baseline`, with `confidence: "none"`. They are neither
  rewarded nor penalised, and the explanation says so.
* **Sample size:** estimates move from the population rate toward the farmer's own rate as lots accumulate.
  `confidence` is `low` with fewer than 3 lots or sd ≥ 7, `medium` with fewer than 10 lots or sd ≥ 4, and
  `high` otherwise.
* **Sorting:** by `score` (default) or `lower_bound` (the conservative 90 % lower end). Ties break by
  `farmer_id`, so ordering is stable. `limit` (1–100), `offset`, `crop` and `min_evidence` filters are
  supported.
* **Reproducibility:** the response includes the model name/version, prior strength, weights and a
  `dataset.fingerprint` (sha256 over the model version and the sorted record digests and fraud states it
  used). The same dataset and config give the same scores (unit-tested).

## Response shape (abridged)

```json
{
  "model": {"name": "farmer-reliability", "version": "bayes-shrinkage-v1", "kind": "...", "prior_strength": 4, "weights": {...}},
  "intended_use": "...", "not_for": ["credit or loan decisions", "..."],
  "generated_at": "...", "dataset": {"records_considered": 12, "records_excluded": 0, "fingerprint": "0x..."},
  "population": {"baseline_score": 81.2, "rates": {...}, "farmers": 4},
  "total": 4, "offset": 0, "limit": 20,
  "items": [{"rank": 1, "farmer_id": "0x...", "label": "SFPO member 0042", "score": 84.1, "std_dev": 5.3,
             "interval_90": [75.4, 92.8], "confidence": "low", "evidence_lots": 2,
             "features": [{"name": "quality_pass_rate", "observed": {"successes": 2, "trials": 2},
                           "baseline": 0.75, "estimate": 0.83, "contribution": 2.9, "explanation": "..."}],
             "explanation": "..."}]
}
```

`observed` holds facts; `estimate`, `score` and the interval are model outputs.

## Known limitations

* A colluding FPO and warehouse could record fabricated facts. The model trusts the signed records it is
  given; on-chain attribution (`submitter`) is what makes such fraud traceable after the fact.
* Features assume independence when computing the interval.
* Weights are a documented judgement, not a learned result. Revisit them once outcome data, such as
  verified downstream quality complaints, exists.
