//! What a layer above sets up on each owned connection and keeps with it (the transducer's demand module and its
//! match registry): the interface the connection's owner (`waves::owner::database`) holds it by, registered there
//! at load (`install_connection_setup`).

use crate::content_store::StoreResult;

/// A connection's setup.
pub trait ConnectionSetup {
    /// The setup as its own type, which only the layer that made it reads.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// What sets up an owned connection.
pub type SetUp = fn(&rusqlite::Connection) -> StoreResult<Box<dyn ConnectionSetup>>;
