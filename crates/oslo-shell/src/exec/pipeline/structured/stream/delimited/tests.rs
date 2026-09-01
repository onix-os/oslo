use super::*;

/// **A quoted newline is not a record boundary**, which is the whole reason this is not `rfind`.
/// Cutting there would turn one record into two, silently, and only for data that quotes a newline.
#[test]
fn a_batch_ends_at_a_record_rather_than_at_a_newline() {
    // The last newline closes a record: everything is whole.
    assert_eq!(whole_records("a,b\n1,2\n", ','), Some(8));

    // The last newline is *inside* a quoted field, so nothing after the previous record is whole
    // yet and the reader has to go back for more.
    assert_eq!(whole_records("a,b\n\"one\n", ','), None);

    // Once the field closes, the record is taken whole — newline and all.
    let closed = "a,b\n\"one\ntwo\",2\n";
    assert_eq!(whole_records(closed, ','), Some(closed.len()));
}

/// A batch with no newline at all has no whole record in it.
#[test]
fn a_partial_line_is_not_a_record() {
    assert_eq!(whole_records("a,b", ','), None);
    assert_eq!(whole_records("", ','), None);
}

/// **The header is remembered and put back**, because every batch after the first arrives without
/// it and would otherwise read its own first record as the column names.
#[test]
fn the_header_is_carried_into_every_later_batch() {
    let mut header = Header::default();

    // The first batch carries its own header, so it is handed over untouched.
    assert_eq!(
        header.before("name,age\nann,31\n", ','),
        "name,age\nann,31\n"
    );
    // Every later one gets it back in front.
    assert_eq!(header.before("bob,24\n", ','), "name,age\nbob,24\n");
    assert_eq!(header.before("cal,47\n", ','), "name,age\ncal,47\n");
}

/// A header may itself quote a newline, so it is found the same way a batch boundary is.
#[test]
fn a_header_that_quotes_a_newline_is_still_one_record() {
    let mut header = Header::default();
    header.before("\"first\nname\",age\nann,31\n", ',');
    assert_eq!(
        header.before("bob,24\n", ','),
        "\"first\nname\",age\nbob,24\n",
        "the whole header, including the newline inside it"
    );
}
