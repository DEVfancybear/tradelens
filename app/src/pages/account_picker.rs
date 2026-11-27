//! The account switch in the header of the trades page and the overview.

use nsh_core::prelude::*;

use crate::{
    store::{AccountFilter, store},
    ui::controls::{Choice, select_field},
};

const ALL: &str = "all";

/// A drop-down of the accounts, "All" first; it sets the store's account filter.
pub(crate) fn account_picker() -> impl IntoElement {
    let store = store();
    select_field(
        t!("account_picker.label"),
        move || {
            let snapshot = store.snapshot();
            let mut choices = vec![Choice::new(ALL, t!("account_picker.all", n = snapshot.accounts.len()))];
            choices.extend(snapshot.accounts.iter().map(|a| Choice::new(a.id.to_string(), a.title())));
            choices
        },
        move || match store.accounts.get() {
            AccountFilter::All => SharedString::from(ALL),
            AccountFilter::One(id) => SharedString::from(id.to_string()),
        },
        move |value| {
            let filter = value.parse().map_or(AccountFilter::All, AccountFilter::One);
            store.accounts.set(filter);
        },
    )
}
