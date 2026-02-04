use dashmap::DashMap;

/// Wrapper struct that prevents storing more keys than capacity
///
/// As long as .`len()` < .`capacity()` it should work
pub struct LimitedSlotsMap<K: Eq + std::hash::Hash, V>(DashMap<K, V>);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Operation would require resize")]
    WouldIncrease,
}

type Result<T> = std::result::Result<T, Error>;

impl<K, V> LimitedSlotsMap<K, V>
where
    K: Eq + std::hash::Hash,
{
    pub fn with_capacity(size: usize) -> Self {
        LimitedSlotsMap(DashMap::with_capacity(size))
    }

    fn check_capacity(&self) -> Result<()> {
        if self.0.len() < self.0.capacity() {
            Ok(())
        } else {
            Err(Error::WouldIncrease)
        }
    }

    pub fn insert(&self, key: K, value: V) -> Result<Option<V>> {
        self.check_capacity()?;
        Ok(self.0.insert(key, value))
    }

    pub fn get(&self, key: &K) -> Option<dashmap::mapref::one::Ref<'_, K, V>> {
        self.0.get(key)
    }

    pub fn remove(&self, key: &K) -> Option<(K, V)> {
        self.0.remove(key)
    }
}
