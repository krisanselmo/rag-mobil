use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;

/// Asks `first`, then `second` only for the sets `first` doesn't know.
///
/// Any other error of `first` (network, invalid inventory...) is returned
/// as is: falling back would hide a broken source behind another one.
pub struct Fallback<A, B> {
    first: A,
    second: B,
}

/// Which source of a [`Fallback`] answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    First,
    Second,
}

impl<A, B> Fallback<A, B> {
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }

    pub fn first(&self) -> &A {
        &self.first
    }

    pub fn second(&self) -> &B {
        &self.second
    }
}

impl<A: InventorySource + Sync, B: InventorySource + Sync> Fallback<A, B> {
    /// Like [`InventorySource::fetch`], also telling which source answered.
    pub async fn fetch_traced(&self, set: &SetNumber) -> Result<(SetInventory, Answered)> {
        match self.first.fetch(set).await {
            Ok(inventory) => Ok((inventory, Answered::First)),
            Err(RagError::SetNotFound(first)) => match self.second.fetch(set).await {
                Ok(inventory) => Ok((inventory, Answered::Second)),
                Err(RagError::SetNotFound(second)) => {
                    Err(RagError::SetNotFound(format!("{first} ; {second}")))
                }
                Err(err) => Err(err),
            },
            Err(err) => Err(err),
        }
    }
}

impl<A: InventorySource + Sync, B: InventorySource + Sync> InventorySource for Fallback<A, B> {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        self.fetch_traced(set).await.map(|(inventory, _)| inventory)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Knows only `known`; fails every other set with `error`.
    struct Fake {
        known: &'static str,
        error: fn(&SetNumber) -> RagError,
        calls: AtomicUsize,
    }

    impl Fake {
        fn new(known: &'static str, error: fn(&SetNumber) -> RagError) -> Self {
            Self {
                known,
                error,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl InventorySource for Fake {
        async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if set.as_str() != self.known {
                return Err((self.error)(set));
            }
            Ok(SetInventory {
                set_number: set.clone(),
                name: format!("from {}", self.known),
                theme: None,
                focus: None,
                release_date: None,
                parts: vec![],
                source_url: "test".into(),
            })
        }
    }

    fn not_found(set: &SetNumber) -> RagError {
        RagError::SetNotFound(format!("{set} (absent de fake)"))
    }

    fn unavailable(set: &SetNumber) -> RagError {
        RagError::SourceStatus {
            url: format!("http://fake/{set}"),
            status: 503,
        }
    }

    fn set(n: &str) -> SetNumber {
        SetNumber::parse(n).unwrap()
    }

    #[tokio::test]
    async fn first_source_wins_without_asking_the_second() {
        let chain = Fallback::new(Fake::new("1", not_found), Fake::new("1", not_found));
        let (inventory, answered) = chain.fetch_traced(&set("1")).await.unwrap();
        assert_eq!(answered, Answered::First);
        assert_eq!(inventory.name, "from 1");
        assert_eq!(chain.second().calls(), 0);
    }

    #[tokio::test]
    async fn unknown_set_is_asked_to_the_second_source() {
        let chain = Fallback::new(Fake::new("1", not_found), Fake::new("2", not_found));
        let (inventory, answered) = chain.fetch_traced(&set("2")).await.unwrap();
        assert_eq!(answered, Answered::Second);
        assert_eq!(inventory.name, "from 2");
        assert_eq!(chain.fetch(&set("2")).await.unwrap().name, "from 2");
    }

    #[tokio::test]
    async fn other_errors_of_the_first_source_do_not_fall_back() {
        let chain = Fallback::new(Fake::new("1", unavailable), Fake::new("2", not_found));
        let err = chain.fetch(&set("2")).await.unwrap_err();
        assert!(matches!(err, RagError::SourceStatus { status: 503, .. }));
        assert_eq!(chain.second().calls(), 0);
    }

    #[tokio::test]
    async fn errors_of_the_second_source_are_returned() {
        let chain = Fallback::new(Fake::new("1", not_found), Fake::new("1", unavailable));
        let err = chain.fetch(&set("2")).await.unwrap_err();
        assert!(matches!(err, RagError::SourceStatus { .. }));

        let chain = Fallback::new(Fake::new("1", not_found), Fake::new("1", not_found));
        let err = chain.fetch(&set("2")).await.unwrap_err();
        assert!(matches!(err, RagError::SetNotFound(_)));
        assert_eq!(
            err.to_string(),
            "set introuvable : 2 (absent de fake) ; 2 (absent de fake)"
        );
    }
}
