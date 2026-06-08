//! Spec 3 "definition of done": create collections, add requests/folders, edit
//! fields, reorder, move across collections, save — and re-load without error,
//! with every intended edit present and round-trip stability.
//!
//! All operations are performed against a TEMPDIR so the repo's committed files
//! are never mutated.

use golden_core::model::Header;
use golden_core::store;
use tempfile::tempdir;

#[test]
fn full_editing_lifecycle_round_trips() {
    let dir = tempdir().unwrap();

    // 1) create two collections
    let src = store::create_collection(dir.path(), "Source API").unwrap();
    let dst = store::create_collection(dir.path(), "Dest API").unwrap();
    assert_eq!(
        src.file_name().unwrap().to_str().unwrap(),
        "source-api.json"
    );

    // 2) add a folder + request to source
    let mut s = store::load_collection(&src).unwrap();
    store::add_folder(&mut s.item, &[], "auth").unwrap();
    store::add_request(&mut s.item, &[0], "login", "GET").unwrap();
    store::add_request(&mut s.item, &[], "ping", "GET").unwrap();

    // 3) edit the nested request: method, url, headers, body, scripts
    // s.item[0] = auth folder, s.item[0].item[0] = login request → path [0, 0]
    store::set_method(&mut s.item, &[0, 0], "POST").unwrap();
    store::set_url(&mut s.item, &[0, 0], "https://api.test/login").unwrap();
    store::set_headers(
        &mut s.item,
        &[0, 0],
        vec![Header {
            key: "Content-Type".into(),
            value: "application/json".into(),
            disabled: false,
            extra: Default::default(),
        }],
    )
    .unwrap();
    store::set_raw_body(&mut s.item, &[0, 0], "{\"u\":\"x\"}").unwrap();
    store::set_script(
        &mut s.item,
        &[0, 0],
        "test",
        &["pm.test('ok', function () { pm.expect(pm.response.code).to.equal(200); });".into()],
    )
    .unwrap();

    // 4) collection variable CRUD
    store::set_variable(&mut s, "base", "https://api.test");
    store::set_variable(&mut s, "base", "https://api.test/v2"); // edit (overwrite)
    store::set_variable(&mut s, "drop", "x");
    assert!(store::delete_variable(&mut s, "drop"));

    // 5) reorder root of source: move "ping" (index 1) before "auth" (index 0)
    store::move_item_in_container(&mut s.item, &[], 1, 0).unwrap();
    assert_eq!(s.item[0].name, "ping");

    store::save_collection(&src, &s).unwrap();

    // 6) duplicate the login request inside its folder
    let mut s = store::load_collection(&src).unwrap();
    // After reorder: s.item[0]=ping, s.item[1]=auth; auth.item[0]=login → path [1, 0]
    store::duplicate_item_by_name(&mut s.item, "login").unwrap();
    store::save_collection(&src, &s).unwrap();

    // 7) cross-collection move "ping" → Dest API root, atomic
    store::move_item_across_collections(&src, "ping", &dst, None).unwrap();

    // ----- assertions -----
    let s = store::load_collection(&src).unwrap();
    let d = store::load_collection(&dst).unwrap();

    // "ping" moved out of source and into dest
    assert!(
        s.item.iter().all(|i| i.name != "ping"),
        "ping must have moved out of source"
    );
    assert_eq!(
        d.item.iter().filter(|i| i.name == "ping").count(),
        1,
        "ping must appear exactly once in dest"
    );

    // auth folder still in source, with login + login (Copy)
    let auth = s.item.iter().find(|i| i.name == "auth").unwrap();
    let children = auth.item.as_ref().expect("auth must have children");
    assert!(
        children.iter().any(|i| i.name == "login"),
        "login must still be in auth"
    );
    assert!(
        children.iter().any(|i| i.name == "login (Copy)"),
        "login (Copy) must be in auth after duplicate"
    );

    // login request fields
    let login = children.iter().find(|i| i.name == "login").unwrap();
    let req = login.request.as_ref().expect("login must have a request");
    assert_eq!(req.method, "POST");
    assert_eq!(req.url.raw(), "https://api.test/login");
    assert_eq!(req.header[0].key, "Content-Type");

    // body raw
    let body_raw = req
        .body
        .as_ref()
        .expect("body must be set")
        .raw
        .as_ref()
        .and_then(|v| v.as_str())
        .expect("body.raw must be a JSON string");
    assert_eq!(body_raw, "{\"u\":\"x\"}");

    // test script
    let test_event = login
        .event
        .iter()
        .find(|e| e.listen == "test")
        .expect("test event must be present");
    assert_eq!(test_event.script.exec.len(), 1);

    // collection variables
    let base_var = s
        .variable
        .iter()
        .find(|v| v.key == "base")
        .expect("base variable must be present");
    assert_eq!(base_var.value, "https://api.test/v2");
    assert!(
        s.variable.iter().all(|v| v.key != "drop"),
        "drop variable must have been deleted"
    );

    // 8) round-trip stability: load → save → bytes unchanged
    let before = std::fs::read_to_string(&src).unwrap();
    let again = store::load_collection(&src).unwrap();
    store::save_collection(&src, &again).unwrap();
    let after = std::fs::read_to_string(&src).unwrap();
    assert_eq!(
        before, after,
        "save must be idempotent (no spurious mutations)"
    );
}
