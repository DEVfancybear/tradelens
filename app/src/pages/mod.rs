//! One module per screen; [`page`] picks the one of a route.

use nsh_core::prelude::*;

use crate::nav::Route;

mod accounts;
mod analytics;
mod calendar;
mod cloud;
mod dashboard;
mod data_sources;
mod journal;
mod settings;
mod trades;

pub(crate) fn page(route: Route) -> AnyElement {
    match route {
        Route::Dashboard => dashboard::dashboard().into_element(),
        Route::Trades => trades::trades().into_element(),
        Route::Calendar => calendar::calendar().into_element(),
        Route::Analytics => analytics::analytics().into_element(),
        Route::Journal => journal::journal().into_element(),
        Route::Accounts => accounts::accounts().into_element(),
        Route::DataSources => data_sources::data_sources().into_element(),
        Route::Cloud => cloud::cloud().into_element(),
        Route::Settings => settings::settings().into_element(),
    }
}
