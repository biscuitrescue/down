import { api, ApiError, session, newIdempotencyKey, pollOperation } from "./api.js";

// ---------- tiny DOM helpers (all untrusted text goes through text nodes) ----------
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c === undefined || c === null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}
const app = document.getElementById("app");
const short = (s) => (typeof s === "string" && s.length > 18 ? `${s.slice(0, 10)}…${s.slice(-6)}` : s ?? "—");
const mono = (s, full = false) => h("span", { class: "mono", title: s || "" }, full ? s ?? "—" : short(s));
const fmtTime = (t) => (t ? new Date(typeof t === "number" ? t * 1000 : t).toLocaleString() : "—");
function badge(text, kind) { return h("span", { class: `badge ${kind}` }, text); }
function statusBadge(status) {
  const map = { confirmed: "ok", verified: "ok", revealed: "ok", minted: "ok", completed: "ok", submitted: "warn", stored: "warn", prepared: "warn", in_progress: "warn", committed: "warn", pending_commit: "warn" };
  return badge(status.replaceAll("_", " "), map[status] || "bad");
}
function notice(kind, ...children) { return h("div", { class: `notice ${kind}` }, ...children); }
function errorBox(e) {
  const msg = e instanceof ApiError ? `${e.status ? `HTTP ${e.status}: ` : ""}${e.message}` : String(e);
  return notice("bad", msg);
}
function loading(text = "Loading…") { return h("p", { class: "muted" }, text); }
function render(...nodes) { app.replaceChildren(...nodes); }
function params() {
  const q = location.hash.split("?")[1] || "";
  return Object.fromEntries(new URLSearchParams(q));
}

let config = null;
let me = null;

async function loadSession() {
  const box = document.getElementById("session");
  me = null;
  if (!session.token) { box.replaceChildren("Not signed in"); return; }
  try {
    me = (await api("GET", "/api/me")).data;
    box.replaceChildren(`${me.name} · ${me.is_admin ? "admin" : me.roles.join(", ")}`);
  } catch (e) {
    box.replaceChildren(h("span", { class: "badge bad" }, "token rejected"));
  }
}

// ---------- TRACE (public) ----------
function recordItem(r) {
  return h("li", { class: r.verified ? "" : "failed" },
    h("div", { class: "row" },
      h("strong", {}, r.record_type.replaceAll("_", " ")),
      r.verified ? badge("verified", "ok") : badge(r.status.replaceAll("_", " "), "bad"),
      r.encrypted ? badge(r.private_verified ? "private part verified" : "encrypted", "warn") : null,
      (r.replicas || []).length ? badge(`${r.replicas.filter((x) => x.status === "pinned").length}/${r.replicas.length} backups`, r.replicas.every((x) => x.status === "pinned") ? "ok" : "warn") : null,
      h("span", { class: "muted" }, `by ${r.submitter_name || short(r.submitter)} · ${fmtTime(r.created_at || r.anchored_at)}`)),
    r.detail ? h("div", { class: "muted" }, r.detail) : null,
    h("details", {},
      h("summary", { class: "muted" }, "evidence"),
      h("div", {}, "record digest ", mono(r.record_digest, true)),
      h("div", {}, "content hash ", mono(r.content_hash, true)),
      h("div", {}, "tx ", mono(r.tx_hash, true), ` · block ${r.block_number ?? "—"}`),
      h("div", {}, "IPFS ", h("a", { href: r.ipfs_url, target: "_blank", rel: "noopener" }, r.cid), r.cid_checked ? " (CID checked)" : " (CID not locally checkable)"),
      r.data ? h("pre", {}, JSON.stringify(r.data, null, 2)) : r.encrypted ? h("p", { class: "muted" }, "Confidential record: verified with the server key, contents not disclosed.") : null,
      (r.private_fields || []).length ? h("p", { class: "muted" }, `Encrypted (not shown): ${r.private_fields.join(", ")}`) : null,
      (r.replicas || []).map((x) => h("div", { class: "muted" }, `backup ${x.target}: ${x.status}${x.verified_at ? ` (verified ${fmtTime(x.verified_at)})` : ""}`))));
}

function provenanceView(p) {
  return h("div", {},
    h("div", { class: "panel" },
      h("div", { class: "row" }, h("h2", {}, "Provenance"), p.verified ? badge("all records verified", "ok") : badge("verification failed", "bad")),
      h("p", { class: "muted" }, `Checked ${fmtTime(p.verified_at)} against chain ${p.chain_id}, registry `, mono(p.registry)),
      p.problems.length ? notice("bad", h("strong", {}, "Problems"), h("ul", {}, p.problems.map((x) => h("li", {}, x)))) : null,
      h("h3", {}, "Farmer"),
      p.farmer
        ? h("p", {}, mono(p.lineage[0]?.farmer_id), " registration ", p.farmer.verified ? badge("verified", "ok") : badge(p.farmer.status, "bad"), h("span", { class: "muted" }, " (identity kept off-chain, encrypted)"))
        : h("p", { class: "muted" }, "No registration record found.")),
    p.lineage.map((b) => h("div", { class: "panel" },
      h("div", { class: "row" },
        h("h2", {}, b.label || short(b.batch_id)),
        badge(b.stage.replaceAll("_", " "), "warn"),
        b.timeline_complete ? badge("timeline complete", "ok") : badge("timeline incomplete", "bad")),
      h("p", { class: "muted" }, "batch ", mono(b.batch_id, true), b.parent_batch_id ? h("span", {}, " · from ", mono(b.parent_batch_id)) : null, ` · custodian ${b.custodian_name || short(b.custodian)}`),
      h("p", { class: "muted" }, `on-chain head ${short(b.onchain_head)} (${b.onchain_record_count} records) · recomputed ${short(b.recomputed_head)} (${b.recomputed_record_count})`),
      h("ul", { class: "timeline" }, b.records.map(recordItem)))));
}

async function traceView() {
  const q = params();
  const id = h("input", { type: "text", placeholder: "0x… batch or SKU id", value: q.sku || q.batch || "" });
  const kind = h("select", {}, h("option", { value: "sku" }, "SKU (from QR)"), h("option", { value: "batch" }, "Batch"));
  if (q.batch) kind.value = "batch";
  const unit = h("input", { type: "text", placeholder: "unit id (optional)", value: q.unit || "" });
  const out = h("div");
  const go = async (ev) => {
    ev && ev.preventDefault();
    const v = id.value.trim();
    if (!v) return;
    out.replaceChildren(loading("Fetching records from IPFS and checking them against the chain…"));
    try {
      if (kind.value === "sku") {
        const qs = unit.value.trim() ? `?unit_id=${encodeURIComponent(unit.value.trim())}` : "";
        const { data } = await api("GET", `/api/skus/${encodeURIComponent(v)}/verify${qs}`, { auth: false });
        out.replaceChildren(
          h("div", { class: "panel" },
            h("div", { class: "row" }, h("h2", {}, data.label || "SKU"), data.verified ? badge("verified", "ok") : badge("not verified", "bad")),
            h("p", { class: "muted" }, "SKU ", mono(data.sku_id, true), ` · packaged ${fmtTime(data.packaged_at)}`),
            data.unit ? h("p", {}, `Unit ${data.unit.unit_id}: `, data.unit.included === true ? badge("part of this SKU (on-chain Merkle proof)", "ok") : badge(data.unit.detail || "not part of this SKU", "bad")) : null),
          provenanceView(data.provenance));
      } else {
        const { data } = await api("GET", `/api/batches/${encodeURIComponent(v)}/provenance`, { auth: false });
        out.replaceChildren(provenanceView(data));
      }
    } catch (e) { out.replaceChildren(errorBox(e)); }
  };
  render(
    h("h1", {}, "Trace a product"),
    h("p", { class: "muted" }, "Every record is re-downloaded from IPFS, re-hashed and matched against its on-chain commitment. Nothing is shown as verified unless all checks passed."),
    h("form", { class: "panel row", onsubmit: go }, h("label", {}, "Look up", kind), h("label", {}, "Id", id), h("label", {}, "Unit", unit), h("button", {}, "Verify")),
    out);
  if (id.value) go();
}

// ---------- CERTIFICATES (NFTs, public) ----------
async function nftView() {
  const kind = h("select", {}, h("option", { value: "provenance" }, "Batch provenance NFT"), h("option", { value: "credential" }, "Farmer credential (soulbound)"));
  const id = h("input", { type: "number", min: "1", value: params().token || "1" });
  const out = h("div");
  const go = async (ev) => {
    ev && ev.preventDefault();
    out.replaceChildren(loading());
    try {
      const { data: t } = await api("GET", `/api/nft/${kind.value}/${encodeURIComponent(id.value)}`, { auth: false });
      out.replaceChildren(h("div", { class: "panel" },
        h("div", { class: "row" }, h("h2", {}, t.metadata?.name || `Token #${t.token_id}`), t.metadata_verified ? badge("metadata verified", "ok") : badge(t.metadata_problem || "metadata not verified", "bad"), t.soulbound ? badge("soulbound", "warn") : null),
        h("p", {}, "Owner ", mono(t.owner, true), t.owner_name ? ` (${t.owner_name})` : "",
          t.owner_is_custodian === false ? h("span", {}, " · ", badge("custody has moved; anyone can sync the token to the custodian", "warn")) : null),
        h("p", { class: "muted" }, "tokenURI ", mono(t.token_uri, true), " · contract ", mono(t.contract_address), " · mint tx ", mono(t.mint_tx_hash)),
        t.metadata?.description ? h("p", {}, t.metadata.description) : null,
        t.source_record ? h("div", {}, h("h3", {}, "Source record"), h("ul", { class: "timeline" }, recordItem(t.source_record))) : null,
        t.metadata ? h("details", {}, h("summary", {}, "metadata JSON"), h("pre", {}, JSON.stringify(t.metadata, null, 2))) : null));
    } catch (e) { out.replaceChildren(errorBox(e)); }
  };
  render(h("h1", {}, "Certificates"),
    h("p", { class: "muted" }, "A token links to verified on-chain records. Owning it is not proof that every real-world claim is true."),
    h("form", { class: "panel row", onsubmit: go }, h("label", {}, "Kind", kind), h("label", {}, "Token id", id), h("button", {}, "Look up")), out);
}

// ---------- RANKINGS ----------
let rankingState = { offset: 0, limit: 20, crop: "", sort: "score", minEvidence: "" };
async function rankingsView() {
  if (!session.token) return render(h("h1", {}, "Rankings"), notice("warn", "Rankings are visible to registered participants. Sign in under Workspace."));
  const crop = h("select", {}, h("option", { value: "" }, "all crops"), (config?.crops || []).map((c) => h("option", { value: c }, c)));
  crop.value = rankingState.crop;
  const sort = h("select", {}, h("option", { value: "score" }, "score"), h("option", { value: "lower_bound" }, "conservative (90% lower bound)"));
  sort.value = rankingState.sort;
  const minEv = h("input", { type: "number", min: "0", value: rankingState.minEvidence, placeholder: "0" });
  const out = h("div", {}, loading());
  let loadedAt = 0;
  const staleNote = h("span", { class: "muted" });
  setInterval(() => { if (loadedAt && Date.now() - loadedAt > 5 * 60 * 1000) staleNote.replaceChildren(badge("stale — refresh to recompute", "warn")); }, 30000);

  const load = async () => {
    rankingState = { ...rankingState, crop: crop.value, sort: sort.value, minEvidence: minEv.value };
    const qs = new URLSearchParams({ limit: rankingState.limit, offset: rankingState.offset, sort: rankingState.sort });
    if (rankingState.crop) qs.set("crop", rankingState.crop);
    if (rankingState.minEvidence) qs.set("min_evidence", rankingState.minEvidence);
    out.replaceChildren(loading("Computing from verified records…"));
    try {
      const { data: r } = await api("GET", `/api/rankings/farmers?${qs}`);
      loadedAt = Date.now();
      staleNote.replaceChildren(`computed ${fmtTime(r.generated_at)}`);
      if (!r.items.length) return out.replaceChildren(notice("warn", "No farmers match these filters yet."));
      const rows = r.items.map((f) => {
        const lo = f.interval_90[0], hi = f.interval_90[1];
        return [
          h("tr", {},
            h("td", {}, String(f.rank)),
            h("td", {}, f.label, h("div", { class: "muted mono" }, short(f.farmer_id))),
            h("td", {}, f.score.toFixed(1)),
            h("td", {}, h("div", { class: "bar", title: `90% interval ${lo}–${hi}` }, h("span", { style: `left:${lo}%;right:${100 - hi}%` }), h("i", { style: `left:${f.score}%` })), h("small", { class: "muted" }, `${lo.toFixed(1)}–${hi.toFixed(1)}`)),
            h("td", {}, badge(f.confidence, f.confidence === "high" ? "ok" : f.confidence === "none" ? "bad" : "warn")),
            h("td", {}, String(f.evidence_lots))),
          h("tr", {}, h("td", { colspan: "6" }, h("details", {}, h("summary", { class: "muted" }, f.explanation),
            h("table", {}, h("tr", {}, ["feature", "observed", "estimate", "baseline", "points", "why"].map((x) => h("th", {}, x))),
              f.features.map((ft) => h("tr", {},
                h("td", {}, ft.name.replaceAll("_", " ")),
                h("td", {}, ft.observed.trials ? `${ft.observed.successes}/${ft.observed.trials}` : "—"),
                h("td", {}, ft.estimate.toFixed(2)),
                h("td", {}, ft.baseline.toFixed(2)),
                h("td", {}, (ft.contribution > 0 ? "+" : "") + ft.contribution.toFixed(1)),
                h("td", {}, ft.explanation))))))),
        ];
      });
      out.replaceChildren(
        h("div", { class: "panel table-wrap" }, h("table", {}, h("tr", {}, ["#", "farmer", "score", "uncertainty", "confidence", "lots"].map((x) => h("th", {}, x))), rows.flat())),
        h("div", { class: "row" },
          h("button", { class: "secondary", disabled: r.offset === 0, onclick: () => { rankingState.offset = Math.max(0, r.offset - r.limit); load(); } }, "← previous"),
          h("span", { class: "muted" }, `${r.offset + 1}–${r.offset + r.items.length} of ${r.total}`),
          h("button", { class: "secondary", disabled: r.offset + r.limit >= r.total, onclick: () => { rankingState.offset = r.offset + r.limit; load(); } }, "next →")),
        h("div", { class: "panel" },
          h("p", {}, h("strong", {}, `${r.model.name} ${r.model.version}: `), r.model.kind, `. Prior strength ${r.model.prior_strength}; weights `, Object.entries(r.model.weights).map(([k, v]) => `${k} ${v}`).join(", "), "."),
          h("p", {}, h("strong", {}, "Intended use: "), r.intended_use, " ", h("strong", {}, "Not for: "), r.not_for.join("; "), "."),
          h("p", { class: "muted" }, `Cold-start baseline ${r.population.baseline_score} · ${r.dataset.records_considered} records considered, ${r.dataset.records_excluded} excluded · dataset `, mono(r.dataset.fingerprint))));
    } catch (e) { out.replaceChildren(errorBox(e)); }
  };
  render(h("h1", {}, "Farmer supply-reliability ranking"),
    h("p", { class: "muted" }, "Computed only from records anchored by FPOs, warehouses and fraud reviewers. New farmers start at the population baseline. This is not a credit score."),
    h("div", { class: "panel row" }, h("label", {}, "Crop", crop), h("label", {}, "Sort", sort), h("label", {}, "Min. lots", minEv),
      h("button", { onclick: () => { rankingState.offset = 0; load(); } }, "Refresh"), staleNote),
    out);
  load();
}

// ---------- ADVISORY ----------
async function advisoryView() {
  render(h("h1", {}, "Crop advisory"), loading());
  let status;
  try { status = (await api("GET", "/api/advisory/status", { auth: false })).data; } catch (e) { return render(h("h1", {}, "Crop advisory"), errorBox(e)); }
  if (!status.configured || !status.reachable || status.model_available === false) {
    return render(h("h1", {}, "Crop advisory"), notice("warn",
      h("strong", {}, "The offline advisory model is not available on this server. "), status.detail || "",
      h("p", {}, "Setup: install Ollama on this machine, run ", h("code", {}, `ollama pull ${status.model}`), ", then set ", h("code", {}, "ADVISORY_BASE_URL=http://127.0.0.1:11434"), " and restart the backend. Inference stays local; no external API is used.")));
  }
  if (!session.token) return render(h("h1", {}, "Crop advisory"), notice("warn", "Sign in under Workspace to ask a question."));
  const q = h("textarea", { placeholder: "e.g. How should I store groundnut pods to avoid aflatoxin?", style: "min-height:90px" });
  const lang = h("select", {}, status.languages.map((l) => h("option", { value: l }, l)));
  const farmer = h("input", { type: "text", placeholder: "farmer id (optional, adds verified history)" });
  const crop = h("select", {}, h("option", { value: "" }, "crop (optional)"), (config?.crops || []).map((c) => h("option", { value: c }, c)));
  const out = h("div");
  const ask = async (ev) => {
    ev.preventDefault();
    out.replaceChildren(loading("Asking the local model…"));
    try {
      const body = { question: q.value, language: lang.value };
      if (farmer.value.trim()) body.farmer_id = farmer.value.trim();
      if (crop.value) body.crop = crop.value;
      const { data } = await api("POST", "/api/advisory", { body });
      out.replaceChildren(h("div", { class: "panel" },
        h("p", { style: "white-space:pre-wrap" }, data.answer),
        h("h3", {}, "Grounded on (verified facts)"), data.grounded_on.length ? h("ul", {}, data.grounded_on.map((f) => h("li", {}, f))) : h("p", { class: "muted" }, "No farmer-specific facts."),
        notice("warn", data.disclaimer),
        h("p", { class: "muted" }, `${data.model} · ${fmtTime(data.generated_at)}`)));
    } catch (e) { out.replaceChildren(errorBox(e)); }
  };
  render(h("h1", {}, "Crop advisory"), h("p", { class: "muted" }, `Model ${status.model}, running locally. General guidance only.`),
    h("form", { class: "panel", onsubmit: ask }, q, h("div", { class: "row" }, h("label", {}, "Language", lang), h("label", {}, "Crop", crop), h("label", {}, "Farmer", farmer), h("button", {}, "Ask"))), out);
}

// ---------- WORKSPACE (participants) ----------
const today = () => new Date().toISOString().slice(0, 10);
const nowIso = () => new Date().toISOString().replace(/\.\d+Z$/, "Z");
const ACTIONS = [
  { id: "register", role: "fpo", title: "Register farmer", path: "/api/farmer/register", template: () => ({ label: "FPO member 0001", region: { state: "Maharashtra", district: "Latur" }, primary_crop: "soybean", profile: { name: "", phone: "", village: "", land_area_ha: 1.0, crops: ["soybean"] } }) },
  { id: "purchase", role: "fpo", title: "Record procurement (creates batch)", path: "/api/fpo/purchase", template: () => ({ farmer_id: "0x…", label: "LOT-001", crop: "soybean", season: "kharif", delivered_quantity_kg: 1000, committed_quantity_kg: 1000, promised_delivery_date: today(), delivery_date: today(), price_per_quintal_inr: 4892, moisture_pct: 10 }) },
  { id: "mint", role: "fpo", title: "Mint provenance NFT from procurement tx", path: "/api/nft/mint-from-tx", template: () => ({ tx_hash: "0x…" }) },
  { id: "credential", role: "fpo", title: "Issue soulbound farmer credential", path: "/api/nft/credential", template: () => ({ farmer_id: "0x…" }) },
  { id: "custody", role: "custodian", title: "Transfer custody", path: "/api/custody/transfer", template: () => ({ batch_id: "0x…", to_participant_id: "p_…", quantity_kg: 1000 }) },
  { id: "inspect", role: "warehouse", title: "Warehouse inspection", path: "/api/warehouse/inspection", template: () => ({ batch_id: "0x…", warehouse_ref: "WH-1", inspection: { passed: true, grade: "FAQ", moisture_pct: 9.5, foreign_matter_pct: 1, oil_content_pct: 18, inspected_at: nowIso() }, storage: { temperature_c: 27, humidity_pct: 60 } }) },
  { id: "logistics", role: "logistics", title: "Logistics milestone", path: "/api/logistics/record", template: () => ({ batch_id: "0x…", shipment_ref: "SHP-1", delivered: false, checkpoint: { name: "Dispatched", at: nowIso() } }) },
  { id: "process", role: "processor", title: "Process batch", path: "/api/processing/batch", template: () => ({ input_batch_id: "0x…", method: "expeller pressing", outputs: [{ label: "OIL-001", product: "crude_oil", quantity_kg: 180 }, { label: "CAKE-001", product: "oilcake", quantity_kg: 800 }], lab_report: { oil_yield_pct: 18 } }) },
  { id: "package", role: "packager", title: "Create SKU", path: "/api/packaging/sku", template: () => ({ batch_id: "0x… (product batch)", label: "Oil 1L", unit_ids: ["U-0001", "U-0002"], package: { pack_size: "1 L" } }) },
  { id: "fraud", role: "any", title: "Report fraud", path: "/api/fraud/report", template: () => ({ sku_id: "0x…", category: "adulteration", description: "", evidence_cids: [] }) },
  { id: "ai_commit", role: "ai_oracle", title: "Commit AI assessment", path: "/api/ai/commit", template: () => ({ batch_id: "0x…", assessment: { model_id: "", model_version: "", method: "", outputs: {} } }) },
  { id: "ai_reveal", role: "ai_oracle", title: "Reveal AI assessment", path: "/api/ai/reveal", template: () => ({ batch_id: "0x…" }) },
];
const CUSTODIAN = ["fpo", "warehouse", "logistics", "processor", "packager", "retailer"];
function allowed(a) {
  if (!me) return false;
  if (me.is_admin) return true;
  if (a.role === "any") return me.roles.length > 0;
  if (a.role === "custodian") return me.roles.some((r) => CUSTODIAN.includes(r));
  return me.roles.includes(a.role);
}

function operationPanel(op) {
  const box = h("div", { class: "panel" });
  const draw = (o) => box.replaceChildren(
    h("div", { class: "row" }, h("strong", {}, o.kind.replaceAll("_", " ")), statusBadge(o.status), o.retryable ? h("span", { class: "muted" }, "in progress — the server keeps checking; you can also retry with the same key") : null),
    o.error ? notice("bad", o.error) : null,
    h("div", {}, "operation ", mono(o.operation_id, true)),
    o.tx_hash ? h("div", {}, "tx ", mono(o.tx_hash, true), o.status === "confirmed" ? ` · block ${o.block_number}` : " (submitted, not confirmed)") : null,
    o.cid ? h("div", {}, "IPFS ", mono(o.cid, true), o.encrypted ? " (encrypted)" : "") : null,
    o.status === "confirmed" && o.result ? h("pre", {}, JSON.stringify(o.result, null, 2)) : null);
  draw(op);
  if (op.retryable) pollOperation(op.operation_id, draw).catch((e) => box.append(errorBox(e)));
  return box;
}

async function referenceLists() {
  const wrap = h("details", { class: "panel" }, h("summary", {}, "Reference: participants, farmers, batches"));
  try {
    const [p, f, b] = await Promise.all([api("GET", "/api/participants"), api("GET", "/api/farmers"), api("GET", "/api/batches")]);
    wrap.append(
      h("h3", {}, "Participants"), h("div", { class: "table-wrap" }, h("table", {}, p.data.map((x) => h("tr", {}, h("td", {}, x.name), h("td", {}, x.roles.join(", ")), h("td", {}, mono(x.id, true)))))),
      h("h3", {}, "Farmers"), f.data.length ? h("div", { class: "table-wrap" }, h("table", {}, f.data.map((x) => h("tr", {}, h("td", {}, x.label), h("td", {}, x.primary_crop || ""), h("td", {}, mono(x.farmer_id, true)))))) : h("p", { class: "muted" }, "none yet"),
      h("h3", {}, "Batches"), b.data.length ? h("div", { class: "table-wrap" }, h("table", {}, b.data.map((x) => h("tr", {}, h("td", {}, x.label), h("td", {}, x.parent_batch_id ? "product" : "raw lot"), h("td", {}, mono(x.batch_id, true)), h("td", {}, h("a", { href: `#/trace?batch=${x.batch_id}` }, "trace")))))) : h("p", { class: "muted" }, "none yet"));
  } catch (e) { wrap.append(errorBox(e)); }
  return wrap;
}

async function workspaceView() {
  const token = h("input", { type: "password", placeholder: "API token (participant or admin)", value: "" });
  const login = h("form", { class: "panel row", onsubmit: async (ev) => { ev.preventDefault(); session.token = token.value.trim(); await loadSession(); workspaceView(); } },
    h("label", {}, "Sign in", token), h("button", {}, "Sign in"),
    session.token ? h("button", { type: "button", class: "secondary", onclick: async () => { session.token = ""; await loadSession(); workspaceView(); } }, "Sign out") : null);
  if (!me) return render(h("h1", {}, "Workspace"), login, session.token ? notice("bad", "Token rejected.") : null);

  const actions = ACTIONS.filter(allowed);
  const select = h("select", {}, actions.map((a) => h("option", { value: a.id }, a.title)));
  const editor = h("textarea", { spellcheck: "false" });
  const out = h("div");
  const setTemplate = () => { const a = actions.find((x) => x.id === select.value); editor.value = JSON.stringify(a.template(), null, 2); };
  select.addEventListener("change", setTemplate);
  if (actions.length) setTemplate();
  let idem = newIdempotencyKey();
  const submit = async (ev) => {
    ev.preventDefault();
    const a = actions.find((x) => x.id === select.value);
    let body;
    try { body = JSON.parse(editor.value); } catch (e) { return out.prepend(notice("bad", `Invalid JSON: ${e.message}`)); }
    const btn = ev.submitter; btn && (btn.disabled = true);
    try {
      const { data } = await api("POST", a.path, { body, idem });
      out.prepend(operationPanel(data));
      idem = newIdempotencyKey();
    } catch (e) {
      out.prepend(errorBox(e));
      // Keep the same key so a retry after a transient error resumes the same operation.
    } finally { btn && (btn.disabled = false); }
  };
  render(h("h1", {}, "Workspace"), login,
    h("div", { class: "panel" }, h("p", {}, `Signed in as ${me.name}. Roles on-chain: `, me.onchain_roles.join(", ") || "none yet", ". Wallet ", mono(me.address, true))),
    actions.length
      ? h("form", { class: "panel", onsubmit: submit }, h("div", { class: "row" }, h("label", {}, "Action", select), h("button", {}, "Submit")), editor,
        h("p", { class: "muted" }, "Requests are idempotent: a failed network call can be retried safely. Results show as confirmed only after the transaction is mined."))
      : notice("warn", "Your roles do not allow any write actions."),
    out, await referenceLists());
}

// ---------- ADMIN ----------
async function adminView() {
  if (!me?.is_admin) return render(h("h1", {}, "Admin"), notice("warn", "Sign in with the admin token under Workspace."));
  const name = h("input", { type: "text", placeholder: "Participant name" });
  const roleBoxes = ["fpo", "warehouse", "logistics", "processor", "packager", "ai_oracle", "retailer"].map((r) => h("label", { style: "flex-direction:row;align-items:center" }, h("input", { type: "checkbox", value: r }), r));
  const out = h("div");
  const create = async (ev) => {
    ev.preventDefault();
    const roles = roleBoxes.map((l) => l.querySelector("input")).filter((i) => i.checked).map((i) => i.value);
    try {
      const { data } = await api("POST", "/api/admin/participants", { body: { name: name.value, roles } });
      out.prepend(h("div", { class: "panel" },
        h("p", {}, `Created ${data.participant.name} (${data.participant.roles.join(", ")}) · wallet `, mono(data.participant.address, true)),
        notice("warn", h("strong", {}, "API token (shown once): "), mono(data.api_token, true)),
        data.operations.map(operationPanel)));
    } catch (e) { out.prepend(errorBox(e)); }
  };
  const resolve = h("textarea", { style: "min-height:120px" });
  resolve.value = JSON.stringify({ report_digest: "0x…", upheld: false, attribution: "unattributed", notes: "" }, null, 2);
  const wf = h("textarea");
  wf.value = JSON.stringify({ workflow_key: "demo-1", participants: { fpo: "p_…", warehouse: "p_…", logistics: "p_…", processor: "p_…" }, farmer_id: "0x…", procurement: { label: "LOT-DEMO", crop: "groundnut", delivered_quantity_kg: 500, delivery_date: today() }, warehouse: { warehouse_ref: "WH-1", inspection: { passed: true, inspected_at: nowIso() } }, logistics: [{ shipment_ref: "SHP-1", delivered: true, checkpoint: { name: "mill", at: nowIso() } }], processing: { method: "expeller", outputs: [{ label: "OIL-DEMO", product: "crude_oil", quantity_kg: 200 }] }, packaging: { label: "Oil 5L", unit_ids: ["D-1", "D-2"] } }, null, 2);
  const postJson = (path, area) => async (ev) => {
    ev.preventDefault();
    try {
      const { data } = await api("POST", path, { body: JSON.parse(area.value), idem: newIdempotencyKey() });
      out.prepend(data.operation_id ? operationPanel(data) : h("div", { class: "panel" }, statusBadge(data.status), h("pre", {}, JSON.stringify(data, null, 2))));
    } catch (e) { out.prepend(errorBox(e)); }
  };
  render(h("h1", {}, "Admin"),
    h("form", { class: "panel", onsubmit: create }, h("h2", {}, "Onboard participant"), h("div", { class: "row" }, h("label", {}, "Name", name), roleBoxes, h("button", {}, "Create"))),
    h("form", { class: "panel", onsubmit: postJson("/api/fraud/resolve", resolve) }, h("h2", {}, "Resolve fraud report"), h("p", { class: "muted" }, "Only upheld reports attributed to the farmer affect farmer rankings. You cannot resolve a report you filed."), resolve, h("button", {}, "Resolve")),
    h("form", { class: "panel", onsubmit: postJson("/api/workflow/execute", wf) }, h("h2", {}, "Run end-to-end workflow"), h("p", { class: "muted" }, "Not atomic: each stage is a separate operation. Re-run with the same workflow_key to resume."), wf, h("button", {}, "Execute")),
    out);
}

// ---------- router ----------
const routes = { trace: traceView, nft: nftView, rankings: rankingsView, advisory: advisoryView, workspace: workspaceView, admin: adminView };
async function route() {
  const name = (location.hash.replace(/^#\//, "").split("?")[0]) || "trace";
  document.querySelectorAll("#nav a").forEach((a) => a.classList.toggle("active", a.getAttribute("href") === `#/${name}`));
  try { await (routes[name] || traceView)(); } catch (e) { render(errorBox(e)); }
}
window.addEventListener("hashchange", route);

(async () => {
  try {
    config = (await api("GET", "/api/config", { auth: false })).data;
    document.getElementById("foot").replaceChildren(`Chain ${config.chain_id} · registry `, mono(config.registry), ` · ${config.record_schema} · ${config.ranking_model}`);
  } catch (e) {
    document.getElementById("foot").replaceChildren(errorBox(e));
  }
  await loadSession();
  route();
})();
