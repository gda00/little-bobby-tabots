pub mod guild_state;
pub mod ping;
pub mod play;
pub mod skip;
pub mod leave;
pub mod queue;
pub mod pause;
pub mod resume;

use std::sync::Arc;

use serenity::prelude::TypeMapKey;

use crate::Data;

/// TypeMap key for the shared bot data.
pub struct DataKey;

impl TypeMapKey for DataKey {
    type Value = Arc<Data>;
}
