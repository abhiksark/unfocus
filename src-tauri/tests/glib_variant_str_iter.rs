#![cfg(target_os = "linux")]

use glib::variant::ToVariant;

fn strings() -> glib::Variant {
    ["", "alpha", "বাংলা", "🌿"].to_variant()
}

#[test]
fn next_preserves_empty_and_unicode_strings() {
    let value = strings();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some(""));
    assert_eq!(iter.next(), Some("alpha"));
    assert_eq!(iter.next(), Some("বাংলা"));
    assert_eq!(iter.next(), Some("🌿"));
    assert_eq!(iter.next(), None);
}

#[test]
fn last_reads_the_final_string() {
    assert_eq!(strings().array_iter_str().unwrap().last(), Some("🌿"));
}

#[test]
fn nth_reads_the_selected_string_and_handles_overflow() {
    let value = strings();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.nth(2), Some("বাংলা"));
    assert_eq!(iter.nth(usize::MAX), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn next_back_reads_strings_in_reverse() {
    let value = strings();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.next_back(), Some("🌿"));
    assert_eq!(iter.next_back(), Some("বাংলা"));
    assert_eq!(iter.next_back(), Some("alpha"));
    assert_eq!(iter.next_back(), Some(""));
    assert_eq!(iter.next_back(), None);
}

#[test]
fn nth_back_reads_the_selected_string_and_handles_overflow() {
    let value = strings();
    let mut iter = value.array_iter_str().unwrap();
    assert_eq!(iter.nth_back(1), Some("বাংলা"));
    assert_eq!(iter.nth_back(usize::MAX), None);
    assert_eq!(iter.next_back(), None);
}

#[test]
fn mixed_iteration_exhausts_once_and_keeps_borrowed_strings_valid() {
    let value = strings();
    let mut iter = value.array_iter_str().unwrap();
    let first = iter.next().unwrap();
    assert_eq!(iter.next_back(), Some("🌿"));
    assert_eq!(iter.nth(1), Some("বাংলা"));
    assert_eq!(iter.len(), 0);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
    assert_eq!(first, "");
}

#[test]
fn empty_arrays_are_empty_for_all_affected_operations() {
    let value = Vec::<String>::new().to_variant();
    assert_eq!(value.array_iter_str().unwrap().next(), None);
    assert_eq!(value.array_iter_str().unwrap().last(), None);
    assert_eq!(value.array_iter_str().unwrap().nth(1), None);
    assert_eq!(value.array_iter_str().unwrap().next_back(), None);
    assert_eq!(value.array_iter_str().unwrap().nth_back(0), None);
}
