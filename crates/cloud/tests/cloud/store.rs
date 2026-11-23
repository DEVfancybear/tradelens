//! The OS credential store. It writes real entries (under a test service name and removed at
//! the end), so it runs only on request: `cargo test -p tradelens-cloud -- --ignored`.

use tradelens_cloud::auth::{Identity, Provider, Tokens};
use tradelens_cloud::token_store::{KeyringStore, TokenStore};

#[test]
#[ignore = "writes to the OS credential store"]
fn the_os_store_keeps_the_refresh_token_and_the_identity() {
    let store = KeyringStore::with_service("vn.dat.tradelens.test");
    let tokens = Tokens { access_token: "a".repeat(3000), refresh_token: Some("rt-keep".into()), expires_at: 99 };
    let me = Identity { provider: Provider::Google, email: "a@b.c".into(), name: "A".into() };

    store.save(Provider::Google, &tokens).expect("saves, however long the access token is");
    store.save_identity(&me).expect("saves");
    let loaded = store.load(Provider::Google).expect("loads").expect("is there");
    assert_eq!(loaded.refresh_token.as_deref(), Some("rt-keep"));
    assert!(loaded.access_token.is_empty() && loaded.is_expired(0), "the access token is not persisted");
    assert_eq!(store.load_identity(Provider::Google).expect("loads"), Some(me));

    store.save_secret("notion-token", "ntn_x").expect("saves");
    assert_eq!(store.load_secret("notion-token").expect("loads").as_deref(), Some("ntn_x"));

    store.clear(Provider::Google).expect("clears");
    store.clear_secret("notion-token").expect("clears");
    store.clear(Provider::Google).expect("clearing twice is fine");
    assert_eq!(store.load(Provider::Google).expect("loads"), None);
    assert_eq!(store.load_identity(Provider::Google).expect("loads"), None);
    assert_eq!(store.load_secret("notion-token").expect("loads"), None);
}
