//! `?page[…]` and `?sort=` parsing, and the pagination links that follow.
use ash_jsonapi::query::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, Page, Paginator};
use ash_jsonapi::{Context, Query};

fn ctx() -> Context {
    Context::new("c")
}

fn parse(query: &str) -> Query {
    Query::parse(query, &ctx()).expect("parses")
}

#[test]
fn an_empty_query_is_the_first_page_by_offset() {
    let query = parse("");
    assert_eq!(
        query.page(),
        &Page::Offset {
            offset: 0,
            limit: DEFAULT_PAGE_SIZE
        }
    );
    assert!(query.sort().is_empty());
}

#[test]
fn offset_and_limit_are_read() {
    let query = parse("page[offset]=20&page[limit]=10");
    assert_eq!(
        query.page(),
        &Page::Offset {
            offset: 20,
            limit: 10
        }
    );
}

/// Most clients send the brackets percent-encoded.
#[test]
fn encoded_brackets_survive_the_round_trip() {
    let query = parse("page%5Boffset%5D=20&page%5Blimit%5D=10");
    assert_eq!(
        query.page(),
        &Page::Offset {
            offset: 20,
            limit: 10
        }
    );
}

#[test]
fn a_plus_decodes_to_a_space_in_a_sort_key() {
    let query = parse("sort=first+name");
    assert_eq!(query.sort()[0].field(), "first name");
}

#[test]
fn a_malformed_escape_is_kept_verbatim() {
    // `%zz` is not a valid escape; the `%` stays rather than failing.
    let query = parse("sort=a%zz");
    assert_eq!(query.sort()[0].field(), "a%zz");
}

/// The cap is what stops `page[limit]=1000000` from being a denial of service.
#[test]
fn a_limit_over_the_cap_is_clamped() {
    let query = parse("page[limit]=1000000");
    assert_eq!(query.page().size(), MAX_PAGE_SIZE);

    let query = parse("page[size]=1000000");
    assert_eq!(query.page().size(), MAX_PAGE_SIZE);
}

#[test]
fn cursor_paging_is_recognised() {
    let query = parse("page[cursor]=t_20&page[size]=10");
    assert_eq!(
        query.page(),
        &Page::Cursor {
            after: Some("t_20".to_string()),
            size: 10
        }
    );
}

#[test]
fn an_empty_cursor_is_the_first_page() {
    let query = parse("page[cursor]=&page[size]=10");
    assert_eq!(
        query.page(),
        &Page::Cursor {
            after: None,
            size: 10
        }
    );
}

#[test]
fn size_alone_starts_a_cursor_walk() {
    let query = parse("page[size]=10");
    assert_eq!(
        query.page(),
        &Page::Cursor {
            after: None,
            size: 10
        }
    );
}

#[test]
fn mixing_the_two_strategies_is_refused() {
    let err = Query::parse("page[offset]=1&page[cursor]=x", &ctx()).expect_err("a 400");
    assert_eq!(err.status().as_u16(), 400);

    let object = err.to_object();
    assert_eq!(object.source.unwrap().parameter.unwrap(), "page");
}

#[test]
fn a_non_numeric_page_parameter_is_refused_naming_itself() {
    let err = Query::parse("page[limit]=lots", &ctx()).expect_err("a 400");
    assert_eq!(err.status().as_u16(), 400);

    let object = err.to_object();
    assert_eq!(object.source.unwrap().parameter.unwrap(), "page[limit]");
    assert!(
        object.detail.unwrap().contains("lots"),
        "the message should quote what was sent"
    );
}

#[test]
fn a_negative_page_parameter_is_refused() {
    assert!(Query::parse("page[offset]=-1", &ctx()).is_err());
}

#[test]
fn unknown_parameters_are_ignored() {
    // A shared link carrying `?utm_source=` must not fail the request.
    let query = parse("utm_source=newsletter&page[limit]=5");
    assert_eq!(query.page().size(), 5);
}

#[test]
fn sort_keys_keep_their_order_and_direction() {
    let query = parse("sort=-created,message");
    let keys = query.sort();

    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].field(), "created");
    assert!(keys[0].descending());
    assert_eq!(keys[1].field(), "message");
    assert!(!keys[1].descending());
}

#[test]
fn empty_sort_keys_are_dropped() {
    // A trailing comma, and a bare `-`, are not errors.
    let query = parse("sort=message,,-");
    assert_eq!(query.sort().len(), 1);
    assert_eq!(query.sort()[0].field(), "message");
}

// --- Pagination links -------------------------------------------------

#[test]
fn the_first_page_links_forward_but_not_back() {
    let query = parse("page[offset]=0&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(10, None, None, None);

    assert!(links.this.is_some());
    assert!(links.first.is_some());
    assert!(
        links.prev.is_none(),
        "nowhere back to go from the first page"
    );
    assert!(links.next.is_some(), "a full page suggests there is more");
}

#[test]
fn a_later_page_links_back() {
    let query = parse("page[offset]=20&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(10, None, None, None);

    assert_eq!(links.prev.unwrap(), "/todos?page[offset]=10&page[limit]=10");
}

#[test]
fn prev_does_not_underflow_past_zero() {
    let query = parse("page[offset]=5&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(5, None, None, None);

    assert_eq!(links.prev.unwrap(), "/todos?page[offset]=0&page[limit]=10");
}

#[test]
fn a_known_total_gives_a_last_link() {
    let query = parse("page[offset]=0&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(10, Some(95), None, None);

    // The largest multiple of the limit below the total.
    assert_eq!(links.last.unwrap(), "/todos?page[offset]=90&page[limit]=10");
}

#[test]
fn an_exact_multiple_lands_on_the_final_full_page() {
    let query = parse("page[offset]=0&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(10, Some(100), None, None);

    assert_eq!(links.last.unwrap(), "/todos?page[offset]=90&page[limit]=10");
}

#[test]
fn an_empty_collections_last_page_is_its_first() {
    let query = parse("page[offset]=0&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(0, Some(0), None, None);

    assert_eq!(links.last.unwrap(), "/todos?page[offset]=0&page[limit]=10");
    assert!(links.next.is_none(), "nothing to page into");
}

#[test]
fn storage_that_knows_is_believed_over_the_guess() {
    let query = parse("page[offset]=0&page[limit]=10");

    // A full page would otherwise be taken as evidence of more.
    let links = Paginator::new("/todos", &query).links(10, None, None, Some(false));
    assert!(links.next.is_none(), "storage said this is the end");

    // And the reverse: a short page that storage says continues.
    let links = Paginator::new("/todos", &query).links(3, None, None, Some(true));
    assert!(links.next.is_some());
}

#[test]
fn a_total_ends_the_walk_without_an_empty_page() {
    let query = parse("page[offset]=90&page[limit]=10");
    let links = Paginator::new("/todos", &query).links(10, Some(100), None, None);

    assert!(links.next.is_none(), "offset + returned == total");
}

#[test]
fn sort_keys_are_carried_across_every_link() {
    let query = parse("page[offset]=10&page[limit]=10&sort=-created");
    let links = Paginator::new("/todos", &query).links(10, Some(100), None, None);

    for link in [links.this, links.first, links.prev, links.next, links.last] {
        let link = link.expect("every link is present here");
        assert!(
            link.contains("sort=-created"),
            "following a link must not reorder the collection: {link}"
        );
    }
}

#[test]
fn a_cursor_page_links_only_forward() {
    let query = parse("page[cursor]=t_20&page[size]=10");
    let links = Paginator::new("/todos", &query).links(10, None, Some("t_30"), None);

    assert!(links.this.is_some());
    assert!(links.next.unwrap().contains("page[cursor]=t_30"));
    assert!(links.first.is_none(), "a cursor cannot jump to the start");
    assert!(links.prev.is_none());
    assert!(links.last.is_none(), "and cannot know the end");
}

#[test]
fn no_cursor_back_means_no_next() {
    let query = parse("page[cursor]=t_20&page[size]=10");
    let links = Paginator::new("/todos", &query).links(4, None, None, None);

    assert!(links.next.is_none(), "storage reported no further page");
}

/// A cursor is storage's opaque string and may hold anything.
#[test]
fn a_cursor_is_percent_encoded_into_the_link() {
    let query = parse("page[size]=10");
    let links = Paginator::new("/todos", &query).links(10, None, Some("a b&c=d"), None);

    let next = links.next.unwrap();
    assert!(next.contains("a%20b%26c%3Dd"), "{next}");
}

#[test]
fn the_first_cursor_page_names_no_cursor() {
    let query = parse("page[size]=10");
    let links = Paginator::new("/todos", &query).links(10, None, None, None);

    let this = links.this.unwrap();
    assert!(!this.contains("page[cursor]"), "{this}");
    assert!(this.contains("page[size]=10"), "{this}");
}
