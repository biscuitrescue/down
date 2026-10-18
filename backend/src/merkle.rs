//! Merkle tree over retail units of a SKU, compatible with OpenZeppelin `MerkleProof` (sorted-pair hashing)
//! and `OilseedValueChain.unitLeaf` / `verifyUnit`.

use alloy::primitives::{keccak256, B256};
use alloy::sol_types::SolValue;

/// `keccak256(bytes.concat(keccak256(abi.encode(skuId, keccak256(bytes(unitId))))))`
pub fn unit_leaf(sku_id: B256, unit_id: &str) -> B256 {
    let inner = keccak256((sku_id, keccak256(unit_id.as_bytes())).abi_encode_params());
    keccak256(inner)
}

fn hash_pair(a: B256, b: B256) -> B256 {
    if a < b {
        keccak256((a, b).abi_encode_params())
    } else {
        keccak256((b, a).abi_encode_params())
    }
}

#[derive(Debug, Clone)]
pub struct UnitTree {
    /// levels[0] = sorted leaves, last level = [root]
    levels: Vec<Vec<B256>>,
}

impl UnitTree {
    /// Build the tree. Unit ids must be non-empty and unique.
    pub fn build(sku_id: B256, unit_ids: &[String]) -> Result<Self, String> {
        if unit_ids.is_empty() {
            return Err("at least one unit id is required".into());
        }
        let mut seen = std::collections::HashSet::new();
        for u in unit_ids {
            if u.trim().is_empty() || u.len() > 128 {
                return Err("unit ids must be 1-128 characters".into());
            }
            if !seen.insert(u) {
                return Err(format!("duplicate unit id `{u}`"));
            }
        }
        let mut leaves: Vec<B256> = unit_ids.iter().map(|u| unit_leaf(sku_id, u)).collect();
        leaves.sort();
        let mut levels = vec![leaves];
        while levels.last().expect("non-empty").len() > 1 {
            let prev = levels.last().expect("non-empty");
            let next = prev
                .chunks(2)
                .map(|pair| {
                    if pair.len() == 2 {
                        hash_pair(pair[0], pair[1])
                    } else {
                        pair[0]
                    }
                })
                .collect();
            levels.push(next);
        }
        Ok(Self { levels })
    }

    pub fn root(&self) -> B256 {
        self.levels.last().expect("non-empty")[0]
    }

    pub fn proof(&self, sku_id: B256, unit_id: &str) -> Option<Vec<B256>> {
        let leaf = unit_leaf(sku_id, unit_id);
        let mut index = self.levels[0].binary_search(&leaf).ok()?;
        let mut proof = Vec::new();
        for level in &self.levels[..self.levels.len() - 1] {
            let sibling = index ^ 1;
            if sibling < level.len() {
                proof.push(level[sibling]);
            }
            index /= 2;
        }
        Some(proof)
    }
}

/// Same computation as `MerkleProof.verify`.
pub fn verify(root: B256, leaf: B256, proof: &[B256]) -> bool {
    proof.iter().fold(leaf, |acc, p| hash_pair(acc, *p)) == root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_verify_for_every_unit() {
        let sku = B256::repeat_byte(3);
        for n in [1usize, 2, 3, 5, 8, 13] {
            let ids: Vec<String> = (0..n).map(|i| format!("unit-{i}")).collect();
            let tree = UnitTree::build(sku, &ids).unwrap();
            for id in &ids {
                let proof = tree.proof(sku, id).unwrap();
                assert!(
                    verify(tree.root(), unit_leaf(sku, id), &proof),
                    "n={n} id={id}"
                );
            }
            assert!(tree.proof(sku, "missing").is_none());
            assert!(!verify(tree.root(), unit_leaf(sku, "missing"), &[]));
        }
    }

    #[test]
    fn rejects_duplicates_and_empty() {
        let sku = B256::ZERO;
        assert!(UnitTree::build(sku, &[]).is_err());
        assert!(UnitTree::build(sku, &["a".into(), "a".into()]).is_err());
    }
}
