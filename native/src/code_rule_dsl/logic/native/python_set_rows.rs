//! Batched legacy string-set order, without a Python runtime. The contract is
//! CPython 3.13 on a 64-bit little-endian host with PYTHONHASHSEED=0, monotone
//! `set.add` only. No deletion, set-literal/update preallocation, or arbitrary keys.
//! Algorithm references: CPython v3.13.18 Objects/setobject.c and Python/pyhash.c.
use siphasher::sip::SipHasher13;
use std::hash::Hasher;

fn hash(value: &str) -> u64 {
    if value.is_empty() { return 0; }
    // CPython hashes its canonical 1/2/4-byte Unicode representation, not UTF-8.
    let maximum = value.chars().map(u32::from).max().unwrap();
    let mut bytes = Vec::with_capacity(value.len());
    if maximum <= 255 {
        bytes.extend(value.chars().map(|c| c as u8));
    } else if maximum <= 65535 {
        for c in value.chars() { bytes.extend_from_slice(&(c as u16).to_le_bytes()); }
    } else {
        for c in value.chars() { bytes.extend_from_slice(&(c as u32).to_le_bytes()); }
    }
    let mut hasher = SipHasher13::new_with_keys(0, 0);
    hasher.write(&bytes);
    match hasher.finish() { u64::MAX => u64::MAX - 1, h => h }
}

/// Output indices refer to the first insertion of each distinct string.
/// Input rows are already sorted by insertion ordinal (text breaks ordinal ties).
pub(in crate::code_rule_dsl) fn order(values: &[&str]) -> Vec<usize> {
    let _profile = super::super::profile::span(|| "legacy string-set order".into());
    let mut table: Vec<Option<(u64, usize)>> = vec![None; 8];
    let mut used = 0;
    for (index, value) in values.iter().enumerate() {
        let h = hash(value);
        let mask = table.len() - 1;
        let mut slot = h as usize & mask;
        let mut perturb = h;
        let inserted = 'search: loop {
            let probes = if slot + 9 <= mask { 9 } else { 0 };
            for at in slot..=slot + probes {
                match table[at] {
                    Some((other_hash, other)) if other_hash == h && values[other] == *value => break 'search false,
                    None => { table[at] = Some((h, index)); break 'search true; }
                    _ => (),
                }
            }
            perturb >>= 5;
            slot = slot.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize) & mask;
        };
        if !inserted { continue; }
        used += 1;
        if used * 5 < mask * 3 { continue; }
        let minimum = used * if used > 50000 { 2 } else { 4 };
        let mut size = 8;
        while size <= minimum { size *= 2; }
        let mut next = vec![None; size];
        let mask = size - 1;
        // Resize reinserts in *old table slot order*, not insertion order.
        for entry in table.into_iter().flatten() {
            let mut slot = entry.0 as usize & mask;
            let mut perturb = entry.0;
            'clean: loop {
                let probes = if slot + 9 <= mask { 9 } else { 0 };
                for at in slot..=slot + probes {
                    if next[at].is_none() { next[at] = Some(entry); break 'clean; }
                }
                perturb >>= 5;
                slot = slot.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize) & mask;
            }
        }
        table = next;
    }
    table.into_iter().flatten().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::{hash, order};
    use sha2::{Digest, Sha256};

    #[test]
    fn unicode_hashes_match_the_pinned_python_oracle() {
        for (text, expected) in [("impl.a::alpha", -7256336285983540041_i64),
            ("impl.a::zebra", -8125403953184358735), ("é", 6047309291227476195),
            ("α", 5682473352319658451), ("😀", -3536540696076613844), ("", 0),
            ("a\0b", -2562947060344435835)] {
            assert_eq!(hash(text) as i64, expected, "{text:?}");
        }
    }

    #[test]
    fn definition_order_does_not_replace_hash_set_order() {
        for values in [["impl.a::alpha", "impl.a::zebra"], ["impl.a::zebra", "impl.a::alpha"]] {
            let actual: Vec<_> = order(&values).into_iter().map(|i| values[i]).collect();
            assert_eq!(actual, ["impl.a::zebra", "impl.a::alpha"]);
        }
    }

    #[test]
    fn duplicates_collisions_and_resizes_match_the_python_oracle() {
        // Recorded offline with CPython 3.13.18, PYTHONHASHSEED=0; no runtime Python.
        for (n, expected) in [
            (0, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            (1, "222c77c8435912f2bc789918fdf441f1d2a6b21a020f811d87b8884fb6c6cde5"),
            (2, "8648bfded9ce9dc71f207d6c175552db6de428be18825ae15cea13afed0af5cb"),
            (4, "6a906ff491bd635dd2b7f6606f31b57e8621e2180e44caf44accb8291ee13f9a"),
            (5, "6fcae9444354fc06fe8ccaf02a93a00c00d3a65a471028988dcf4dec8c9e6b6d"),
            (18, "9ad26cb964724d90dfaf241f3c6fe40dcc07e2de1c0771fefccee9784fde2454"),
            (19, "f9406e1b4acc36a232e0681f71782a7d255472af38da6e09f64a30fa867ee1c8"),
            (76, "60feb30385f9bd12bc0f2e73ce5565d6b22070180f4537967e250c8a461a41d5"),
            (77, "8319bdb672e1b884593c1f845fa789741432a9cb287fd2bdd9ac33efd2b65ab1"),
            (307, "81f0c3e77ffe1fa4d831da2464e95750cf20af5204e228d1f70cd12823e9b40a"),
            (308, "0db61cf76e32836ab41509b52bd882a6e50b3d2dd078d2538c5ec5f11209dbdd"),
            (1200, "f1d653eaaf8e5365a5d985579ae8a388d98a22d4908a530649325716662fc456"),
            (50001, "b119e12685a7eb8bdf487645e6138055ed78a9baf680970f367c4d312abaa439"),
        ] {
            let mut values: Vec<_> = (0..n).map(|i| format!("impl.module::{i:05}{}", ["", "é", "α", "😀"][i % 4])).collect();
            if n > 2 { values.extend(values.clone().into_iter().step_by(3)); }
            let refs: Vec<_> = values.iter().map(String::as_str).collect();
            let ordered: Vec<_> = order(&refs).into_iter().map(|i| refs[i]).collect();
            assert_eq!(ordered.len(), n);
            assert_eq!(format!("{:x}", Sha256::digest(ordered.join("\n").as_bytes())), expected, "n={n}");
        }
    }
}
