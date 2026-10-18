# Record integrity: serialization, hashing, CIDs and verification

Implemented in `backend/src/canonical.rs`, `cid.rs`, `crypto.rs`, `records.rs`, `verify.rs` and
`contracts/src/OilseedValueChain.sol`.

## 1. Record envelope

Every supply-chain record is a JSON object:

```json
{
  "schema": "oilschain.record.v1",
  "record_type": "procurement",
  "subject_id": "0x<32-byte batch id, or farmer id for registrations>",
  "chain_id": 31337,
  "registry": "0x<OilseedValueChain address>",
  "submitted_by": "0x<participant wallet>",
  "created_at": "2025-10-18T09:00:00Z",
  "nonce": "0x<16 random bytes>",
  "data": { "...type-specific, validated by the API..." }
}
```

`nonce` makes two otherwise identical events distinct and stops anyone from confirming a guessed plaintext
against the public digest by brute force. That matters most for low-entropy personal data.

Record types and their numeric codes are the same in the contract and the backend:
`1` farmer_registration, `2` procurement, `3` custody_transfer, `4` warehouse_inspection, `5` logistics,
`6` processing, `7` packaging, `8` fraud_report, `9` fraud_resolution, `10` ai_assessment.

## 2. Canonical serialization

`canonical_json(value)`:

* UTF-8 JSON with no insignificant whitespace.
* Object keys sorted by UTF-8 byte order, recursively.
* Strings escaped as by `serde_json`.
* Numbers in `serde_json`'s shortest round-trip form. Integers must satisfy `|n| <= 2^53` and floats must be
  finite; anything else is rejected.

This is close to, but **not**, RFC 8785 (JCS). Two differences: key order is by UTF-8 bytes rather than UTF-16
code units, and float formatting can differ from ECMAScript for extreme exponents. A third-party verifier in
another language must apply these exact rules. Verification is strict: the bytes fetched from IPFS must
already equal `canonical_json(parse(bytes))` (`parse_canonical`). Re-encoded or reformatted payloads fail as
`malformed_payload`.

## 3. The three identifiers

| Identifier | Definition | What it is for |
|---|---|---|
| `contentHash` | `keccak256(canonical_json(record))` over the **plaintext** | Commits to the record's meaning. It is the same whether the payload is stored encrypted or not. |
| `recordDigest` | `keccak256(abi.encode(RECORD_TYPEHASH, chainId, registry, uint8 recordType, bytes32 subjectId, contentHash))`, with `RECORD_TYPEHASH = keccak256("OilsChainRecord(uint256 chainId,address registry,uint8 recordType,bytes32 subjectId,bytes32 contentHash)")` | Domain-separated on-chain key. The contract computes it itself, so a commitment for one type, subject, chain or registry can never be replayed as another. |
| CID | CIDv1, codec `raw` (0x55), `sha2-256` over the **exact bytes stored**: canonical JSON, or the encryption envelope | Locator on IPFS. It is emitted in `RecordAnchored` and never written to contract storage. |

**Why keep both a CID and a contentHash?** The CID addresses the stored bytes, and for confidential records
those bytes are ciphertext. The contentHash addresses the plaintext meaning and is what the contract binds
to batch, stage and chain. Also, a CID is not something Solidity can recompute or domain-separate cheaply.
Because records are stored as single raw blocks (`block/put?cid-codec=raw&mhtype=sha2-256&pin=true`), the
CID is still computed locally before upload and checked against what the node returns and against every
fetched block. There is no dependence on Kubo's chunker or DAG settings.

## 4. Confidential fields (selective disclosure)

Each record type declares its private `data` fields (`RecordType::private_fields`). They are declared, not
detected: a field is public only if the schema says so.

| Record type | Private (encrypted) | Public |
|---|---|---|
| farmer_registration | `profile` (name, phone, village, land area, external farmer id), `wallet_address` | `label` (pseudonymous), `region` (state/district), `primary_crop`, `crops` |
| fraud_report | `description`, `evidence_cids` | `category`, `sku_id` |
| all other types | — | everything |

When a record has private fields:

```text
private          = {salt: 0x<16 random bytes>, <private fields>}
commitment       = keccak256(canonical_json(private))
committed record = {envelope…, data: {public fields}, private_commitment: commitment}
contentHash      = keccak256(canonical_json(committed record))          ← anchored on-chain
stored block     = committed record + {private_ciphertext: AES-256-GCM(canonical_json(private))}
                   AAD = "<record_type>:<subject_id>:<commitment>"
CID              = CIDv1(raw, sha256(canonical_json(stored block)))
```

`private_ciphertext` uses the envelope format
`{"schema":"oilschain.encrypted.v1","alg":"A256GCM","kid":"<sha256(key)[0..8]>","aad":…,"nonce":…,"ciphertext":…}`.

* **Anyone** can fetch the block, drop `private_ciphertext`, re-canonicalize and check `contentHash`. That
  verifies every public field and the commitment to the private part without a key.
* **The key holder** (this backend, `PII_ENCRYPTION_KEY`) can also decrypt the private part and check it
  against `private_commitment`. The salt stops anyone confirming a guessed name or phone number against the
  commitment.
* The verifier rejects a record whose public part contains a declared private field (`private_field_exposed`),
  a ciphertext that does not open to the commitment (`private_commitment_mismatch`), and a ciphertext bound to
  another record (`association_mismatch`).
* Keys never go to IPFS or the chain. Records written before this layout (whole record in one envelope) still
  verify through the legacy path.

Attachments uploaded with `/api/ipfs/upload` are public. Do not put personal data in public fields.

## 5. On-chain commitments

`OilseedValueChain` stores the following per record and batch:

* `anchors[recordDigest] = {subjectId, submitter, anchoredAt, recordType}`. Two slots; a duplicate digest is
  rejected.
* Per batch: `head = keccak256(abi.encode(head, recordDigest))` and `recordCount`. A processing output batch
  starts from `keccak256(abi.encode(0, processingDigest))` with `recordCount = 1`, which binds its history to
  its parent's processing record.
* `RecordAnchored(subjectId, recordType, recordDigest, contentHash, submitter, head, cid)` for every record.

## 6. Verification algorithm (`verify.rs`)

For a batch, walk `parentBatchId` up to the root (at most 8 levels). For each batch in that lineage:

1. Read every `RecordAnchored` event for the batch from the chain, in (block, logIndex) order. The chain is
   the source of truth; the database is not consulted for the timeline.
2. For each record:
   1. Recompute `recordDigest` from the event fields. Mismatch: `digest_mismatch`.
   2. Read `anchors[recordDigest]`; subject and type must match. Otherwise: `not_anchored`.
   3. Parse the CID (`malformed_cid`), fetch the block (`content_unavailable`), and check
      sha256(bytes) against the CID (`cid_mismatch`).
   4. Run the strict canonical parse of the stored block and remove `private_ciphertext` (`malformed_payload`).
   5. Check `keccak256(canonical(committed record)) == contentHash` (`hash_mismatch`).
   6. Check that the record's own `schema`, `record_type`, `subject_id`, `chain_id` and `registry` match
      (`association_mismatch`), and that no declared private field is public (`private_field_exposed`).
   7. If there is a `private_commitment`, decrypt the ciphertext (`decryption_failed`, `association_mismatch`)
      and check it against the commitment (`private_commitment_mismatch`).
3. Recompute the hash chain and compare it with the on-chain `head` and `recordCount`. This proves no record
   is missing, extra or reordered (`timeline_complete`).

The batch reports `verified: true` only if every record and every timeline passes. Every failure is listed
in `problems`. For SKUs, `verifyUnit` checks a unit's Merkle proof on-chain. Leaves are
`keccak256(bytes.concat(keccak256(abi.encode(skuId, keccak256(bytes(unitId))))))` and pairs are hashed in
sorted order, as in OpenZeppelin `MerkleProof`.

## 7. AI assessments (commit–reveal)

The commit is `keccak256(abi.encode(AI_COMMIT_TYPEHASH, batchId, contentHash, salt, oracle))`. The
assessment and salt are sealed in the operation journal (AES-GCM) and are **not** uploaded until reveal, so
nothing is public before the window opens. The reveal is accepted 1 h to 24 h after the commit, by the
committing oracle only. After reveal the assessment is an ordinary anchored record. An expired, unrevealed
commit can be replaced.
