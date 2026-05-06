//! Per-provider Keychain access. Phase 1 hard-coded a single OpenAI slot;
//! Phase 2 generalises to one slot per cloud provider, keyed by the
//! `provider_id` that adapters return. Cache lives on AppState as a
//! HashMap so each provider's first read triggers exactly one Keychain
//! prompt, and subsequent reads are free.

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;

pub const KEYCHAIN_SERVICE: &str = "no.humla.app";

pub type ApiKeyCache = Arc<Mutex<HashMap<&'static str, Option<String>>>>;

pub fn new_cache() -> ApiKeyCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Map a provider id to its Keychain account name. Static (no allocation)
/// because adapter ids are `&'static str`. Returns None for providers
/// that don't take an API key (e.g. local Whisper).
pub fn keychain_account_for(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "openai" => Some("openai_api_key"),
        "deepgram" => Some("deepgram_api_key"),
        "groq" => Some("groq_api_key"),
        _ => None,
    }
}

/// True if the provider needs an API key. Phase 2: matches the same set
/// as `keychain_account_for`. Used by transcribe_chunk to decide whether
/// to look one up.
pub fn requires_api_key(provider_id: &str) -> bool {
    keychain_account_for(provider_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_have_keychain_accounts() {
        assert_eq!(keychain_account_for("openai"), Some("openai_api_key"));
        assert_eq!(keychain_account_for("deepgram"), Some("deepgram_api_key"));
        assert_eq!(keychain_account_for("groq"), Some("groq_api_key"));
        assert_eq!(keychain_account_for("local"), None);
        assert_eq!(keychain_account_for("nonsense"), None);
    }
}
