//! Which Convex reads a session may make. The hub decides here, from the tab list its machine
//! reported, and nowhere else.

use super::{Connection, Offer};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Everything,
    /// Listing these tables and reading their documents, nothing else.
    Tables(BTreeSet<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub connection: Connection,
    pub scope: Scope,
}

impl Grant {
    pub fn offer(&self) -> Offer {
        let tables = match &self.scope {
            Scope::Everything => None,
            Scope::Tables(tables) => Some(tables.iter().cloned().collect()),
        };
        Offer { deployment: self.connection.id.clone(), kind: self.connection.kind, tables }
    }
}
