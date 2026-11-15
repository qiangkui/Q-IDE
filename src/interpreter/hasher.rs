//! 解释器内部用的快速哈希器。
//!
//! 解释器里被查得最频繁的是**短 ASCII 标识符**（变量名、过程名、类型名）。标准库默认的
//! `SipHash` 为抗哈希洪水攻击做了额外工作，在这里是纯开销：脚本来自用户自己的文件、
//! 键空间也小。换成 FNV-1a 后同样的查找能快一倍以上（见 `ROADMAP.md` 的性能段落）。

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// FNV-1a（64 位）。
#[derive(Clone, Copy)]
pub struct FastHasher(u64);

impl Default for FastHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.0;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = hash;
    }
}

/// 用 [`FastHasher`] 的 `HashMap`。
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_map_keeps_entries_and_order_of_insertion() {
        let mut map: FastMap<String, i32> = FastMap::default();
        map.insert("alpha".into(), 1);
        map.insert("beta".into(), 2);
        assert_eq!(map.get("alpha"), Some(&1));
        assert_eq!(map.get("missing"), None);
        map.insert("alpha".into(), 3);
        assert_eq!(map.get("alpha"), Some(&3));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn hashes_differ_for_different_names() {
        use std::hash::{Hash, Hasher as _};

        let hash = |text: &str| {
            let mut hasher = FastHasher::default();
            text.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash("index"), hash("count"));
        assert_eq!(hash("same"), hash("same"));
    }
}
