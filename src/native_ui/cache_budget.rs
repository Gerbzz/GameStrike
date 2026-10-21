#[derive(Clone,Copy)]
pub struct Resident { pub bytes: usize, pub last_used: u64 }

pub fn eviction_indices(entries: &[Resident],max_entries: usize,max_bytes: usize) -> Vec<usize> {
    let mut ordered: Vec<_>=(0..entries.len()).collect();
    ordered.sort_by_key(|&index|(entries[index].last_used,index));
    let mut count=entries.len();
    let mut bytes: u128=entries.iter().map(|entry|entry.bytes as u128).sum();
    let mut evicted=Vec::new();
    for index in ordered {
        if count<=max_entries && bytes<=max_bytes as u128 { break; }
        count-=1;bytes-=entries[index].bytes as u128;evicted.push(index);
    }
    evicted
}
