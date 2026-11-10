use std::sync::{Arc, Weak};

/// Which shared value a key refers to, without keeping the value alive; the held `Weak` keeps its address from being reused, so a new value never passes for it
#[derive(Debug)]
pub(super) struct SharedHandle<T>(Option<Weak<T>>);

impl<T> SharedHandle<T> {
    pub(super) fn of(value: Option<&Arc<T>>) -> SharedHandle<T> {
        SharedHandle(value.map(Arc::downgrade))
    }
}

impl<T> Clone for SharedHandle<T> {
    fn clone(&self) -> Self {
        SharedHandle(self.0.clone())
    }
}

impl<T> Default for SharedHandle<T> {
    fn default() -> Self {
        SharedHandle(None)
    }
}

impl<T> PartialEq for SharedHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(this), Some(other)) => Weak::ptr_eq(this, other),
            (this, other) => this.is_none() && other.is_none(),
        }
    }
}
