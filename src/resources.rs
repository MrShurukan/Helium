//! Static hardware ownership, distinct from runtime shared-access analysis.
use std::collections::{btree_map::Entry, BTreeMap};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum HardwareResource {
    Pin { port: String, bit: usize },
    Peripheral(String),
    InterruptVector(String),
}

pub struct Reservations<T> {
    owners: BTreeMap<HardwareResource, T>,
}

impl<T: Copy> Default for Reservations<T> {
    fn default() -> Self {
        Self {
            owners: BTreeMap::new(),
        }
    }
}

impl<T: Copy> Reservations<T> {
    /// Preserve the first owner, so repeated conflicts point to its declaration.
    pub fn claim(&mut self, resource: HardwareResource, owner: T) -> Result<(), T> {
        match self.owners.entry(resource) {
            Entry::Vacant(entry) => {
                entry.insert(owner);
                Ok(())
            }
            Entry::Occupied(entry) => Err(*entry.get()),
        }
    }
}
