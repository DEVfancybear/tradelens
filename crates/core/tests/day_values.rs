//! The values of the trader's own day fields, kept per account and day.

use chrono::NaiveDate;
use tradelens_core::{
    DayNote, Db,
    catalog::{CustomValues, FieldValue},
};

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("fixture days parse")
}

#[test]
fn day_values_save_reload_and_stay_apart() {
    let mut db = Db::in_memory().expect("a memory database opens");
    let values = CustomValues::from([
        ("focus".to_owned(), FieldValue::Scale(7)),
        ("news".to_owned(), FieldValue::Text("NFP 19:30".into())),
    ]);
    assert!(db.day_values(0, day("2026-11-20")).expect("reads").is_empty());
    db.save_day_values(0, day("2026-11-20"), &values).expect("saves");
    assert_eq!(db.day_values(0, day("2026-11-20")).expect("reads"), values);
    assert!(db.day_values(1, day("2026-11-20")).expect("reads").is_empty(), "another account has its own");
    assert!(db.day_values(0, day("2026-11-21")).expect("reads").is_empty(), "another day has its own");
}

#[test]
fn saving_no_values_removes_them() {
    let mut db = Db::in_memory().expect("a memory database opens");
    let values = CustomValues::from([("focus".to_owned(), FieldValue::Scale(3))]);
    db.save_day_values(0, day("2026-11-20"), &values).expect("saves");
    db.save_day_values(0, day("2026-11-20"), &CustomValues::new()).expect("clears");
    assert!(db.day_values(0, day("2026-11-20")).expect("reads").is_empty());
}

#[test]
fn saving_values_keeps_the_note_and_saving_the_note_keeps_the_values() {
    let mut db = Db::in_memory().expect("a memory database opens");
    let note = DayNote { plan: "Chờ tin 19:30".into(), review: String::new(), mood: "calm".into() };
    let values = CustomValues::from([("news".to_owned(), FieldValue::Text("NFP bất ngờ".into()))]);
    db.save_day_note(1, day("2026-11-20"), &note).expect("saves the note");
    db.save_day_values(1, day("2026-11-20"), &values).expect("saves the values");
    assert_eq!(db.day_note(1, day("2026-11-20")).expect("reads"), note, "the note is untouched");
    let changed = DayNote { plan: "Đổi kế hoạch".into(), ..note };
    db.save_day_note(1, day("2026-11-20"), &changed).expect("saves the note again");
    assert_eq!(db.day_values(1, day("2026-11-20")).expect("reads"), values, "the values are untouched");
}

#[test]
fn values_on_a_day_without_a_note_create_it_and_are_searchable() {
    let mut db = Db::in_memory().expect("a memory database opens");
    let values = CustomValues::from([("news".to_owned(), FieldValue::Text("Powell phát biểu".into()))]);
    db.save_day_values(0, day("2026-11-21"), &values).expect("saves the values");
    assert_eq!(db.day_note(0, day("2026-11-21")).expect("reads"), DayNote::default());
    let hits = db.search("powell", 10).expect("searches");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, "2026-11-21", "the same reference as the day's note");
    // The note's own save keeps the values in the index.
    db.save_day_note(0, day("2026-11-21"), &DayNote { plan: "Kế hoạch".into(), ..DayNote::default() })
        .expect("saves the note");
    assert_eq!(db.search("powell", 10).expect("searches").len(), 1);
}
